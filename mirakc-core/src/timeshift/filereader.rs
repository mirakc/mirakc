use std::future::Future;
use std::io;
use std::io::SeekFrom;
use std::pin::Pin;

use tokio::fs::File;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncSeekExt;
use tokio::io::ReadBuf;
use tokio::sync::oneshot;

use super::TimeshiftStreamStopTrigger;
use crate::error::Error;

pub struct TimeshiftFileReader {
    state: TimeshiftFileReaderState,
    path: String,
    file: File,
    stop_signal: Option<oneshot::Receiver<()>>,
}

enum TimeshiftFileReaderState {
    Read,
    Seek,
    Wait,
}

impl TimeshiftFileReader {
    pub async fn open(path: &str) -> Result<Self, Error> {
        let reader = TimeshiftFileReader {
            state: TimeshiftFileReaderState::Read,
            path: path.to_string(),
            file: File::open(path).await?,
            stop_signal: None,
        };
        Ok(reader)
    }

    pub fn with_stop_trigger(mut self) -> (Self, TimeshiftStreamStopTrigger) {
        let (tx, rx) = oneshot::channel();
        let stop_trigger = TimeshiftStreamStopTrigger::new(tx);
        self.stop_signal = Some(rx);
        (self, stop_trigger)
    }

    pub async fn set_position(&mut self, pos: u64) -> Result<(), Error> {
        let _ = self.file.seek(SeekFrom::Start(pos)).await;
        Ok(())
    }
}

impl AsyncRead for TimeshiftFileReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;

        loop {
            if let Some(ref mut stop_signal) = self.stop_signal {
                if Pin::new(stop_signal).poll(cx).is_ready() {
                    tracing::debug!(path = self.path, "Stopped reading");
                    return Poll::Ready(Ok(()));
                }
            }
            match self.state {
                TimeshiftFileReaderState::Read => {
                    let len = buf.filled().len();
                    match Pin::new(&mut self.file).poll_read(cx, buf) {
                        Poll::Ready(Ok(_)) if buf.filled().len() == len => {
                            self.state = TimeshiftFileReaderState::Seek;
                            tracing::debug!(path = self.path, "EOF");
                        }
                        poll => {
                            return poll;
                        }
                    }
                }
                TimeshiftFileReaderState::Seek => {
                    match Pin::new(&mut self.file).start_seek(SeekFrom::Start(0)) {
                        Ok(_) => {
                            self.state = TimeshiftFileReaderState::Wait;
                            tracing::debug!(path = self.path, "Seek to the beginning");
                        }
                        Err(err) => {
                            return Poll::Ready(Err(err));
                        }
                    }
                }
                TimeshiftFileReaderState::Wait => {
                    match Pin::new(&mut self.file).poll_complete(cx) {
                        Poll::Ready(Ok(pos)) => {
                            assert!(pos == 0);
                            self.state = TimeshiftFileReaderState::Read;
                            tracing::debug!(
                                path = self.path,
                                "The seek completed, restart streaming"
                            );
                        }
                        Poll::Ready(Err(err)) => {
                            return Poll::Ready(Err(err));
                        }
                        Poll::Pending => {
                            return Poll::Pending;
                        }
                    }
                }
            }
        }
    }
}
