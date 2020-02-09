use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::stream::{Stream, StreamExt};
use tokio::sync::mpsc::Receiver;

use crate::tuner;
pub use crate::tuner::TunerSubscriptionId as MpegTsStreamId;

pub struct MpegTsStream {
    id: MpegTsStreamId,
    receiver: Receiver<Bytes>,
}

impl MpegTsStream {
    pub fn new(id: MpegTsStreamId, receiver: Receiver<Bytes>) -> Self {
        MpegTsStream { id, receiver }
    }

    pub async fn pipe<W>(self, writer: W)
    where
        W: AsyncWrite + Unpin,
    {
        pipe(self, writer).await
    }
}

impl WithMpegTsStreamId for MpegTsStream {
    fn id(&self) -> MpegTsStreamId {
        self.id
    }
}

impl Stream for MpegTsStream {
    type Item = io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver)
            .poll_next(cx)
            .map(|opt| opt.map(|chunk| Ok(chunk)))
    }
}

// terminator
//
// A terminator is attached on the output-side endpoint of an MPEG-TS packets
// filtering pipeline in order to shutting down streaming quickly when a HTTP
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
// https://github.com/masnagam/mirakc/issues/4#issuecomment-583818912.

pub struct MpegTsStreamTerminator<S> {
    id: MpegTsStreamId,
    inner: S,
}

impl<S> MpegTsStreamTerminator<S>
where
    S: Stream<Item = io::Result<Bytes>> + Unpin
{
    pub fn new(id: MpegTsStreamId, inner: S) -> Self {
        MpegTsStreamTerminator { id, inner }
    }

    pub async fn pipe<W>(self, writer: W)
    where
        W: AsyncWrite + Unpin,
    {
        pipe(self, writer).await
    }
}

impl<S> WithMpegTsStreamId for MpegTsStreamTerminator<S> {
    fn id(&self) -> MpegTsStreamId {
        self.id
    }
}

impl<S> Stream for MpegTsStreamTerminator<S>
where
    S: Stream<Item = io::Result<Bytes>> + Unpin
{
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for MpegTsStreamTerminator<S> {
    fn drop(&mut self) {
        log::debug!("{}: Closing...", self.id);
        tuner::stop_streaming(self.id);
    }
}

pub trait WithMpegTsStreamId {
    fn id(&self) -> MpegTsStreamId;
}

async fn pipe<S, W>(mut stream: S, mut writer: W)
where
    S: Stream<Item = io::Result<Bytes>> + WithMpegTsStreamId + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                if let Err(err) = writer.write_all(&chunk).await {
                    if err.kind() == io::ErrorKind::BrokenPipe {
                        log::debug!("Downstream has been closed");
                    } else {
                        log::error!("{}: Failed to write to downstream: {}",
                                    stream.id(), err);
                    }
                    return;
                }
            }
            Some(Err(err)) => {
                if err.kind() == io::ErrorKind::BrokenPipe {
                    log::debug!("Upstream has been closed");
                } else {
                    log::error!("{}: Failed to read from upstream: {}",
                                stream.id(), err);
                }
                return;
            }
            None => {
                log::debug!("{}: EOF reached", stream.id());
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_test::io::Builder;

    #[tokio::test]
    async fn test_pipe() {
        let hello = "hello";

        let (mut tx, rx) = tokio::sync::mpsc::channel(10);
        let stream = MpegTsStream::new(Default::default(), rx);
        let mock = Builder::new()
            .write(hello.as_bytes())
            .build();
        let handle = tokio::spawn(stream.pipe(mock));

        let result = tx.send(Bytes::from(hello.as_bytes())).await;
        assert!(result.is_ok());

        drop(tx);

        let _ = handle.await;
    }
}
