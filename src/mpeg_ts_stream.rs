use std::io;
use std::pin::Pin;

use actix::prelude::*;
use bytes::Bytes;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::stream::{Stream, StreamExt};

use crate::broadcaster::BroadcasterStream;
use crate::tuner::StopStreamingMessage;
pub use crate::tuner::TunerSubscriptionId as MpegTsStreamId;

pub struct MpegTsStream {
    id: MpegTsStreamId,
    stream: BroadcasterStream,
    stop_trigger: Option<MpegTsStreamStopTrigger>,
}

impl MpegTsStream {
    pub fn new(
        id: MpegTsStreamId,
        stream: BroadcasterStream,
        recipient: Recipient<StopStreamingMessage>
    ) -> Self {
        MpegTsStream {
            id, stream,
            stop_trigger: Some(MpegTsStreamStopTrigger::new(id, recipient))
        }
    }

    pub fn id(&self) -> MpegTsStreamId {
        self.id
    }

    pub fn take_stop_trigger(&mut self) -> Option<MpegTsStreamStopTrigger> {
        self.stop_trigger.take()
    }

    pub async fn pipe<W>(self, writer: W) -> (Self, W)
    where
        W: AsyncWrite + Unpin,
    {
        pipe(self, writer).await
    }
}

impl Stream for MpegTsStream {
    type Item = io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

pub struct MpegTsStreamStopTrigger {
    id: MpegTsStreamId,
    recipient: Recipient<StopStreamingMessage>,
}

impl MpegTsStreamStopTrigger {
    fn new(
        id: MpegTsStreamId,
        recipient: Recipient<StopStreamingMessage>
    ) -> Self {
        Self { id, recipient }
    }
}

impl Drop for MpegTsStreamStopTrigger {
    fn drop(&mut self) {
        log::debug!("{}: Closing...", self.id);
        let _ = self.recipient.do_send(StopStreamingMessage {
            id: self.id
        });
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

pub struct MpegTsStreamTerminator<S, T> {
    inner: S,
    _stop_trigger: T,
}

impl<S, T> MpegTsStreamTerminator<S, T>
where
    S: Stream<Item = io::Result<Bytes>> + Unpin
{
    pub fn new(inner: S, _stop_trigger: T) -> Self {
        Self { inner, _stop_trigger }
    }
}

impl<S, T> Stream for MpegTsStreamTerminator<S, T>
where
    S: Stream<Item = io::Result<Bytes>> + Unpin,
    T: Unpin,
{
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

async fn pipe<W>(mut stream: MpegTsStream, mut writer: W) -> (MpegTsStream, W)
where
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
                    break;
                }
            }
            Some(Err(err)) => {
                if err.kind() == io::ErrorKind::BrokenPipe {
                    log::debug!("Upstream has been closed");
                } else {
                    log::error!("{}: Failed to read from upstream: {}",
                                stream.id(), err);
                }
                break;
            }
            None => {
                log::debug!("{}: EOF reached", stream.id());
                break;
            }
        }
    }

    (stream, writer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[actix_rt::test]
    async fn test_pipe() {
        let addr = TestActor.start();

        let (mut tx, stream) = BroadcasterStream::new_for_test();

        let stream = MpegTsStream::new(
            Default::default(), stream, addr.recipient());
        let buf: Vec<u8> = Vec::new();
        let handle = tokio::spawn(stream.pipe(buf));

        let result = tx.send(Bytes::from("hello")).await;
        assert!(result.is_ok());

        drop(tx);
        let (_, buf) = handle.await.unwrap();
        assert_eq!(&buf, b"hello");
    }

    struct TestActor;

    impl Actor for TestActor {
        type Context = Context<Self>;
    }

    impl Handler<StopStreamingMessage> for TestActor {
        type Result = ();

        fn handle(
            &mut self,
            _: StopStreamingMessage,
            _: &mut Self::Context,
        ) -> Self::Result {}
    }
}
