mod clock_synchronizer;
mod eit_feeder;
mod job;
mod models;
mod service_scanner;

#[cfg(test)]
pub(crate) mod stub;

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

use actlet::prelude::*;
use chrono::DateTime;
use chrono::Duration;
use chrono::NaiveDate;
use chrono_jst::serde::duration_milliseconds_option;
use chrono_jst::serde::ts_milliseconds_option;
use chrono_jst::Jst;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;

use crate::config::ChannelConfig;
use crate::config::Config;
use crate::error::Error;
use crate::file_util;
use crate::models::*;
use crate::tuner::*;

use eit_feeder::EitFeeder;
use job::JobManager;

pub use clock_synchronizer::SyncClock;
pub use models::AudioComponentDescriptor;
pub use models::ComponentDescriptor;
pub use models::EitDescriptor;
pub use models::EitEvent;
pub use models::EitSection;
pub use models::EventGroupDescriptor;
pub use models::SeriesDescriptor;

pub struct Epg<T> {
    config: Arc<Config>,
    tuner_manager: T,

    services: Arc<IndexMap<ServiceId, EpgService>>, // keeps insertion order
    clocks: Arc<HashMap<ServiceId, Clock>>,
    // Allocate EpgSchedule in the heap in order to avoid stack overflow in
    // serialization using serde_json.
    schedules: HashMap<ServiceId, Box<EpgSchedule>>,

    service_updated: EmitterRegistry<ServicesUpdated>,
    clocks_updated: EmitterRegistry<ClocksUpdated>,
    programs_updated: EmitterRegistry<ProgramsUpdated>,
}

impl<T> Epg<T> {
    pub fn new(config: Arc<Config>, tuner_manager: T) -> Self {
        Epg {
            config,
            tuner_manager,
            services: Default::default(),
            clocks: Default::default(),
            schedules: Default::default(),
            service_updated: Default::default(),
            clocks_updated: Default::default(),
            programs_updated: Default::default(),
        }
    }

    async fn update_services(
        &mut self,
        results: Vec<(EpgChannel, Option<IndexMap<ServiceId, EpgService>>)>,
    ) {
        let mut services = IndexMap::new();

        for (channel, result) in results.into_iter() {
            match result {
                Some(new_services) => {
                    services.extend(new_services);
                }
                None => {
                    // Failed to scan services for some reason.  Reuse old
                    // services if properties of the channel hasn't changed.
                    for service in self.services.values() {
                        if service.channel == channel {
                            tracing::warn!(
                                channel.name,
                                %service.id,
                                "Reuse result of previous scan-services"
                            );
                            services.insert(service.id, service.clone());
                        }
                    }
                }
            }
        }

        self.services = Arc::new(services);

        let msg = ServicesUpdated {
            services: self.services.clone(),
        };
        self.service_updated.emit(msg).await;

        self.save_services();

        // Remove garbage.
        // clocks will be updated in update_clocks().
        self.schedules
            .retain(|service_id, _| self.services.contains_key(service_id));
    }

    async fn update_clocks(
        &mut self,
        results: Vec<(EpgChannel, Option<HashMap<ServiceId, Clock>>)>,
    ) {
        let mut clocks = HashMap::new();

        for (channel, result) in results.into_iter() {
            match result {
                Some(new_clocks) => clocks.extend(new_clocks),
                None => {
                    // Failed to synchronize clocks for some reason.  Reuse old
                    // clocks if exist.
                    //
                    // Assumed that services has been updated at least once
                    // after launch.
                    for (service_id, service) in self.services.iter() {
                        if service.channel == channel {
                            if let Some(clock) = self.clocks.get(service_id) {
                                clocks.insert(service_id.clone(), clock.clone());
                            }
                        }
                    }
                }
            }
        }

        self.clocks = Arc::new(clocks);

        let msg = ClocksUpdated {
            clocks: self.clocks.clone(),
        };
        self.clocks_updated.emit(msg).await;

        self.save_clocks();
    }

    fn prepare_schedule(&mut self, service_id: ServiceId, today: NaiveDate) {
        self.schedules
            .entry(service_id)
            .and_modify(|sched| sched.update_start_index(today))
            .or_insert(Box::new(EpgSchedule::new(service_id)));
    }

    fn update_schedule(&mut self, section: EitSection) {
        let service_id = section.service_id();
        self.schedules.entry(service_id).and_modify(move |sched| {
            sched.update(section);
        });
    }

