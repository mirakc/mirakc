use std::fmt;
use std::io;
use std::pin::Pin;

use bytes::Bytes;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

#[cfg_attr(test, derive(Debug))]
pub struct MpegTsStream<T, S> {
    id: T,
    stream: S,
    decoded: bool,
}

impl<T, S> MpegTsStream<T, S> {
    pub fn new(id: T, stream: S) -> Self {
        MpegTsStream {
            id,
            stream,
            decoded: false,
        }
    }

    pub fn decoded(mut self) -> Self {
        self.decoded = true;
        self
    }

    pub fn is_decoded(&self) -> bool {
        self.decoded
    }
}

impl<T, S> MpegTsStream<T, S>
where
    T: Clone,
{
    pub fn id(&self) -> T {
        self.id.clone()
    }
}

impl<T, S> MpegTsStream<T, S>
where
    T: fmt::Display + Clone + Unpin,
    S: Stream<Item = io::Result<Bytes>> + Unpin,
{
    pub async fn pipe<W>(self, writer: W)
    where
        W: AsyncWrite + Unpin,
    {
        pipe(self, writer).await;
    }
}

impl<T, S> Stream for MpegTsStream<T, S>
where
    T: Unpin,
    S: Stream + Unpin,
{
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

// terminator
//
// A terminator is attached on the output-side endpoint of an MPEG-TS packets
// filtering pipeline in order to shutting down streaming quickly when an HTTP
// transaction ends.
//
// There is a delay from the HTTP transaction end to the tuner release when
// using filters.  On some environments, the delay is about 40ms.  On those
// environments, the next streaming request may be processed before the tuner is
// released.
//
// It's impossible to eliminate the delay completely, but it's possible to
// reduce the delay as much as possible.
//
// See a discussion in Japanese on:
// https://github.com/mirakc/mirakc/issues/4#issuecomment-583818912.

pub struct MpegTsStreamTerminator<S, T> {
    inner: S,
    _stop_trigger: T,
}

impl<S, T> MpegTsStreamTerminator<S, T> {
    pub fn new(inner: S, _stop_trigger: T) -> Self {
        Self {
            inner,
            _stop_trigger,
        }
    }
}

impl<S, T> Stream for MpegTsStreamTerminator<S, T>
where
    S: Stream + Unpin,
    T: Unpin,
{
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

async fn pipe<T, S, W>(mut stream: MpegTsStream<T, S>, mut writer: W)
where
    T: fmt::Display + Clone + Unpin,
    S: Stream<Item = io::Result<Bytes>> + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                tracing::trace!(stream.id = %stream.id(), chunk.size = chunk.len(), "Received a chunk");
                if let Err(err) = writer.write_all(&chunk).await {
                    if err.kind() == io::ErrorKind::BrokenPipe {
                        tracing::debug!(stream.id = %stream.id(), "Output has been closed");
                    } else {
                        tracing::error!(%err, stream.id = %stream.id(), "Failed to write to output");
                    }
                    break;
                }
            }
            Some(Err(err)) => {
                if err.kind() == io::ErrorKind::BrokenPipe {
                    tracing::debug!(stream.id = %stream.id(), "Input has been closed");
                } else {
                    tracing::error!(%err, stream.id = %stream.id(), "Failed to read from input");
                }
                break;
            }
            None => {
                tracing::debug!(stream.id = %stream.id(), "EOF");
                break;
            }
        }

        // TODO: Should yield here like web::streaming()?
    }

    if let Err(err) = writer.shutdown().await {
        tracing::warn!(%err, stream.id = %stream.id(), "Failed to shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;
    use tokio_stream::wrappers::ReceiverStream;

    #[test(tokio::test)]
    async fn test_pipe() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);

        let stream = MpegTsStream::new(0, ReceiverStream::new(rx));
        let writer = TestWriter::new(b"hello");
        let handle = tokio::spawn(stream.pipe(writer));

        let result = tx.send(Ok(Bytes::from("hello"))).await;
        assert!(result.is_ok());

        drop(tx);
        handle.await.unwrap();
    }

    struct TestWriter {
        buf: Vec<u8>,
        expected: &'static [u8],
    }

    impl TestWriter {
        fn new(expected: &'static [u8]) -> Self {
            Self {
                buf: Vec::new(),
                expected,
            }
        }
    }

    impl AsyncWrite for TestWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut std::task::Context,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            self.buf.extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut std::task::Context,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _: &mut std::task::Context,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl Drop for TestWriter {
        fn drop(&mut self) {
            assert_eq!(self.buf.as_slice(), self.expected);
        }
    }
}
