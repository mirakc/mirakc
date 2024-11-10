use std::str::FromStr;
use std::sync::Arc;

use actlet::prelude::*;
use chrono_jst::Jst;
use indexmap::IndexMap;

use super::recorder::TimeshiftRecorder;
use super::*;
use crate::config::Config;
use crate::epg;
use crate::error::Error;
use crate::models::*;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;

pub struct TimeshiftManager<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    recorders: IndexMap<String, RecorderHolder<T>>,
    event_emitters: EmitterRegistry<TimeshiftEvent>,
}

impl<T, E> TimeshiftManager<T, E> {
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        TimeshiftManager {
            config,
            tuner_manager,
            epg,
            recorders: IndexMap::new(),
            event_emitters: Default::default(),
        }
    }
}

// actor

#[async_trait]
impl<T, E> Actor for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");

        if !self.config.timeshift.is_enabled() {
            tracing::info!("Timeshift recording is disabled");
            return;
        }

        self.epg
            .call(epg::RegisterEmitter::ServicesUpdated(ctx.emitter()))
            .await
            .expect("Failed to register the emitter");

        // Spawn recorders regardless of whether its service is available or not.
        // Records should be accessible even if the service is unavailable.
        for (index, name) in self.config.timeshift.recorders.keys().enumerate() {
            let addr = ctx
                .spawn_actor(TimeshiftRecorder::new(
                    index,
                    name.clone(),
                    self.config.clone(),
                    self.tuner_manager.clone(),
                    ctx.emitter(),
                ))
                .await;
            let holder = RecorderHolder::new(addr);
            self.recorders.insert(name.clone(), holder);
        }

        // Perform health check for each recorder at 50s every minute.
        let task = {
            let addr = ctx.address().clone();
            async move {
                let schedule = cron::Schedule::from_str("50 * * * * * *").unwrap();
                for next in schedule.upcoming(Jst) {
                    let interval = match (next - Jst::now()).to_std() {
                        Ok(v) => v,
                        Err(_) => {
                            // This situation happens if `cron::ScheduleIterator::next()`
                            // was called just before the next scheduled time.
                            // Use the next one.
                            continue;
                        }
                    };
                    tokio::time::sleep(interval).await;
                    if addr.call(HealthCheck).await.is_err() {
                        // The manager has been gone.
                        return;
                    }
                }
            }
        };
        ctx.spawn_task(task);
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopping...");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        for (name, holder) in self.recorders.iter() {
            tracing::debug!(recorder.name = name, "Waiting for the recorder to stop...");
            holder.addr.wait().await;
        }
        tracing::debug!("Stopped");
    }
}

// register emitter

#[async_trait]
impl<T, E> Handler<RegisterEmitter> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        let id = self.event_emitters.register(msg.0.clone());
        tracing::debug!(msg.name = "RegisterEmitter", id);
        if id != 0 {
            // Sending many messages in the message handler may cause a deadlock
            // when the number of messages to be sent is larger than the capacity
            // of the emitter's channel.  See the issue #705 for example.
            ctx.set_post_process(RegisterEmitterPostProcess(msg.0));
        }
        id
    }
}

#[derive(Message)]
struct RegisterEmitterPostProcess(Emitter<TimeshiftEvent>);

#[async_trait]
impl<T, E> Handler<RegisterEmitterPostProcess> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RegisterEmitterPostProcess, _ctx: &mut Context<Self>) {
        let emitter = msg.0;
        for (recorder, holder) in self.recorders.iter() {
            let msg = TimeshiftEvent::Timeline {
                recorder: recorder.clone(),
                start_time: holder.start_time,
                end_time: holder.end_time,
                duration: holder.duration,
            };
            emitter.emit(msg).await;
            if holder.started {
                let msg = TimeshiftEvent::Started {
                    recorder: recorder.clone(),
                };
                emitter.emit(msg).await;
            }
            if let Some(record_id) = holder.current_record_id {
                let msg = TimeshiftEvent::RecordStarted {
                    recorder: recorder.clone(),
                    record_id,
                };
                emitter.emit(msg).await;
            }
        }
    }
}

// unregister emitter

#[async_trait]
impl<T, E> Handler<UnregisterEmitter> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: UnregisterEmitter, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UnregisterEmitter", id = msg.0);
        self.event_emitters.unregister(msg.0);
    }
}

// query timeshift recorders

#[async_trait]
impl<T, E> Handler<QueryTimeshiftRecorders> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryTimeshiftRecorders,
        _ctx: &mut Context<Self>,
    ) -> <QueryTimeshiftRecorders as Message>::Reply {
        let mut models = vec![];
        for (index, holder) in self.recorders.values().enumerate() {
            let msg = QueryTimeshiftRecorder {
                recorder: TimeshiftRecorderQuery::ByIndex(index),
            };
            models.push(holder.addr.call(msg).await??);
        }
        Ok(models)
    }
}

// forward messages to a specified recorder

