use std::fmt;
use std::io;
use std::pin::Pin;
use std::time::Duration;
use std::time::Instant;

use actlet::prelude::*;
use bytes::Bytes;
use tokio::io::AsyncRead;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

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

    pub fn new(id: BroadcasterId, time_limit: u64) -> Self {
        Self {
            id,
            subscribers: Vec::new(),
            time_limit: Duration::from_millis(time_limit),
            last_received: Instant::now(),
        }
    }

    fn bind_stream<R>(&self, src: R, ctx: &mut Context<Self>)
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let id = self.id.clone();
        let addr = ctx.address().clone();
        ctx.spawn_task(async move {
            let mut stream = ReaderStream::with_capacity(src, Self::CHUNK_SIZE);

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        addr.emit(Broadcast(chunk)).await;
                    }
                    Err(err) => {
                        tracing::error!("{}: Error, stop: {}", id, err);
                        addr.emit(actlet::Stop).await;
                        break;
                    }
                }
            }
        });
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
                    tracing::trace!(
                        "{}: Sent a chunk of {} bytes to {}",
                        self.id,
                        chunk_size,
                        subscriber.id
                    );
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        "{}: No space for {}, drop the chunk",
                        self.id,
                        subscriber.id
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        "{}: Closed by {}, wait for unsubscribe",
                        self.id,
                        subscriber.id
                    );
                }
            }
        }

        self.last_received = Instant::now();
    }

    fn check_timeout(&mut self, ctx: &mut Context<Self>) {
        let elapsed = self.last_received.elapsed();
        if elapsed > self.time_limit {
            tracing::error!(
                "{}: No packet came from the tuner within the time limit, stop",
                self.id
            );
            ctx.stop();
        }
    }
}

#[async_trait]
impl Actor for Broadcaster {
    async fn started(&mut self, ctx: &mut Context<Self>) {
        let addr = ctx.address().clone();
        let mut interval = tokio::time::interval(self.time_limit);
        ctx.spawn_task(async move {
            loop {
                interval.tick().await;
                addr.emit(CheckTimeout).await;
            }
        });
        tracing::debug!("{}: Started", self.id);
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("{}: Stopped", self.id);
    }
}

// bind stream

#[derive(Message)]
pub struct BindStream<R: Send>(pub R);

#[async_trait]
impl<R> Handler<BindStream<R>> for Broadcaster
where
    R: AsyncRead + Send + Unpin + 'static,
{
    async fn handle(&mut self, msg: BindStream<R>, ctx: &mut Context<Self>) {
        self.bind_stream(msg.0, ctx);
    }
}

// subscribe

#[derive(Message)]
#[reply("BroadcasterStream")]
pub struct Subscribe {
    pub id: SubscriberId,
}

impl fmt::Display for Subscribe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Subscribe with {}", self.id)
    }
}

#[async_trait]
impl Handler<Subscribe> for Broadcaster {
    async fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> <Subscribe as Message>::Reply {
        tracing::debug!("{}", msg);
        self.subscribe(msg.id)
    }
}

// unsubscribe

#[derive(Message)]
pub struct Unsubscribe {
    pub id: SubscriberId,
}

impl fmt::Display for Unsubscribe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unsubscribe with {}", self.id)
    }
}

#[async_trait]
impl Handler<Unsubscribe> for Broadcaster {
    async fn handle(&mut self, msg: Unsubscribe, ctx: &mut Context<Self>) {
        tracing::debug!("{}", msg);
        self.unsubscribe(msg.id);
        if self.subscribers.is_empty() {
            ctx.stop();
        }
    }
}

// chunk

#[derive(Message)]
pub struct Broadcast(pub Bytes);

#[async_trait]
impl Handler<Broadcast> for Broadcaster {
    async fn handle(&mut self, msg: Broadcast, _ctx: &mut Context<Self>) {
        tracing::trace!(msg.name = "Broadcast", msg.chunk.bytes = msg.0.len());
        self.broadcast(msg.0);
    }
}

#[derive(Message)]
struct CheckTimeout;

#[async_trait]
impl Handler<CheckTimeout> for Broadcaster {
    async fn handle(&mut self, _msg: CheckTimeout, ctx: &mut Context<Self>) {
        tracing::trace!(msg.name = "CheckTimeout");
        self.check_timeout(ctx);
    }
}

// stream

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
        cx: &mut std::task::Context,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.0)
            .poll_next(cx)
            .map(|item| item.map(|chunk| Ok(chunk)))
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcast() {
        let system = System::new();
        {
            let broadcaster = system
                .spawn_actor(Broadcaster::new(Default::default(), 1000))
                .await;

            let mut stream1 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 1),
                })
                .await
                .unwrap();

            let mut stream2 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 2),
                })
                .await
                .unwrap();

            broadcaster.emit(Broadcast(Bytes::from("hello"))).await;
            broadcaster.emit(actlet::Stop).await;

            let chunk = stream1.next().await;
            assert!(chunk.is_some());

            let chunk = stream2.next().await;
            assert!(chunk.is_some());
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_unsubscribe() {
        let system = System::new();
        {
            let broadcaster = system
                .spawn_actor(Broadcaster::new(Default::default(), 1000))
                .await;

            let mut stream1 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 1),
                })
                .await
                .unwrap();

            let mut stream2 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 2),
                })
                .await
                .unwrap();

            broadcaster
                .emit(Unsubscribe {
                    id: SubscriberId::new(Default::default(), 1),
                })
                .await;

            broadcaster.emit(Broadcast(Bytes::from("hello"))).await;
            broadcaster.emit(actlet::Stop).await;

            let chunk = stream1.next().await;
            assert!(chunk.is_none());

            let chunk = stream2.next().await;
            assert!(chunk.is_some());
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_timeout() {
        let system = System::new();
        {
            let broadcaster = system
                .spawn_actor(Broadcaster::new(Default::default(), 50))
                .await;

            let mut stream1 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 1),
                })
                .await
                .unwrap();

            broadcaster.emit(Broadcast(Bytes::from("hello"))).await;
            broadcaster.emit(actlet::Stop).await;

            let chunk = stream1.next().await;
            assert!(chunk.is_some());

            tokio::time::sleep(Duration::from_millis(51)).await;
            broadcaster.emit(Broadcast(Bytes::from("hello"))).await;

            let chunk = stream1.next().await;
            assert!(chunk.is_none());
        }
        system.stop();
    }
}
// </coverage:exclude>