    async fn flush_schedule(&mut self, service_id: ServiceId) {
        let num_programs = match self.schedules.get_mut(&service_id) {
            Some(schedule) => {
                schedule.collect_programs();
                schedule.programs.len()
            }
            None => 0,
        };

        let service = self.services.get(&service_id).expect("Service must exist");

        if num_programs > 0 {
            tracing::info!(%service.id, programs.len = num_programs, "Collected programs");
        }

        let msg = ProgramsUpdated { service_id };
        self.programs_updated.emit(msg).await;
    }

    // Must be called before other load functions.
    fn load_services(&mut self) -> Result<(), Error> {
        let channels: Vec<EpgChannel> = self
            .config
            .channels
            .iter()
            .cloned()
            .map(EpgChannel::from)
            .collect();

        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            let json_path = cache_dir.join("services.json");
            tracing::debug!(path = %json_path.display(), "Loading schedules...");
            let reader = BufReader::new(File::open(&json_path)?);
            let services: Vec<(ServiceId, EpgService)> = serde_json::from_reader(reader)?;
            // Drop a service if the channel of the service has been changed.
            let iter = services.into_iter().filter(|(_, service)| {
                let not_changed = channels.iter().any(|ch| ch == &service.channel);
                if !not_changed {
                    // if changed
                    tracing::debug!(
                        %service.id,
                        "Drop the service due to changes of the channel config"
                    );
                }
                not_changed
            });
            self.services = Arc::new(IndexMap::from_iter(iter));
            tracing::info!(services.len = self.services.len(), "Loaded services");
        }
        Ok(())
    }

    fn load_clocks(&mut self) -> Result<(), Error> {
        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            let json_path = cache_dir.join("clocks.json");
            tracing::debug!(path = %json_path.display(), "Loading clocks...");
            let reader = BufReader::new(File::open(&json_path)?);
            let clocks: Vec<(ServiceId, Clock)> = serde_json::from_reader(reader)?;
            // Drop a clock if the service ID of the clock is not contained in `self::services`.
            let iter = clocks.into_iter().filter(|(service_id, _)| {
                let contained = self.services.contains_key(service_id);
                if !contained {
                    tracing::debug!(
                        service.id = %service_id,
                        "Drop clock for missing service"
                    );
                }
                contained
            });
            self.clocks = Arc::new(HashMap::from_iter(iter));
            tracing::info!(clocks.len = self.clocks.len(), "Loaded clocks");
        }
        Ok(())
    }

    fn load_schedules(&mut self) -> Result<(), Error> {
        let today = Jst::today();
        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            let json_path = cache_dir.join("schedules.json");
            tracing::debug!(path = %json_path.display(), "Loading schedules...");
            let reader = BufReader::new(File::open(&json_path)?);
            let schedules: Vec<(ServiceId, Box<EpgSchedule>)> = serde_json::from_reader(reader)?;
            // Drop a clock if the service ID of the clock is not contained in `self::services`.
            let iter = schedules
                .into_iter()
                .filter(|(service_id, _)| {
                    let contained = self.services.contains_key(service_id);
                    if !contained {
                        tracing::debug!(
                            service.id = %service_id,
                            "Drop schedule for missing service"
                        );
                    }
                    contained
                })
                .map(|(service_id, mut sched)| {
                    sched.update_start_index(today);
                    (service_id, sched)
                });
            self.schedules = HashMap::from_iter(iter);
            tracing::info!(schedules.len = self.schedules.len(), "Loaded schedules");
        }
        Ok(())
    }

    fn save_services(&self) {
        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            // Serialize as a list of tuples in order to avoid failures in serialization to JSON.
            //
            // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
            // but we simply create `Vec<(&K, &V)>` in order to reduce maintenance cost.
            let services = self.services.iter().collect_vec();
            if file_util::save_json(&services, cache_dir.join("services.json")) {
                tracing::info!(services.len = self.services.len(), "Saved services");
            } else {
                tracing::error!("Failed to save services");
            }
        }
    }

    fn save_clocks(&self) {
        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            // Serialize as a list of tuples in order to avoid failures in serialization to JSON.
            //
            // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
            // but we simply create `Vec<(&K, &V)>` in order to reduce maintenance cost.
            let clocks = self.clocks.iter().collect_vec();
            if file_util::save_json(&clocks, cache_dir.join("clocks.json")) {
                tracing::info!(clocks.len = self.clocks.len(), "Saved clocks");
            } else {
                tracing::error!("Failed to save clocks");
            }
        }
    }

    fn save_schedules(&self) {
        if let Some(ref cache_dir) = self.config.epg.cache_dir {
            // Serialize as a list of tuples in order to avoid failures in serialization to JSON.
            //
            // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
            // but we simply create `Vec<(&K, &V)>` in order to reduce maintenance cost.
            let schedules = self.schedules.iter().collect_vec();
            if file_util::save_json(&schedules, cache_dir.join("schedules.json")) {
                tracing::info!(schedules.len = self.schedules.len(), "Saved schedules");
            } else {
                tracing::error!("Failed to save schedules");
            }
        }
    }

    fn collect_programs(&mut self) {
        for schedule in self.schedules.values_mut() {
            schedule.collect_programs();
        }
    }
}