macro_rules! impl_proxy_handler {
    ($msg:ty) => {
        #[async_trait]
        impl<T, E> Handler<$msg> for TimeshiftManager<T, E>
        where
            T: Clone + Send + Sync + 'static,
            T: Call<StartStreaming>,
            T: TriggerFactory<StopStreaming>,
            E: Send + Sync + 'static,
            E: Call<epg::RegisterEmitter>,
        {
            async fn handle(
                &mut self,
                msg: $msg,
                _ctx: &mut Context<Self>,
            ) -> <$msg as Message>::Reply {
                let maybe_recorder = match msg.recorder {
                    TimeshiftRecorderQuery::ByIndex(index) => self
                        .recorders
                        .get_index(index)
                        .map(|(_, holder)| &holder.addr)
                        .ok_or(Error::RecordNotFound),
                    TimeshiftRecorderQuery::ByName(ref name) => self
                        .recorders
                        .get(name)
                        .map(|holder| &holder.addr)
                        .ok_or(Error::RecordNotFound),
                };
                maybe_recorder?.call(msg).await?
            }
        }
    };
}

impl_proxy_handler!(QueryTimeshiftRecorder);
impl_proxy_handler!(QueryTimeshiftRecords);
impl_proxy_handler!(QueryTimeshiftRecord);
impl_proxy_handler!(CreateTimeshiftLiveStreamSource);
impl_proxy_handler!(CreateTimeshiftRecordStreamSource);

// health check

#[async_trait]
impl<T, E> Handler<HealthCheck> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: HealthCheck,
        ctx: &mut Context<Self>,
    ) -> <HealthCheck as Message>::Reply {
        tracing::debug!(msg.name = "HealthCheck");
        for (index, (name, holder)) in self.recorders.iter_mut().enumerate() {
            if holder.addr.call(HealthCheck).await.is_err() {
                // The recorder has been gone.
                assert!(!holder.addr.is_available());

                // Emit events if needed.

                if let Some(record_id) = holder.current_record_id {
                    let msg = TimeshiftEvent::RecordEnded {
                        recorder: name.clone(),
                        record_id,
                    };
                    self.event_emitters.emit(msg).await;
                }
                holder.current_record_id = None;

                if holder.started {
                    let msg = TimeshiftEvent::Stopped {
                        recorder: name.clone(),
                    };
                    self.event_emitters.emit(msg).await;
                }
                holder.started = false;

                // Respawn the recorder.
                let addr = ctx
                    .spawn_actor(TimeshiftRecorder::new(
                        index,
                        name.clone(),
                        self.config.clone(),
                        self.tuner_manager.clone(),
                        ctx.emitter(),
                    ))
                    .await;
                holder.addr = addr;
            }
        }
    }
}

// services updated

#[async_trait]
impl<T, E> Handler<epg::ServicesUpdated> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ServicesUpdated, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        for (name, holder) in self.recorders.iter() {
            let config = self.config.timeshift.recorders.get(name).unwrap();
            let msg = ServiceUpdated {
                service: msg.services.get(&config.service_id).cloned(),
            };
            holder.addr.emit(msg).await;
        }
    }
}

// timeshift event

#[async_trait]
impl<T, E> Handler<TimeshiftEvent> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: TimeshiftEvent, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "TimeshiftEvent");
        // Update the recorder states.
        // See the comment in `RecorderHolder`.
        match msg {
            TimeshiftEvent::Timeline {
                ref recorder,
                start_time,
                end_time,
                duration,
            } => {
                if let Some(holder) = self.recorders.get_mut(recorder) {
                    holder.start_time = start_time;
                    holder.end_time = end_time;
                    holder.duration = duration;
                }
            }
            TimeshiftEvent::Started { ref recorder } => {
                if let Some(holder) = self.recorders.get_mut(recorder) {
                    holder.started = true;
                }
            }
            TimeshiftEvent::Stopped { ref recorder } => {
                if let Some(holder) = self.recorders.get_mut(recorder) {
                    holder.started = false;
                    // The recording pipeline may stop before emitting
                    // TimeshiftEvent::RecordEnded.
                    if let Some(record_id) = holder.current_record_id {
                        let msg = TimeshiftEvent::RecordStarted {
                            recorder: recorder.clone(),
                            record_id,
                        };
                        self.event_emitters.emit(msg).await;
                    }
                    holder.current_record_id = None;
                }
            }
            TimeshiftEvent::RecordStarted {
                ref recorder,
                record_id,
            } => {
                if let Some(holder) = self.recorders.get_mut(recorder) {
                    holder.current_record_id = Some(record_id);
                }
            }
            TimeshiftEvent::RecordUpdated { .. } => {
                // Nothing to do.
            }
            TimeshiftEvent::RecordEnded { ref recorder, .. } => {
                if let Some(holder) = self.recorders.get_mut(recorder) {
                    holder.current_record_id = None;
                }
            }
        }
        self.event_emitters.emit(msg).await;
    }
}

// models

#[derive(Clone)]
struct RecorderHolder<T> {
    addr: Address<TimeshiftRecorder<T>>,

    // Cache the following recorder states in order to emit preceding events
    // when a new emitter is registered.
    //
    // We can fetch these states by using `QueryTimeshiftRecorder` in the
    // `RegisterEmitter` handler, but this may break consistency of the event
    // sequence.  Because a `TimeshiftEvent` message could be sent to the
    // manager while it's handling the RegisterEmitter message.
    start_time: Option<DateTime<Jst>>,
    end_time: Option<DateTime<Jst>>,
    duration: Duration,
    started: bool,
    current_record_id: Option<TimeshiftRecordId>,
}

impl<T> RecorderHolder<T> {
    fn new(addr: Address<TimeshiftRecorder<T>>) -> Self {
        RecorderHolder {
            addr,
            start_time: None,
            end_time: None,
            duration: Duration::zero(),
            started: false,
            current_record_id: None,
        }
    }
}
