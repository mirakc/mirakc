use std::collections::HashMap;
use std::sync::Arc;

use actlet::prelude::*;
use maplit::hashset;

use crate::config::Config;
use crate::config::LocalOnairProgramTrackerConfig;
use crate::config::LocalOnairProgramTrackerUses;
use crate::config::OnairProgramTrackerConfig;
use crate::config::RemoteOnairProgramTrackerConfig;
use crate::epg;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::error::Error;
use crate::models::ServiceId;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerSubscriptionId;

use super::OnairProgramChanged;
use super::TrackerStopped;
use super::local::LocalTracker;
use super::remote::RemoteTracker;

pub struct OnairProgramManager<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,

    cache: HashMap<ServiceId, OnairProgram>,
    trackers: HashMap<String, Tracker<T, E>>,
    temporal_services: HashMap<String, ServiceId>,

    program_changed: EmitterRegistry<OnairProgramChanged>,
}

// The address contained in each variant is never used at this point.
// However, it should be kept by the variant theoretically.
#[allow(dead_code)]
enum Tracker<T, E> {
    Local(Address<LocalTracker<T, E>>),
    Remote(Address<RemoteTracker<E>>),
}

impl<T, E> OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        OnairProgramManager {
            config,
            tuner_manager,
            epg,
            cache: Default::default(),
            trackers: Default::default(),
            temporal_services: Default::default(),
            program_changed: Default::default(),
        }
    }
}

// actor

#[async_trait]
impl<T, E> Actor for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");

        if !self.config.has_onair_program_trackers() {
            tracing::info!("No on-air program tracker is defined");
            return;
        }

        if let Err(err) = self
            .epg
            .call(epg::RegisterEmitter::ServicesUpdated(ctx.emitter()))
            .await
        {
            tracing::error!(?err, "Failed to register emitter for epg::ServicesUpdated");
            return;
        }

        for (name, config) in self.config.onair_program_trackers.iter() {
            let changed = ctx.emitter();
            let tracker = match config {
                OnairProgramTrackerConfig::Local(config) => {
                    self.spawn_local_tracker(name, config, ctx, changed, None)
                        .await
                }
                OnairProgramTrackerConfig::Remote(config) => {
                    self.spawn_remote_tracker(name, config, ctx, changed).await
                }
            };
            self.trackers.insert(name.to_string(), tracker);
        }
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopping...");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

impl<T, E> OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
{
    async fn spawn_local_tracker<C: Spawn>(
        &self,
        name: &str,
        config: &Arc<LocalOnairProgramTrackerConfig>,
        ctx: &C,
        changed: Emitter<OnairProgramChanged>,
        stopped: Option<Emitter<TrackerStopped>>,
    ) -> Tracker<T, E> {
        let tracker = ctx
            .spawn_actor(LocalTracker::new(
                name.to_string(),
                config.clone(),
                self.tuner_manager.clone(),
                self.epg.clone(),
                changed,
                stopped,
            ))
            .await;
        tracing::info!(tracker.name = name, "Spawned");
        Tracker::Local(tracker)
    }

    async fn spawn_remote_tracker<C: Spawn>(
        &self,
        name: &str,
        config: &Arc<RemoteOnairProgramTrackerConfig>,
        ctx: &C,
        changed: Emitter<OnairProgramChanged>,
    ) -> Tracker<T, E> {
        let tracker = ctx
            .spawn_actor(RemoteTracker::new(
                name.to_string(),
                config.clone(),
                self.epg.clone(),
                changed,
            ))
            .await;
        tracing::info!(tracker.name = name, "Spawned");
        Tracker::Remote(tracker)
    }
}

// query on-air programs

#[derive(Message)]
#[reply(HashMap<ServiceId, OnairProgram>)]
pub struct QueryOnairPrograms;

#[async_trait]
impl<T, E> Handler<QueryOnairPrograms> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryOnairPrograms,
        _ctx: &mut Context<Self>,
    ) -> <QueryOnairPrograms as Message>::Reply {
        tracing::debug!(msg.name = "QueryOnairPrograms");
        self.cache.clone()
    }
}

