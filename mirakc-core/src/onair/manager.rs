use std::collections::HashMap;
use std::sync::Arc;

use actlet::prelude::*;

use crate::config::Config;
use crate::config::LocalOnairProgramTrackerConfig;
use crate::config::OnairProgramTrackerConfig;
use crate::epg;
use crate::epg::EpgProgram;
use crate::models::ServiceTriple;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;

use super::local::LocalTracker;
use super::OnairProgramChanged;

pub struct OnairProgramManager<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    cache: HashMap<ServiceTriple, OnairProgram>,
    trackers: HashMap<String, Tracker<T, E>>,
    emitters: Vec<Emitter<OnairProgramChanged>>,
}

enum Tracker<T, E> {
    Local(Address<LocalTracker<T, E>>),
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
            emitters: Default::default(),
        }
    }
}

// actor

#[async_trait]
impl<T, E> Actor for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        for (name, config) in self.config.onair_program_trackers.iter() {
            let emitter = ctx.address().clone().into();
            let tracker = match config {
                OnairProgramTrackerConfig::Local(config) => {
                    self.spawn_local_tracker(name, config, ctx, emitter).await
                }
            };
            self.trackers.insert(name.to_string(), tracker);
        }
        self.epg
            .call(epg::RegisterEmitter::ServicesUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for epg::ServicesUpdated");
        tracing::debug!("Started");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

impl<T, E> OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryServices>,
{
    async fn spawn_local_tracker<C: Spawn>(
        &self,
        name: &str,
        config: &Arc<LocalOnairProgramTrackerConfig>,
        ctx: &C,
        emitter: Emitter<OnairProgramChanged>,
    ) -> Tracker<T, E> {
        let tracker = ctx
            .spawn_actor(LocalTracker::new(
                name.to_string(),
                config.clone(),
                self.tuner_manager.clone(),
                self.epg.clone(),
                emitter,
            ))
            .await;
        tracing::info!(tracker.kind = "local", tracker.name = name, "Spawned",);
        Tracker::Local(tracker)
    }
}

// query on-air program

#[derive(Message)]
#[reply("Option<OnairProgram>")]
pub struct QueryOnairProgram {
    pub service_triple: ServiceTriple,
}

#[async_trait]
impl<T, E> Handler<QueryOnairProgram> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryOnairProgram,
        _ctx: &mut Context<Self>,
    ) -> <QueryOnairProgram as Message>::Reply {
        tracing::debug!(msg.name = "QueryOnairProgram", %msg.service_triple);
        self.cache.get(&msg.service_triple).cloned()
    }
}

// register emitter

#[derive(Message)]
#[reply("()")]
pub struct RegisterEmitter(pub Emitter<OnairProgramChanged>);

#[async_trait]
impl<T, E> Handler<RegisterEmitter> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RegisterEmitter, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RegisterEmitter");
        self.emitters.push(msg.0);
    }
}

// services updated

#[async_trait]
impl<T, E> Handler<epg::ServicesUpdated> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryServices>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ServicesUpdated, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        self.cache
            .retain(|service_triple, _| msg.services.contains_key(service_triple));
    }
}

// on-air program changed

#[async_trait]
impl<T, E> Handler<OnairProgramChanged> for OnairProgramManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
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
        for emitter in self.emitters.iter() {
            emitter.emit(msg.clone()).await;
        }
    }
}

impl<T, E> OnairProgramManager<T, E> {
    async fn update_current_program(&mut self, program: Arc<EpgProgram>) {
        let program_quad = program.quad;
        let service_triple = program_quad.into();
        self.cache.entry(service_triple).or_default().current = Some(program);
        tracing::info!(%service_triple, current.program.quad = %program_quad);
    }

    async fn update_next_program(&mut self, program: Arc<EpgProgram>) {
        let program_quad = program.quad;
        let service_triple = program_quad.into();
        self.cache.entry(service_triple).or_default().next = Some(program);
        tracing::info!(%service_triple, next.program.quad = %program_quad);
    }
}

// models

#[derive(Clone, Debug, Default)]
pub struct OnairProgram {
    pub current: Option<Arc<EpgProgram>>,
    pub next: Option<Arc<EpgProgram>>,
}

// <coverage:exclude>
#[cfg(test)]
mod tests {}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    #[derive(Clone)]
    pub(crate) struct OnairProgramManagerStub;

    #[async_trait]
    impl Call<QueryOnairProgram> for OnairProgramManagerStub {
        async fn call(
            &self,
            _msg: QueryOnairProgram,
        ) -> actlet::Result<<QueryOnairProgram as Message>::Reply> {
            // TODO
            Ok(None)
        }
    }

    #[async_trait]
    impl Call<RegisterEmitter> for OnairProgramManagerStub {
        async fn call(
            &self,
            _msg: RegisterEmitter,
        ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
            Ok(())
        }
    }
}
// </coverage:exclude>
