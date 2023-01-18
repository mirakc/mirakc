use std::env;
use std::io;
use std::pin::Pin;
use std::time::Duration;
use std::time::Instant;

use actlet::prelude::*;
use bytes::Bytes;
use once_cell::sync::Lazy;
use tokio::io::AsyncRead;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::tuner::TunerSessionId as BroadcasterId;
use crate::tuner::TunerSubscriptionId as SubscriberId;

const SLEEP_MS_DEFAULT: Duration = Duration::from_millis(100); // 100ms

static SLEEP_MS: Lazy<Duration> = Lazy::new(|| {
    let ms = env::var("MIRAKC_BROADCASTER_SLEEP_MS")
        .ok()
        .map(|s| {
            s.parse::<u64>()
                .expect("MIRAKC_BROADCASTER_SLEEP_MS must be a u64 value")
        })
        .map(|ms| Duration::from_millis(ms))
        .unwrap_or(SLEEP_MS_DEFAULT);
    tracing::debug!(SLEEP_MS = %humantime::format_duration(ms),
    );
    ms
});

struct Subscriber {
    id: SubscriberId,
    sender: mpsc::Sender<Bytes>,
    max_stuck_time: Duration,
    stuck_start_time: Option<Instant>,
    // Used for suppressing noisy logs.
    drop_count: usize,
}

impl Subscriber {
    fn new(id: SubscriberId, sender: mpsc::Sender<Bytes>, max_stuck_time: Duration) -> Self {
        Subscriber {
            id,
            sender,
            max_stuck_time,
            stuck_start_time: None,
            drop_count: 0,
        }
    }
}

pub struct Broadcaster {
    id: BroadcasterId,
    subscribers: Vec<Subscriber>,
    time_limit: Duration,
    last_received: Instant,
}

impl Broadcaster {
    // large enough for 10 sec buffering.
    const MAX_CHUNKS: usize = 1024;

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
            let mut wait_count = 0;
            loop {
                let mut cap = loop {
                    match addr.call(MinCapacity).await {
                        Ok(0) => {
                            match wait_count {
                                0..=4 => tracing::debug!("No space, wait a while"),
                                5 => tracing::debug!("No space, wait a while (suppressed)"),
                                _ => (), // Suppress noisy logs
                            }
                            tokio::time::sleep(*SLEEP_MS).await;
                            wait_count += 1;
                            continue;
                        }
                        Ok(cap) => {
                            break cap;
                        }
                        Err(err) => {
                            tracing::error!(%err, broadcaster.id = %id, "Broadcaster stopped");
                            return;
                        }
                    }
                };

                while cap > 0 {
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            if let Err(err) = addr.call(Broadcast(chunk)).await {
                                tracing::error!(%err, broadcaster.id = %id, "Broadcaster stopped");
                                return;
                            }
                        }
                        Some(Err(err)) => {
                            tracing::error!(%err, broadcaster.id = %id, "Error, stop");
                            addr.emit(actlet::Stop).await;
                            return;
                        }
                        None => {
                            tracing::debug!(broadcaster.id = %id, "EOF, stop");
                            addr.emit(actlet::Stop).await;
                            return;
                        }
                    }
                    cap -= 1;
                }

                wait_count = 0;
            }
        });
    }

    fn subscribe(&mut self, id: SubscriberId, max_stuck_time: Duration) -> BroadcasterStream {
        let (sender, receiver) = mpsc::channel(Self::MAX_CHUNKS);
        self.subscribers
            .push(Subscriber::new(id, sender, max_stuck_time));
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
                        broadcaster.id = %self.id,
                        %subscriber.id,
                        chunk.size = chunk_size,
                        "Sent the chunk"
                    );
                    subscriber.drop_count = 0;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    match subscriber.drop_count {
                        0..=4 => tracing::warn!(
                            broadcaster.id = %self.id,
                            %subscriber.id,
                            chunk.size = chunk_size,
                            "No space, drop the chunk"
                        ),
                        5 => tracing::warn!(
                            broadcaster.id = %self.id,
                            %subscriber.id,
                            chunk.size = chunk_size,
                            "No space, drop the chunk (suppressed)"
                        ),
                        _ => (),
                    }
                    subscriber.drop_count += 0;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        broadcaster.id = %self.id,
                        %subscriber.id,
                        "Closed, wait for unsubscribe"
                    );
                }
            }
        }

        self.last_received = Instant::now();
    }

    fn min_capacity(&mut self) -> usize {
        let now = Instant::now();
        self.subscribers
            .iter_mut()
            .filter_map(|subscriber| {
                let cap = subscriber.sender.capacity();
                match (cap, subscriber.stuck_start_time) {
                    (0, Some(time)) => {
                        if now - time < subscriber.max_stuck_time {
                            Some(0)
                        } else {
                            None
                        }
                    }
                    (0, None) => {
                        subscriber.stuck_start_time = Some(now);
                        Some(0)
                    }
                    _ => {
                        subscriber.stuck_start_time = None;
                        Some(cap)
                    }
                }
            })
            .min()
            .unwrap_or(Self::MAX_CHUNKS)
    }

    fn check_timeout(&mut self, ctx: &mut Context<Self>) {
        let elapsed = self.last_received.elapsed();
        if elapsed > self.time_limit {
            tracing::error!(
                broadcaster.id = %self.id,
                broadcaster.time_limit = %humantime::format_duration(self.time_limit),
                "No packet came from the tuner within the time limit, stop"
            );
            ctx.stop();
        }
    }
}

