use std::fmt;
use std::io;
use std::pin::Pin;
use std::time::{Duration, Instant};

use actix::prelude::*;
use bytes::Bytes;
use humantime;
use tokio::io::AsyncRead;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::chunk_stream::ChunkStream;
use crate::tuner::TunerSessionId as BroadcasterId;
use crate::tuner::TunerSubscriptionId as SubscriberId;

struct Subscriber {
    id: SubscriberId,
    sender: mpsc::Sender<Bytes>,
}

pub struct Broadcaster {
    id: BroadcasterId,
    subscribers: Vec<Subscriber>,
    time_limit: Duration,
    last_received: Instant,
}

impl Broadcaster {
    // large enough for 10 sec buffering.
    const MAX_CHUNKS: usize = 1000;

    // 32 KiB, large enough for 10 ms buffering.
    const CHUNK_SIZE: usize = 4096 * 8;

    pub fn new<R>(
        id: BroadcasterId,
        source: R,
        time_limit: u64,
        ctx: &mut Context<Self>,
    ) -> Self
    where
        R: AsyncRead + Unpin + 'static,
    {
        let stream = ChunkStream::new(source, Self::CHUNK_SIZE);
        let _ = Self::add_stream(stream, ctx);
        Self {
            id,
            subscribers: Vec::new(),
            time_limit: Duration::from_millis(time_limit),
            last_received: Instant::now(),
        }
    }

    fn subscribe(&mut self, id: SubscriberId) -> BroadcasterStream {
        let (sender, receiver) = mpsc::channel(Self::MAX_CHUNKS);
        self.subscribers.push(Subscriber { id, sender });
        BroadcasterStream::new(receiver)
    }

    fn unsubscribe(&mut self, id: SubscriberId) {
        // Log warning message if the user haven't subscribed.
        self.subscribers.retain(|subscriber| subscriber.id != id);
    }

    fn broadcast(&mut self, chunk: Bytes) {
        let chunk_size = chunk.len();
        for subscriber in self.subscribers.iter_mut() {
            match subscriber.sender.try_send(chunk.clone()) {
                Ok(_) => {
                    tracing::trace!("{}: Sent a chunk of {} bytes to {}",
                                    self.id, chunk_size, subscriber.id);
                },
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!("{}: No space for {}, drop the chunk",
                                   self.id, subscriber.id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!("{}: Closed by {}, wait for unsubscribe",
                                    self.id, subscriber.id);
                }
            }
        }

        self.last_received = Instant::now();
    }

    fn check_timeout(&mut self, ctx: &mut Context<Self>) {
        let elapsed = self.last_received.elapsed();
        if  elapsed > self.time_limit {
            tracing::error!("{}: No packet from the tuner for {}, stop",
                            self.id, humantime::format_duration(elapsed));
            ctx.stop();
        }
    }
}

impl Actor for Broadcaster {
    type Context = actix::Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(self.time_limit, Self::check_timeout);
        tracing::debug!("{}: Started", self.id);
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        tracing::debug!("{}: Stopped", self.id);
    }
}

// subscribe

#[derive(Message)]
#[rtype(result = "BroadcasterStream")]
pub struct SubscribeMessage {
    pub id: SubscriberId
}

impl fmt::Display for SubscribeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Subscribe with {}", self.id)
    }
}

impl Handler<SubscribeMessage> for Broadcaster {
    type Result = BroadcasterStream;

    fn handle(
        &mut self,
        msg: SubscribeMessage,
        _: &mut Self::Context
    ) -> Self::Result {
        tracing::debug!("{}", msg);
        self.subscribe(msg.id)
    }
}

// unsubscribe

#[derive(Message)]
#[rtype(result = "()")]
pub struct UnsubscribeMessage {
    pub id: SubscriberId
}

impl fmt::Display for UnsubscribeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unsubscribe with {}", self.id)
    }
}

impl Handler<UnsubscribeMessage> for Broadcaster {
    type Result = ();

    fn handle(
        &mut self,
        msg: UnsubscribeMessage,
        _: &mut Self::Context
    ) -> Self::Result {
        tracing::debug!("{}", msg);
        self.unsubscribe(msg.id)
    }
}

// stream handler