// query on-air program

#[derive(Message)]
#[reply(Result<OnairProgram, Error>)]
pub struct QueryOnairProgram {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T, E> Handler<QueryOnairProgram> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryOnairProgram,
        _ctx: &mut Context<Self>,
    ) -> <QueryOnairProgram as Message>::Reply {
        tracing::debug!(msg.name = "QueryOnairProgram", %msg.service_id);
        self.cache
            .get(&msg.service_id)
            .cloned()
            .ok_or(Error::ServiceNotFound)
    }
}

// register emitter

#[derive(Message)]
#[reply(usize)]
pub struct RegisterEmitter(pub Emitter<OnairProgramChanged>);

#[async_trait]
impl<T, E> Handler<RegisterEmitter> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        let id = self.program_changed.register(msg.0.clone());
        tracing::debug!(msg.name = "RegisterEmitter", id);
        if id != 0 {
            // Sending many messages in the message handler may cause a deadlock
            // when the number of messages to be sent is larger than the capacity
            // of the emitter's channel.  See the issue #705 for details.
            ctx.set_post_process(RegisterEmitterPostProcess(msg.0))
        }
        id
    }
}

#[derive(Message)]
struct RegisterEmitterPostProcess(Emitter<OnairProgramChanged>);

#[async_trait]
impl<T, E> Handler<RegisterEmitterPostProcess> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RegisterEmitterPostProcess, _ctx: &mut Context<Self>) {
        let emitter = msg.0;
        for (&service_id, entry) in self.cache.iter() {
            let msg = OnairProgramChanged {
                service_id,
                current: entry.current.clone(),
                next: entry.next.clone(),
            };
            emitter.emit(msg).await;
        }
    }
}

// unregister emitter

#[derive(Message)]
pub struct UnregisterEmitter(pub usize);

#[async_trait]
impl<T, E> Handler<UnregisterEmitter> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: UnregisterEmitter, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UnregisterEmitter", id = msg.0);
        self.program_changed.unregister(msg.0);
    }
}

// spawn temporal local tracker

#[derive(Message)]
#[reply()]
pub struct SpawnTemporalTracker {
    pub service: EpgService,
    pub stream_id: TunerSubscriptionId,
}

#[async_trait]
impl<T, E> Handler<SpawnTemporalTracker> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: SpawnTemporalTracker,
        ctx: &mut Context<Self>,
    ) -> <SpawnTemporalTracker as Message>::Reply {
        tracing::debug!(msg.name = "SpawnTempralLocalTracker", %msg.service.id);

        let service_id = msg.service.id;
        for config in self.config.onair_program_trackers.values() {
            if config.matches(&msg.service) {
                tracing::info!(service.id = %service_id, "Tracker for the service is already running");
                return;
            }
        }

        let name = format!(".{}", msg.stream_id);
        if self.trackers.contains_key(&name) {
            tracing::info!(tracker.name = name, "Temporal tracker is already running");
            return;
        }

        // Multiple temporal trackers for the same service may be spawned.
        let config = Arc::new(LocalOnairProgramTrackerConfig {
            channel_types: hashset![msg.service.channel.channel_type],
            services: hashset![service_id],
            excluded_services: hashset![],
            command: LocalOnairProgramTrackerConfig::default_command(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: Some(msg.stream_id),
        });
        let changed = ctx.emitter();
        let stopped = Some(ctx.emitter());
        let tracker = self
            .spawn_local_tracker(&name, &config, ctx, changed, stopped)
            .await;
        self.trackers.insert(name.clone(), tracker);
        self.temporal_services.insert(name.clone(), service_id);
        tracing::info!(tracker.name = name, service.id = %service_id, "Created temporal tracker");
    }
}