#[async_trait]
impl<T> Actor for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        // It's guaranteed that no response is sent before cached EPG data is loaded.
        tracing::debug!("Started");
        if let Err(err) = self.load_services() {
            tracing::warn!(%err, "Failed to load services");
        }
        if let Err(err) = self.load_clocks() {
            tracing::warn!(%err, "Failed to load clocks");
        }
        if let Err(err) = self.load_schedules() {
            tracing::warn!(%err, "Failed to load schedules");
        }
        self.collect_programs();

        let eit_feeder = ctx
            .spawn_actor(EitFeeder::new(
                self.config.clone(),
                self.tuner_manager.clone(),
                ctx.address().clone(),
            ))
            .await;

        let _ = ctx
            .spawn_actor(JobManager::new(
                self.config.clone(),
                self.tuner_manager.clone(),
                ctx.address().clone(),
                eit_feeder,
            ))
            .await;
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// query channels

#[derive(Message)]
#[reply(Vec<MirakurunChannel>)]
pub struct QueryChannels;

#[async_trait]
impl<T> Handler<QueryChannels> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        _msg: QueryChannels,
        _ctx: &mut Context<Self>,
    ) -> <QueryChannels as Message>::Reply {
        tracing::debug!(msg.name = "QueryChannels");
        self.config
            .channels
            .iter()
            .map(|config| MirakurunChannel {
                channel_type: config.channel_type,
                channel: config.channel.clone(),
                name: config.name.clone(),
                services: self
                    .services
                    .values()
                    .filter(|sv| {
                        sv.channel.channel_type == config.channel_type
                            && sv.channel.channel == config.channel
                    })
                    .cloned()
                    .map(|sv| sv.into())
                    .collect(),
            })
            .collect()
    }
}

// query channel

#[derive(Message)]
#[reply(Result<EpgChannel, Error>)]
pub struct QueryChannel {
    pub channel_type: ChannelType,
    pub channel: String,
}

#[async_trait]
impl<T> Handler<QueryChannel> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: QueryChannel,
        _ctx: &mut Context<Self>,
    ) -> <QueryChannel as Message>::Reply {
        tracing::debug!(
            msg.name = "QueryChannel",
            %msg.channel_type,
            msg.channel,
        );
        self.config
            .channels
            .iter()
            .find(|config| config.channel_type == msg.channel_type && config.channel == msg.channel)
            .cloned()
            .map(EpgChannel::from)
            .ok_or(Error::ChannelNotFound)
    }
}

// query services

#[derive(Message)]
#[reply(Arc<IndexMap<ServiceId, EpgService>>)]
pub struct QueryServices;

#[async_trait]
impl<T> Handler<QueryServices> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        _msg: QueryServices,
        _ctx: &mut Context<Self>,
    ) -> <QueryServices as Message>::Reply {
        tracing::debug!(msg.name = "QueryServices");
        self.services.clone()
    }
}

// query service

#[derive(Message)]
#[reply(Result<EpgService, Error>)]
pub struct QueryService {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T> Handler<QueryService> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: QueryService,
        _ctx: &mut Context<Self>,
    ) -> <QueryService as Message>::Reply {
        tracing::debug!(msg.name = "QueryService", %msg.service_id);
        self.services
            .get(&msg.service_id)
            .cloned()
            .ok_or(Error::ServiceNotFound)
    }
}

// query clock

#[derive(Message)]
#[reply(Result<Clock, Error>)]
pub struct QueryClock {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T> Handler<QueryClock> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: QueryClock,
        _ctx: &mut Context<Self>,
    ) -> <QueryClock as Message>::Reply {
        tracing::debug!(msg.name = "QueryClock", %msg.service_id);
        self.clocks
            .get(&msg.service_id)
            .cloned()
            .ok_or(Error::ClockNotSynced)
    }
}

// query programs

#[derive(Message)]
#[reply(Arc<IndexMap<Eid, EpgProgram>>)]
pub struct QueryPrograms {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T> Handler<QueryPrograms> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: QueryPrograms,
        _ctx: &mut Context<Self>,
    ) -> <QueryPrograms as Message>::Reply {
        tracing::debug!(msg.name = "QueryPrograms", %msg.service_id);
        self.schedules
            .get(&msg.service_id)
            .map(|sched| sched.programs.clone())
            .unwrap_or_default()
    }
}