#[async_trait]
impl Actor for Broadcaster {
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!(broadcaster.id = %self.id, "Started");
        let addr = ctx.address().clone();
        let mut interval = tokio::time::interval(self.time_limit);
        ctx.spawn_task(async move {
            loop {
                interval.tick().await;
                addr.emit(CheckTimeout).await;
            }
        });
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(broadcaster.id = %self.id, "Stopped");
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
    pub max_stuck_time: Duration,
}

#[async_trait]
impl Handler<Subscribe> for Broadcaster {
    async fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> <Subscribe as Message>::Reply {
        tracing::debug!(msg.name = "Subscribe", %msg.id, ?msg.max_stuck_time);
        self.subscribe(msg.id, msg.max_stuck_time)
    }
}

// unsubscribe

#[derive(Message)]
pub struct Unsubscribe {
    pub id: SubscriberId,
}

#[async_trait]
impl Handler<Unsubscribe> for Broadcaster {
    async fn handle(&mut self, msg: Unsubscribe, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "Unsubscribe", %msg.id);
        self.unsubscribe(msg.id);
        if self.subscribers.is_empty() {
            ctx.stop();
        }
    }
}

// broadcast

#[derive(Message)]
#[reply("()")]
pub struct Broadcast(pub Bytes);

#[async_trait]
impl Handler<Broadcast> for Broadcaster {
    async fn handle(
        &mut self,
        msg: Broadcast,
        _ctx: &mut Context<Self>,
    ) -> <Broadcast as Message>::Reply {
        tracing::trace!(msg.name = "Broadcast", msg.chunk.size = msg.0.len());
        self.broadcast(msg.0);
    }
}

// min capacity

#[derive(Message)]
#[reply("usize")]
pub struct MinCapacity;

#[async_trait]
impl Handler<MinCapacity> for Broadcaster {
    async fn handle(
        &mut self,
        _msg: MinCapacity,
        _ctx: &mut Context<Self>,
    ) -> <MinCapacity as Message>::Reply {
        tracing::trace!(msg.name = "MinCapacity");
        self.min_capacity()
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
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            let mut stream2 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 2),
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            broadcaster
                .call(Broadcast(Bytes::from("hello")))
                .await
                .unwrap();
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
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            let mut stream2 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 2),
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            broadcaster
                .emit(Unsubscribe {
                    id: SubscriberId::new(Default::default(), 1),
                })
                .await;

            broadcaster
                .call(Broadcast(Bytes::from("hello")))
                .await
                .unwrap();
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
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            let result = broadcaster.call(Broadcast(Bytes::from("hello"))).await;
            assert!(result.is_ok());
            broadcaster.emit(actlet::Stop).await;

            let chunk = stream1.next().await;
            assert!(chunk.is_some());

            tokio::time::sleep(Duration::from_millis(51)).await;
            let result = broadcaster.call(Broadcast(Bytes::from("hello"))).await;
            assert!(result.is_err());

            let chunk = stream1.next().await;
            assert!(chunk.is_none());
        }
        system.stop();
    }
}
// </coverage:exclude>