impl StreamHandler<io::Result<Bytes>> for Broadcaster {
    fn handle(&mut self, chunk: io::Result<Bytes>, ctx: &mut Context<Self>) {
        match chunk {
            Ok(chunk) => {
                self.broadcast(chunk);
            }
            Err(err) => {
                tracing::error!("{}: Error, stop: {}", self.id, err);
                ctx.stop();
            }
        }
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("{}: EOS reached, stop", self.id);
        ctx.stop();
    }
}

// stream

#[derive(MessageResponse)]
#[cfg_attr(test, derive(Debug))]
pub struct BroadcasterStream(ReceiverStream<Bytes>);

impl BroadcasterStream {
    fn new(rx: mpsc::Receiver<Bytes>) -> Self {
        Self(ReceiverStream::new(rx))
    }

    #[cfg(test)]
    pub fn new_for_test() -> (mpsc::Sender<Bytes>, Self) {
        let (tx, rx) = mpsc::channel(10);
        (tx, BroadcasterStream::new(rx))
    }
}

impl Stream for BroadcasterStream {
    type Item = io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.0)
            .poll_next(cx)
            .map(|item| item.map(|chunk| Ok(chunk)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Poll, Context};
    use tokio::io::ReadBuf;
    use tokio::sync::mpsc;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;

    #[actix::test]
    async fn test_broadcast() {
        let (tx, rx) = mpsc::channel(1);

        let broadcaster = Broadcaster::create(|ctx| {
            Broadcaster::new(Default::default(), DataSource::new(rx), 1000, ctx)
        });

        let mut stream1 = broadcaster.send(SubscribeMessage {
            id: SubscriberId::new(Default::default(), 1)
        }).await.unwrap();

        let mut stream2 = broadcaster.send(SubscribeMessage {
            id: SubscriberId::new(Default::default(), 2)
        }).await.unwrap();

        let _ = tx.send(Bytes::from("hello")).await;

        let chunk = stream1.next().await;
        assert!(chunk.is_some());

        let chunk = stream2.next().await;
        assert!(chunk.is_some());
    }

    #[actix::test]
    async fn test_unsubscribe() {
        let (tx, rx) = mpsc::channel(1);

        let broadcaster = Broadcaster::create(|ctx| {
            Broadcaster::new(Default::default(), DataSource::new(rx), 1000, ctx)
        });

        let mut stream1 = broadcaster.send(SubscribeMessage {
            id: SubscriberId::new(Default::default(), 1)
        }).await.unwrap();

        let mut stream2 = broadcaster.send(SubscribeMessage {
            id: SubscriberId::new(Default::default(), 2)
        }).await.unwrap();

        broadcaster.send(UnsubscribeMessage {
            id: SubscriberId::new(Default::default(), 1)
        }).await.unwrap();

        let _ = tx.send(Bytes::from("hello")).await;

        let chunk = stream1.next().await;
        assert!(chunk.is_none());

        let chunk = stream2.next().await;
        assert!(chunk.is_some());
    }

    #[actix::test]
    async fn test_timeout() {
        let (tx, rx) = mpsc::channel(1);

        let broadcaster = Broadcaster::create(|ctx| {
            Broadcaster::new(Default::default(), DataSource::new(rx), 50, ctx)
        });

        let mut stream1 = broadcaster.send(SubscribeMessage {
            id: SubscriberId::new(Default::default(), 1)
        }).await.unwrap();

        let _ = tx.send(Bytes::from("hello")).await;

        let chunk = stream1.next().await;
        assert!(chunk.is_some());

        while broadcaster.connected() {
            // Yield in order to process messages on the Broadcaster.
            tokio::task::yield_now().await;
        }

        let _ = tx.send(Bytes::from("hello")).await;

        let chunk = stream1.next().await;
        assert!(chunk.is_none());
    }

    // we can use `futures::stream::repeat(1)` as data source in tests once
    // actix/actix/pull/363 is release.
    struct DataSource(ReceiverStream<Bytes>);

    impl DataSource {
        fn new(rx: mpsc::Receiver<Bytes>) -> Self {
            DataSource(ReceiverStream::new(rx))
        }
    }

    impl AsyncRead for DataSource {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &mut ReadBuf,
        ) -> Poll<io::Result<()>> {
            match Pin::new(&mut self.0).poll_next(cx) {
                Poll::Ready(Some(chunk)) => {
                    let len = chunk.len().min(buf.remaining());
                    buf.put_slice(&chunk[..len]);
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(None) => Poll::Ready(Ok(())),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}