// query program

#[derive(Message)]
#[reply(Result<EpgProgram, Error>)]
pub struct QueryProgram {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T> Handler<QueryProgram> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: QueryProgram,
        _ctx: &mut Context<Self>,
    ) -> <QueryProgram as Message>::Reply {
        tracing::debug!(msg.name = "QueryProgram", %msg.program_id);
        let (service_id, eid) = msg.program_id.into();
        let schedule = self
            .schedules
            .get(&service_id)
            .ok_or(Error::ProgramNotFound)?;
        schedule
            .programs
            .get(&eid)
            .cloned()
            .ok_or(Error::ProgramNotFound)
    }
}

// update services

#[derive(Message)]
pub struct UpdateServices {
    pub results: Vec<(EpgChannel, Option<IndexMap<ServiceId, EpgService>>)>,
}

#[async_trait]
impl<T> Handler<UpdateServices> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: UpdateServices, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateServices");
        self.update_services(msg.results).await;
    }
}

// update clocks

#[derive(Message)]
pub struct UpdateClocks {
    pub results: Vec<(EpgChannel, Option<HashMap<ServiceId, Clock>>)>,
}

#[async_trait]
impl<T> Handler<UpdateClocks> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: UpdateClocks, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateClocks");
        self.update_clocks(msg.results).await;
    }
}

// prepare schedule

#[derive(Message)]
pub struct PrepareSchedule {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T> Handler<PrepareSchedule> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: PrepareSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "PrepareSchedule", %msg.service_id);
        self.prepare_schedule(msg.service_id, Jst::today());
    }
}

// update schedule

#[derive(Message)]
pub struct UpdateSchedule {
    pub section: EitSection,
}

#[async_trait]
impl<T> Handler<UpdateSchedule> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: UpdateSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(
            msg.name = "UpdateSchedule",
            msg.service_id = %msg.section.service_id(),
            msg.section.table_id,
            msg.section.section_number,
            msg.section.last_section_number,
            msg.section.segment_last_section_number,
            msg.section.version_number,
        );
        self.update_schedule(msg.section);
    }
}

// flush schedule

#[derive(Message)]
pub struct FlushSchedule {
    pub service_id: ServiceId,
}

#[async_trait]
impl<T> Handler<FlushSchedule> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: FlushSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "FlushSchedule", %msg.service_id);
        self.flush_schedule(msg.service_id).await;
    }
}

// save schedules

#[derive(Message)]
pub struct SaveSchedules;

#[async_trait]
impl<T> Handler<SaveSchedules> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, _msg: SaveSchedules, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "SaveSchedules");
        self.save_schedules();
    }
}

// register emitter

#[derive(Message)]
#[reply(usize)]
pub enum RegisterEmitter {
    ServicesUpdated(Emitter<ServicesUpdated>),
    ClocksUpdated(Emitter<ClocksUpdated>),
    ProgramsUpdated(Emitter<ProgramsUpdated>),
}

#[async_trait]
impl<T> Handler<RegisterEmitter> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        match msg {
            RegisterEmitter::ServicesUpdated(emitter) => {
                let msg = ServicesUpdated {
                    services: self.services.clone(),
                };
                emitter.emit(msg).await;
                let id = self.service_updated.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::ServicesUpdated", id);
                id
            }
            RegisterEmitter::ClocksUpdated(emitter) => {
                let msg = ClocksUpdated {
                    clocks: self.clocks.clone(),
                };
                emitter.emit(msg).await;
                let id = self.clocks_updated.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::ClocksUpdated", id);
                id
            }
            RegisterEmitter::ProgramsUpdated(emitter) => {
                // Create a task to send messages.
                //
                // Sending many messages in the message handler may cause a dead lock
                // when the number of messages to be sent is larger than the capacity
                // of the emitter's channel.  See the issue #705 for example.
                let task = {
                    let service_ids = self.schedules.keys().cloned().collect_vec();
                    let emitter = emitter.clone();
                    async move {
                        for service_id in service_ids.into_iter() {
                            emitter.emit(ProgramsUpdated { service_id }).await;
                        }
                    }
                };
                ctx.spawn_task(task);
                let id = self.programs_updated.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::ProgramsUpdated", id);
                id
            }
        }
    }
}

// unregister emitter

#[derive(Message)]
pub enum UnregisterEmitter {
    ServicesUpdated(usize),
    ClocksUpdated(usize),
    ProgramsUpdated(usize),
}

