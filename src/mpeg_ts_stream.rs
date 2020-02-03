use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::stream::{Stream, StreamExt};
use tokio::sync::mpsc::Receiver;

use crate::tuner;
use crate::tuner::TunerSubscriptionId as MpegTsStreamId;

pub struct MpegTsStream {
    id: MpegTsStreamId,
    receiver: Receiver<Bytes>,
}

impl MpegTsStream {
    pub fn new(id: MpegTsStreamId, receiver: Receiver<Bytes>) -> Self {
        MpegTsStream { id, receiver }
    }

    pub fn id(&self) -> MpegTsStreamId {
        self.id
    }

    pub async fn pipe<W>(mut self, mut writer: W)
    where
        W: AsyncWrite + Unpin,
    {
        loop {
            match self.next().await {
                Some(Ok(chunk)) => {
                    if let Err(err) = writer.write_all(&chunk).await {
                        if err.kind() == io::ErrorKind::BrokenPipe {
                            log::debug!("Downstream has been closed");
                        } else {
                            log::error!("{}: Failed to write to downstream: {}",
                                        self.id, err);
                        }
                        return;
                    }
                }
                Some(Err(err)) => {
                    if err.kind() == io::ErrorKind::BrokenPipe {
                        log::debug!("Upstream has been closed");
                    } else {
                        log::error!("{}: Failed to read from upstream: {}",
                                    self.id, err);
                    }
                    return;
                }
                None => {
                    log::debug!("{}: EOF reached", self.id);
                    return;
                }
            }
        }
    }

    fn close(&self) {
        log::debug!("{}: Closing...", self.id);
        tuner::stop_streaming(self.id);
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

impl Drop for MpegTsStream {
    fn drop(&mut self) {
        self.close();
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
