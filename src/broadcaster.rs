use std::io;

use actix::prelude::*;
use actix::dev::{MessageResponse, ResponseChannel};
use bytes::Bytes;
use log;
use tokio::io::AsyncRead;
use tokio::sync::mpsc;

use crate::chunk_stream::ChunkStream;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::tuner::TunerSessionId as BroadcasterId;
use crate::tuner::TunerSubscriptionId as SubscriberId;

struct Subscriber {
    id: SubscriberId,
    sender: mpsc::Sender<Bytes>,
}

pub struct Broadcaster {
    id: BroadcasterId,
    subscribers: Vec<Subscriber>,
}

impl Broadcaster {
    // 100 chunks, large enough for 5 sec buffering.
    const MAX_CHUNKS: usize = 500;

    // 32 KiB, large enough for 10 ms buffering.
    const CHUNK_SIZE: usize = 4096 * 8;

    pub fn new<R>(
        id: BroadcasterId,
        source: R,
        ctx: &mut Context<Self>
    ) -> Self
    where
        R: AsyncRead + Unpin + 'static,
    {
        let stream = ChunkStream::new(source, Self::CHUNK_SIZE);
        let _ = Self::add_stream(stream, ctx);
        Self { id, subscribers: Vec::new() }
    }

    fn subscribe(&mut self, id: SubscriberId) -> MpegTsStream {
        let (sender, receiver) = mpsc::channel(Self::MAX_CHUNKS);
        self.subscribers.push(Subscriber { id, sender });
        MpegTsStream::new(id, receiver)
    }

    fn unsubscribe(&mut self, id: SubscriberId) {
        // Log warning message if the user haven't subscribed.
        self.subscribers.retain(|subscriber| subscriber.id != id);
    }

    fn broadcast(&mut self, chunk: Bytes) {
        for subscriber in self.subscribers.iter_mut() {
            match subscriber.sender.try_send(chunk.clone()) {
                Ok(_) => {},
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::warn!("{}: No space for {}, drop the chunk",
                               self.id, subscriber.id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    log::debug!("{}: Closed by {}, wait for unsubscribe",
                                self.id, subscriber.id);
                }
            }
        }
    }
}

impl Actor for Broadcaster {
    type Context = actix::Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        log::debug!("{}: Started", self.id);
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::debug!("{}: Stopped", self.id);
    }
}

// subscribe

pub struct SubscribeMessage {
    pub id: SubscriberId
}

impl Message for SubscribeMessage {
    type Result = MpegTsStream;
}

impl<A, M> MessageResponse<A, M> for MpegTsStream
where
    A: Actor,
    M: Message<Result = MpegTsStream>,
{
    fn handle<R: ResponseChannel<M>>(self, _: &mut A::Context, tx: Option<R>) {
        if let Some(tx) = tx {
            tx.send(self);
        }
    }
}

impl Handler<SubscribeMessage> for Broadcaster {
    type Result = MpegTsStream;

    fn handle(
        &mut self,
        msg: SubscribeMessage,
        _: &mut Self::Context
    ) -> Self::Result {
        self.subscribe(msg.id)
    }
}

// unsubscribe

pub struct UnsubscribeMessage {
    pub id: SubscriberId
}

impl Message for UnsubscribeMessage {
    type Result = ();
}

impl Handler<UnsubscribeMessage> for Broadcaster {
    type Result = ();

    fn handle(
        &mut self,
        msg: UnsubscribeMessage,
        _: &mut Self::Context
    ) -> Self::Result {
        self.unsubscribe(msg.id)
    }
}

// stream handler

impl StreamHandler<io::Result<Bytes>> for Broadcaster {
    fn handle(&mut self, chunk: io::Result<Bytes>, ctx: &mut Context<Self>) {
        match chunk {
            Ok(chunk) => self.broadcast(chunk),
            Err(err) => {
                log::error!("{}: Error, stop: {}", self.id, err);
                ctx.stop();
            }
        }
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        log::debug!("{}: EOS reached, stop", self.id);
        ctx.stop();
    }
}
