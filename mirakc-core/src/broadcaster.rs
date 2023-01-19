use std::env;
use std::io;
use std::pin::Pin;
use std::time::Duration;
use std::time::Instant;

use actlet::prelude::*;
use bytes::Bytes;
use bytes::BytesMut;
use once_cell::sync::Lazy;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

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
    sender: Option<mpsc::Sender<Bytes>>,
    // Start dropping chunks if the stuck time exceeds this value.
    max_stuck_time: Duration,
    stuck_start_time: Option<Instant>,
    // NOT the total number of dropped bytes.
    // Used for suppressing noisy logs and reporting the number of bytes dropped
    // while streaming stopped.
    dropped_bytes: Option<usize>,
}

impl Subscriber {
    fn new(id: SubscriberId, sender: mpsc::Sender<Bytes>, max_stuck_time: Duration) -> Self {
        Subscriber {
            id,
            sender: Some(sender),
            max_stuck_time,
            stuck_start_time: None,
            dropped_bytes: None,
        }
    }
}

pub struct Broadcaster {
    id: BroadcasterId,
    subscribers: Vec<Subscriber>,
    time_limit: Duration,
    last_received: Instant,
    stream_bounded: bool,
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
            stream_bounded: false,
        }
    }

    fn bind_stream<R>(&mut self, reader: R, ctx: &mut Context<Self>)
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        assert!(!self.stream_bounded);
        let id = self.id.clone();
        let addr = ctx.address().clone();
        let task = async move {
            ChunkSource::new(reader, id, addr).feed_chunks().await;
        };
        ctx.spawn_task(task);
        self.stream_bounded = true;
    }

    fn unbind_stream(&mut self) {
        assert!(self.stream_bounded);
        for subscriber in self.subscribers.iter_mut() {
            subscriber.sender = None;
        }
        self.stream_bounded = false;
    }

    fn subscribe(&mut self, id: SubscriberId, max_stuck_time: Duration) -> BroadcasterStream {
        assert!(self.stream_bounded);
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
        let active_subscribers = self
            .subscribers
            .iter_mut()
            .filter(|subscriber| subscriber.sender.is_some());
        for subscriber in active_subscribers {
            match subscriber.sender.as_ref().unwrap().try_send(chunk.clone()) {
                Ok(_) => {
                    if let Some(dropped_bytes) = subscriber.dropped_bytes {
                        tracing::trace!(
                            broadcaster.id = %self.id,
                            %subscriber.id,
                            dropped_bytes,
                        );
                        subscriber.dropped_bytes = None;
                    }
                    tracing::trace!(
                        broadcaster.id = %self.id,
                        %subscriber.id,
                        chunk.size = chunk_size
                    );
                }
                Err(mpsc::error::TrySendError::Full(chunk)) => {
                    if let Some(dropped_bytes) = subscriber.dropped_bytes {
                        // `subscriber.dropped_bytes` might overflow.
                        subscriber.dropped_bytes = Some(dropped_bytes + chunk.len());
                    } else {
                        tracing::warn!(
                            broadcaster.id = %self.id,
                            %subscriber.id,
                            "No space, drop chunks for a while"
                        );
                        subscriber.dropped_bytes = Some(chunk.len());
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        broadcaster.id = %self.id,
                        %subscriber.id,
                        "Closed, wait for unsubscribe"
                    );
                    subscriber.sender = None;
                }
            }
        }

        self.last_received = Instant::now();
    }

    fn min_capacity(&mut self) -> usize {
        let now = Instant::now();
        self.subscribers
            .iter_mut()
            .filter(|subscriber| subscriber.sender.is_some())
            .filter_map(|subscriber| {
                let cap = subscriber.sender.as_ref().unwrap().capacity();
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

    fn check_timeout(&mut self) {
        let elapsed = self.last_received.elapsed();
        if elapsed > self.time_limit {
            tracing::error!(
                broadcaster.id = %self.id,
                broadcaster.time_limit = %humantime::format_duration(self.time_limit),
                "No packet came from the tuner within the time limit, stop"
            );
            self.unbind_stream();
        }
    }

    fn is_inactive(&self) -> bool {
        self.subscribers.is_empty() && !self.stream_bounded
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
        tracing::debug!(broadcaster.id = %self.id, msg.name = "BindStream");
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
        tracing::debug!(broadcaster.id = %self.id, msg.name = "Subscribe", %msg.id, ?msg.max_stuck_time);
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
        tracing::debug!(broadcaster.id = %self.id, msg.name = "Unsubscribe", %msg.id);
        self.unsubscribe(msg.id);
        if self.is_inactive() {
            tracing::debug!(broadcaster.id = %self.id, "Inactive, stop");
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
        tracing::trace!(broadcaster.id = %self.id, msg.name = "Broadcast", msg.chunk.size = msg.0.len());
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
        tracing::trace!(broadcaster.id = %self.id, msg.name = "MinCapacity");
        self.min_capacity()
    }
}

// check timeout

#[derive(Message)]
struct CheckTimeout;

#[async_trait]
impl Handler<CheckTimeout> for Broadcaster {
    async fn handle(&mut self, _msg: CheckTimeout, _ctx: &mut Context<Self>) {
        tracing::trace!(broadcaster.id = %self.id, msg.name = "CheckTimeout");
        self.check_timeout();
    }
}

// stream ended

#[derive(Message)]
pub struct StreamEnded;

#[async_trait]
impl Handler<StreamEnded> for Broadcaster {
    async fn handle(&mut self, _msg: StreamEnded, ctx: &mut Context<Self>) {
        tracing::debug!(broadcaster.id = %self.id, msg.name = "StreamEnded");
        assert!(self.stream_bounded);
        self.unbind_stream();
        if self.is_inactive() {
            tracing::debug!(broadcaster.id = %self.id, "Inactive, stop");
            ctx.stop();
        }
    }
}

// chunk source

struct ChunkSource<R> {
    reader: R,
    id: BroadcasterId,
    addr: Address<Broadcaster>,
}

impl<R> ChunkSource<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    fn new(reader: R, id: BroadcasterId, addr: Address<Broadcaster>) -> Self {
        ChunkSource { reader, id, addr }
    }

    async fn feed_chunks(&mut self) {
        loop {
            let mut cap = match self.check_capacity().await {
                Some(cap) => cap,
                None => return,
            };
            while cap > 0 {
                let stop = self.feed_chunk().await;
                if stop {
                    return;
                }
                cap -= 1;
            }
        }
    }

    // Some of TS packets outside mirakc may be lost while waiting.
    async fn check_capacity(&mut self) -> Option<usize> {
        let mut waiting = false;
        loop {
            match self.addr.call(MinCapacity).await {
                Ok(0) => {
                    // At least one buffer for a subscriber is full.
                    if !waiting {
                        tracing::debug!(
                            broadcaster.id = %self.id,
                            "No space, wait for a while"
                        );
                        waiting = true;
                    }
                    tokio::time::sleep(*SLEEP_MS).await;
                }
                Ok(cap) => {
                    if cap > Broadcaster::MAX_CHUNKS / 8 {
                        return Some(cap)
                    } else {
                        // Wait
                    }
                }
                Err(err) => {
                    // Normally this error does not happen.
                    // An abnormal termination of the broadcaster might cause
                    // this error.
                    tracing::error!(
                        %err,
                        broadcaster.id = %self.id,
                        "Broadcaster stopped"
                    );
                    return None;
                }
            }
        }
    }

    async fn feed_chunk(&mut self) -> bool {
        match self.read_chunk().await {
            Ok(None) => {
                tracing::debug!(
                    broadcaster.id = %self.id,
                    "EOF, unbind stream"
                );
                self.addr.emit(StreamEnded).await;
                return true;
            }
            Ok(Some(chunk)) => {
                if let Err(err) = self.addr.call(Broadcast(chunk)).await {
                    // Normally this error does not happen.
                    // An abnormal termination of the broadcaster might cause
                    // this error.
                    tracing::error!(
                        %err,
                        broadcaster.id = %self.id,
                        "Broadcaster stopped"
                    );
                    return true;
                }
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    broadcaster.id = %self.id,
                    "Error, unbind stream"
                );
                self.addr.emit(StreamEnded).await;
                return true;
            }
        }
        false // continue
    }

    async fn read_chunk(&mut self) -> io::Result<Option<Bytes>> {
        fn is_full(chunk: &BytesMut) -> bool {
            chunk.capacity() == chunk.len()
        }

        // Allocate a new chunk each time.
        // It will be sent to the broadcaster and shared with subscribers.
        // It will be freed when no subscriber uses it anymore.
        let mut chunk = BytesMut::with_capacity(Broadcaster::CHUNK_SIZE);

        // Fill the chunk with bytes as many as possible in order to avoid
        // a situation that a subscriber's queue is filled with small chunks.
        loop {
            assert!(!is_full(&chunk));
            match self.reader.read_buf(&mut chunk).await? {
                0 => break,                    // EOF
                _ if is_full(&chunk) => break, // Filled
                _ => (),                       // continue
            }
        }
        if chunk.is_empty() {
            Ok(None)
        } else {
            Ok(Some(chunk.freeze()))
        }
    }
}

// broadcaster stream

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
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn test_broadcast() {
        let system = System::new();
        {
            let broadcaster = system
                .spawn_actor(Broadcaster::new(Default::default(), 1000))
                .await;

            broadcaster
                .inspect(|b| b.stream_bounded = true)
                .await
                .unwrap();

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
            broadcaster.emit(StreamEnded).await;

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

            broadcaster
                .inspect(|b| b.stream_bounded = true)
                .await
                .unwrap();

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
            broadcaster.emit(StreamEnded).await;

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

            broadcaster
                .inspect(|b| b.stream_bounded = true)
                .await
                .unwrap();

            let mut stream1 = broadcaster
                .call(Subscribe {
                    id: SubscriberId::new(Default::default(), 1),
                    max_stuck_time: Default::default(),
                })
                .await
                .unwrap();

            let result = broadcaster.call(Broadcast(Bytes::from("hello"))).await;
            assert!(result.is_ok());
            broadcaster.emit(StreamEnded).await;

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