#[async_trait]
impl<T> Handler<UnregisterEmitter> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
{
    async fn handle(&mut self, msg: UnregisterEmitter, _ctx: &mut Context<Self>) {
        match msg {
            UnregisterEmitter::ServicesUpdated(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::ServicesUpdated", id);
                self.service_updated.unregister(id);
            }
            UnregisterEmitter::ClocksUpdated(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::ClocksUpdated", id);
                self.clocks_updated.unregister(id);
            }
            UnregisterEmitter::ProgramsUpdated(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::ProgramsUpdated", id);
                self.programs_updated.unregister(id);
            }
        }
    }
}

// notifications

#[derive(Clone, Message)]
pub struct ServicesUpdated {
    pub services: Arc<IndexMap<ServiceId, EpgService>>,
}

#[derive(Clone, Message)]
pub struct ClocksUpdated {
    pub clocks: Arc<HashMap<ServiceId, Clock>>,
}

#[derive(Clone, Message)]
pub struct ProgramsUpdated {
    pub service_id: ServiceId,
}

// EpgSchedule holds sections of H-EIT[schedule basic] and H-EIT[schedule extended]
// for a particular service.  See ARIB TR-B14 for details.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSchedule {
    service_id: ServiceId,
    // EpgScheduleUnits are stored in chronological order.
    units: [EpgScheduleUnit; Self::MAX_DAYS],
    #[serde(skip)]
    start_index: usize, // used for implementing a ring buffer on `units`.
    #[serde(skip)]
    programs: Arc<IndexMap<Eid, EpgProgram>>,
}

impl EpgSchedule {
    const MAX_DAYS: usize = 9;

    fn new(service_id: ServiceId) -> Self {
        EpgSchedule {
            service_id,
            units: Default::default(),
            start_index: 0,
            programs: Default::default(),
        }
    }

    fn update_start_index(&mut self, today: NaiveDate) {
        let result = self
            .units
            .iter()
            .map(|unit| unit.date())
            .enumerate()
            .filter_map(|(index, date)| date.map(|date| (index, date)))
            .min_by_key(|(_, date)| *date);

        match result {
            Some((index, date)) => {
                let delta = (today - date).num_days();
                let delta = delta % (Self::MAX_DAYS as i64);
                // `delta` may be a negative value.
                let delta = if delta >= 0 {
                    delta as usize
                } else {
                    (delta + (Self::MAX_DAYS as i64)) as usize
                };
                self.start_index = (index + delta) % Self::MAX_DAYS;
                debug_assert!((0..Self::MAX_DAYS).contains(&self.start_index));
            }
            None => self.start_index = 0,
        }
    }

    fn update(&mut self, section: EitSection) {
        let i = (self.start_index + section.unit_index()) % Self::MAX_DAYS;
        self.units[i].update(section);
    }

    fn collect_programs(&mut self) {
        let service_id = self.service_id;
        // Start from the previous day in order to collect programs in chronological order.
        let start_index = if self.start_index > 0 {
            self.start_index - 1
        } else {
            Self::MAX_DAYS - 1
        };
        // Pre-allocate the buffer with the previous size in order to reduce the number of re-allocations.
        let mut programs = IndexMap::with_capacity(self.programs.len());
        for n in 0..Self::MAX_DAYS {
            let i = (start_index + n) % Self::MAX_DAYS;
            self.units[i].collect_programs(service_id, &mut programs);
        }
        programs.shrink_to_fit();
        self.programs = Arc::new(programs);
    }
}

// This type holds TV program information for 1 day.
#[derive(Default, Deserialize, Serialize)]
struct EpgScheduleUnit {
    // Segments are stored in chronological order starting from the midnight.
    segments: [EpgSegment; Self::NUM_SEGMENTS],
}

impl EpgScheduleUnit {
    // 8 segments are used for 1 day.
    const NUM_SEGMENTS: usize = 8;

    fn date(&self) -> Option<NaiveDate> {
        self.segments.iter().find_map(|segment| segment.date())
    }

    fn update(&mut self, section: EitSection) {
        let i = section.segment_index_in_unit();
        self.segments[i].update(section);
    }

    fn collect_programs(&self, service_id: ServiceId, programs: &mut IndexMap<Eid, EpgProgram>) {
        for segment in self.segments.iter() {
            segment.collect_programs(service_id, programs)
        }
    }
}

// This type holds TV program information for 3 hours.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSegment {
    // Sections of EIT[schedule basic].
    // table_id in 0x50..=0x57.
    basic_sections: [Option<EpgSection>; Self::NUM_SECTIONS],
    // Sections of EIT[schedule extended].
    // table_id in 0x58..=0x5F.
    extended_sections: [Option<EpgSection>; Self::NUM_SECTIONS],
}