impl OnairProgramTrackerConfig {
    fn matches(&self, service: &EpgService) -> bool {
        match self {
            Self::Local(config) => config.matches(service),
            Self::Remote(config) => config.matches(service.id),
        }
    }
}

// services updated

#[async_trait]
impl<T, E> Handler<epg::ServicesUpdated> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ServicesUpdated, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        self.cache
            .retain(|service_id, _| msg.services.contains_key(service_id));
    }
}

// on-air program changed

#[async_trait]
impl<T, E> Handler<OnairProgramChanged> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: OnairProgramChanged, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "OnairProgramChanged");
        if let Some(program) = msg.current.clone() {
            self.update_current_program(program).await;
        }
        if let Some(program) = msg.next.clone() {
            self.update_next_program(program).await;
        }
        self.program_changed.emit(msg).await;
    }
}

impl<T, E> OnairProgramManager<T, E> {
    async fn update_current_program(&mut self, program: Arc<EpgProgram>) {
        let program_id = program.id;
        let service_id = program_id.into();
        self.cache.entry(service_id).or_default().current = Some(program);
        tracing::debug!(service.id = %service_id, current.program.id = %program_id);
    }

    async fn update_next_program(&mut self, program: Arc<EpgProgram>) {
        let program_id = program.id;
        let service_id = program_id.into();
        self.cache.entry(service_id).or_default().next = Some(program);
        tracing::debug!(service.id = %service_id, next.program.id = %program_id);
    }
}

// tracker stopped

#[async_trait]
impl<T, E> Handler<TrackerStopped> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: TrackerStopped, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "TrackerStopped", msg.tracker);
        match self.trackers.remove(&msg.tracker) {
            Some(_) => {
                tracing::info!(tracker.name = msg.tracker, "Removed temporal tracker");
            }
            None => {
                tracing::error!(
                    tracker.name = msg.tracker,
                    "INCONSISTENT: Temporal tracker has already been removed"
                );
            }
        }
        if let Some(service_id) = self.temporal_services.remove(&msg.tracker) {
            if !self.temporal_services.values().any(|&v| v == service_id) {
                let _ = self.cache.remove(&service_id);
                tracing::info!(
                    tracker.name = msg.tracker,
                    service.id = %service_id,
                    "Removed cache entry for temporal trackers"
                );
            }
        }
    }
}

// models

#[derive(Clone, Debug, Default)]
pub struct OnairProgram {
    pub current: Option<Arc<EpgProgram>>,
    pub next: Option<Arc<EpgProgram>>,
}

#[cfg(test)]
mod tests {}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    #[derive(Clone)]
    pub(crate) struct OnairProgramManagerStub;

    #[async_trait]
    impl Call<QueryOnairPrograms> for OnairProgramManagerStub {
        async fn call(
            &self,
            _msg: QueryOnairPrograms,
        ) -> actlet::Result<<QueryOnairPrograms as Message>::Reply> {
            // TODO
            Ok(Default::default())
        }
    }

    #[async_trait]
    impl Call<QueryOnairProgram> for OnairProgramManagerStub {
        async fn call(
            &self,
            msg: QueryOnairProgram,
        ) -> actlet::Result<<QueryOnairProgram as Message>::Reply> {
            match msg.service_id.sid().value() {
                0 => Ok(Err(Error::ServiceNotFound)),
                _ => Ok(Ok(OnairProgram {
                    current: None,
                    next: None,
                })),
            }
        }
    }

    #[async_trait]
    impl Call<RegisterEmitter> for OnairProgramManagerStub {
        async fn call(
            &self,
            _msg: RegisterEmitter,
        ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
            Ok(0)
        }
    }

    stub_impl_fire! {OnairProgramManagerStub, UnregisterEmitter}

    #[async_trait]
    impl Call<SpawnTemporalTracker> for OnairProgramManagerStub {
        async fn call(
            &self,
            _msg: SpawnTemporalTracker,
        ) -> actlet::Result<<SpawnTemporalTracker as Message>::Reply> {
            Ok(())
        }
    }
}