impl EpgSegment {
    const NUM_SECTIONS: usize = 8;

    fn date(&self) -> Option<NaiveDate> {
        self.basic_sections
            .iter()
            .filter_map(|section| section.as_ref())
            .find_map(|section| section.date())
    }

    fn update(&mut self, section: EitSection) {
        let sections = if section.is_basic() {
            &mut self.basic_sections
        } else {
            &mut self.extended_sections
        };

        let n = section.last_section_index() + 1;
        for i in n..Self::NUM_SECTIONS {
            sections[i] = None;
        }

        let i = section.section_index();
        sections[i] = Some(EpgSection::from(section));
    }

    fn collect_programs(&self, service_id: ServiceId, programs: &mut IndexMap<Eid, EpgProgram>) {
        let sections = self
            .extended_sections
            .iter()
            .chain(self.basic_sections.iter());
        for section in sections {
            if let Some(section) = section {
                section.collect_programs(service_id, programs)
            }
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSection {
    version: u8,
    // Events are stored in chronological order.
    events: Vec<EitEvent>,
}

impl EpgSection {
    fn date(&self) -> Option<NaiveDate> {
        self.events
            .first()
            .map(|event| event.start_time().unwrap().date_naive())
    }

    fn collect_programs(&self, service_id: ServiceId, programs: &mut IndexMap<Eid, EpgProgram>) {
        let events = self
            .events
            .iter()
            // Remove programs with undefined start time.
            .filter(|event| event.start_time.is_some())
            // Remove programs with undefined duration.
            // TODO: Undefined duration in EIT[schedule] might be allowed?
            .filter(|event| event.duration.is_some());
        for event in events {
            let program_id = ProgramId::from((service_id, Eid::from(event.event_id)));
            programs
                .entry(event.event_id)
                .or_insert(EpgProgram::new(program_id))
                .update(event);
        }
    }
}

impl From<EitSection> for EpgSection {
    fn from(section: EitSection) -> Self {
        EpgSection {
            version: section.version_number,
            events: section.events,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct EpgChannel {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    #[serde(default)]
    pub extra_args: String,
    pub services: Vec<Sid>,
    pub excluded_services: Vec<Sid>,
}

impl fmt::Display for EpgChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.channel_type, self.channel)
    }
}

impl From<ChannelConfig> for EpgChannel {
    fn from(config: ChannelConfig) -> Self {
        EpgChannel {
            name: config.name,
            channel_type: config.channel_type,
            channel: config.channel,
            extra_args: config.extra_args,
            services: config.services,
            excluded_services: config.excluded_services,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgService {
    pub id: ServiceId,
    #[serde(rename = "type")]
    pub service_type: u16,
    #[serde(default)]
    pub logo_id: i16,
    #[serde(default)]
    pub remote_control_key_id: u16,
    pub name: String,
    pub channel: EpgChannel,
}

impl EpgService {
    pub fn nid(&self) -> Nid {
        self.id.nid()
    }

    pub fn sid(&self) -> Sid {
        self.id.sid()
    }
}

impl Into<MirakurunChannelService> for EpgService {
    fn into(self) -> MirakurunChannelService {
        MirakurunChannelService {
            id: self.id,
            service_id: self.sid(),
            network_id: self.nid(),
            name: self.name,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EpgProgram {
    pub id: ProgramId,
    #[serde(with = "ts_milliseconds_option")]
    pub start_at: Option<DateTime<Jst>>,
    #[serde(with = "duration_milliseconds_option")]
    pub duration: Option<Duration>,
    pub scrambled: bool,
    pub name: Option<String>,
    pub description: Option<String>,
    pub extended: Option<IndexMap<String, String>>,
    pub video: Option<ComponentDescriptor>,
    pub audios: HashMap<u8, AudioComponentDescriptor>, // component_tag -> value
    pub genres: Option<Vec<EpgGenre>>,
    pub series: Option<SeriesDescriptor>,
    pub event_group: Option<EventGroupDescriptor>,
}

impl EpgProgram {
    pub fn new(id: ProgramId) -> Self {
        Self {
            id,
            start_at: None,
            duration: None,
            scrambled: false,
            name: None,
            description: None,
            extended: None,
            video: None,
            audios: HashMap::new(), // assumed that component_tag is unique in the same event
            genres: None,
            series: None,
            event_group: None,
        }
    }

    pub fn eid(&self) -> Eid {
        self.id.eid()
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("NO TITLE")
    }

    pub fn end_at(&self) -> Option<DateTime<Jst>> {
        self.start_at
            .zip(self.duration)
            .map(|(start_at, duration)| start_at + duration)
    }

    pub fn update(&mut self, event: &EitEvent) {
        debug_assert!(event.start_time().is_some());
        debug_assert!(event.duration().is_some());
        self.start_at = event.start_time();
        self.duration = event.duration();
        self.scrambled = event.scrambled;
        for desc in event.descriptors.iter() {
            match desc {
                EitDescriptor::ShortEvent { event_name, text } => {
                    self.name = event_name.clone();
                    self.description = text.clone();
                }
                EitDescriptor::Component(value) => {
                    self.video = Some(value.clone());
                }
                EitDescriptor::AudioComponent(value) => {
                    self.audios
                        .entry(value.component_tag)
                        .and_modify(|entry| *entry = value.clone())
                        .or_insert_with(|| value.clone());
                }
                EitDescriptor::Content { nibbles } => {
                    self.genres = Some(
                        nibbles
                            .iter()
                            .map(|nibble| EpgGenre::new(*nibble))
                            .collect(),
                    );
                }
                EitDescriptor::Series(value) => {
                    self.series = Some(value.clone());
                }
                EitDescriptor::EventGroup(value) => {
                    self.event_group = Some(value.clone());
                }
                EitDescriptor::ExtendedEvent { items } => {
                    let mut map = IndexMap::new();
                    map.extend(items.clone());
                    self.extended = Some(map);
                }
            }
        }
    }
}

impl EitSection {
    fn unit_index(&self) -> usize {
        // A single table holds TV program information for 4 days.
        (self.table_id as usize % 8) * 4 + self.segment_index() / EpgScheduleUnit::NUM_SEGMENTS
    }

    fn segment_index_in_unit(&self) -> usize {
        self.segment_index() % EpgScheduleUnit::NUM_SEGMENTS
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use chrono::Datelike;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_update_services() {
        let mut epg = Epg::new(Arc::new(Default::default()), TunerManagerStub);

        let ch1 = channel!("ch1", ChannelType::GR, "1");
        let ch2 = channel!("ch2", ChannelType::GR, "2");

        let id1_1 = ServiceId::from((1, 1));
        let id1_2 = ServiceId::from((1, 2));
        let id2_3 = ServiceId::from((2, 3));
        let id2_4 = ServiceId::from((2, 4));

        // initial update

        let results = vec![
            (
                ch1.clone(),
                Some(indexmap::indexmap! {
                    id1_1 => service!(id1_1, "sv1", ch1.clone()),
                    id1_2 => service!(id1_2, "sv2", ch1.clone()),
                }),
            ),
            (
                ch2.clone(),
                Some(indexmap::indexmap! {
                    id2_3 => service!(id2_3, "sv3", ch2.clone()),
                    id2_4 => service!(id2_4, "sv4", ch2.clone()),
                }),
            ),
        ];

        epg.update_services(results).await;
        {
            let iter = epg.services.values().map(|sv| &sv.name);
            assert!(iter.eq(["sv1", "sv2", "sv3", "sv4"].iter()));
        }

        // update with a complete list

        let results = vec![
            (
                ch1.clone(),
                Some(indexmap::indexmap! {
                    id1_1 => service!(id1_1, "sv1", ch1.clone()),
                    id1_2 => service!(id1_2, "sv2", ch1.clone()),
                }),
            ),
            (
                ch2.clone(),
                Some(indexmap::indexmap! {
                    id2_3 => service!(id2_3, "sv3", ch2.clone()),
                    id2_4 => service!(id2_4, "sv4", ch2.clone()),
                }),
            ),
        ];

        epg.update_services(results).await;
        {
            let iter = epg.services.values().map(|sv| &sv.name);
            assert!(iter.eq(["sv1", "sv2", "sv3", "sv4"].iter()));
        }

        // update with an incomplete list

        let results = vec![
            (ch1.clone(), None),
            (
                ch2.clone(),
                Some(indexmap::indexmap! {
                    id2_3 => service!(id2_3, "sv3", ch2.clone()),
                    id2_4 => service!(id2_4, "sv4", ch2.clone()),
                }),
            ),
        ];

        epg.update_services(results).await;
        {
            let iter = epg.services.values().map(|sv| &sv.name);
            assert!(iter.eq(["sv1", "sv2", "sv3", "sv4"].iter()));
        }

        // update with a new service

        let results = vec![
            (
                ch1.clone(),
                Some(indexmap::indexmap! {
                    id1_1 => service!(id1_1, "sv1.new", ch1.clone()),
                }),
            ),
            (ch2.clone(), None),
        ];

        epg.update_services(results).await;
        {
            let iter = epg.services.values().map(|sv| &sv.name);
            assert!(iter.eq(["sv1.new", "sv3", "sv4"].iter()));
        }
    }

    #[test(tokio::test)]
    async fn test_update_services_purge_garbage_schedules() {
        let mut epg = Epg::new(Arc::new(Default::default()), TunerManagerStub);
        let id = ServiceId::from((1, 2));
        epg.schedules.insert(id, Box::new(EpgSchedule::new(id)));
        assert!(!epg.schedules.is_empty());
        let id = ServiceId::from((1, 1));
        let sv = create_epg_service(id, ChannelType::GR);
        let ch = sv.channel.clone();
        epg.update_services(vec![(
            ch,
            Some(indexmap::indexmap! {
                id => sv,
            }),
        )])
        .await;
        assert!(epg.schedules.is_empty());
    }

    #[test]
    fn test_epg_schedule_update_start_index() {
        let id = ServiceId::from((1, 2));
        let mut sched = EpgSchedule::new(id);
        assert_eq!(sched.start_index, 0);
        assert!(sched.units.iter().all(|unit| unit.date().is_none()));

        sched.update_start_index(Jst::today());
        assert_eq!(sched.start_index, 0);

        let mut date = Jst::today();
        for i in 0..EpgSchedule::MAX_DAYS {
            sched.units[i].segments[0].basic_sections[0] = Some(create_epg_section(date));
            assert_matches!(sched.units[i].date(), Some(v) => assert_eq!(v, date));
            date = date.succ_opt().unwrap();
        }

        sched.update_start_index(Jst::today());
        assert_eq!(sched.start_index, 0);

        sched.update_start_index(Jst::today().succ_opt().unwrap());
        assert_eq!(sched.start_index, 1);

        sched.update_start_index(Jst::today().pred_opt().unwrap());
        assert_eq!(sched.start_index, 8);

        sched.update_start_index(date); // today + 9 days
        assert_eq!(sched.start_index, 0);
    }

    #[test]
    fn test_epg_schedule_update() {
        let id = ServiceId::from((1, 2));

        let mut sched = EpgSchedule::new(id);
        sched.update(EitSection {
            original_network_id: id.nid(),
            transport_stream_id: 0.into(),
            service_id: id.sid(),
            table_id: 0x50, // unit-index(0)
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(sched.units[0].segments[0].basic_sections[0].is_some());

        let mut sched = EpgSchedule::new(id);
        sched.update(EitSection {
            original_network_id: id.nid(),
            transport_stream_id: 0.into(),
            service_id: id.sid(),
            table_id: 0x51, // unit-index(4)
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(sched.units[4].segments[0].basic_sections[0].is_some());

        let mut sched = EpgSchedule::new(id);
        sched.start_index = 5;
        sched.update(EitSection {
            original_network_id: id.nid(),
            transport_stream_id: 0.into(),
            service_id: id.sid(),
            table_id: 0x51, // unit-index(4)
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        // The unit-index of the section must wrap around to 0.
        assert!(sched.units[0].segments[0].basic_sections[0].is_some());
    }

    #[test]
    fn test_epg_segment_update() {
        let mut segment: EpgSegment = Default::default();

        segment.update(EitSection {
            original_network_id: 1.into(),
            transport_stream_id: 0.into(),
            service_id: 2.into(),
            table_id: 0x50,
            section_number: 0x01,
            last_section_number: 0xF8,
            segment_last_section_number: 0x01,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(segment.basic_sections[0].is_none());
        assert!(segment.basic_sections[1].is_some());

        segment.update(EitSection {
            original_network_id: 1.into(),
            transport_stream_id: 0.into(),
            service_id: 2.into(),
            table_id: 0x50,
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(segment.basic_sections[0].is_some());
        assert!(segment.basic_sections[1].is_none());
    }

    fn create_epg_service(id: ServiceId, channel_type: ChannelType) -> EpgService {
        service!(id, "Service", channel!("Ch", channel_type, "ch"))
    }

    fn create_epg_section(date: NaiveDate) -> EpgSection {
        let start_time = date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Jst)
            .unwrap()
            .timestamp_millis();
        EpgSection {
            version: 1,
            events: vec![EitEvent {
                event_id: (date.day() as u16).into(),
                start_time: Some(start_time),
                duration: Some(30 * 60000), // 30min
                scrambled: false,
                descriptors: Vec::new(),
            }],
        }
    }

    #[derive(Clone)]
    struct TunerManagerStub;
}
// </coverage:exclude>
