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
use std::io::BufWriter;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use chrono::DateTime;
use chrono::Duration;
use chrono::NaiveDate;
use chrono::TimeZone;
use chrono_jst::Jst;
use chrono_jst::serde::duration_milliseconds;
use chrono_jst::serde::ts_milliseconds;
use indexmap::IndexMap;
use itertools::Itertools;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde::Serialize;

use crate::config::ChannelConfig;
use crate::config::Config;
use crate::error::Error;
use crate::models::*;
use crate::tuner::*;

use eit_feeder::EitFeeder;
use job::JobManager;

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
    services_emitters: Vec<Emitter<ServicesUpdated>>,
    clocks_emitters: Vec<Emitter<ClocksUpdated>>,
    programs_emitters: Vec<Emitter<ProgramsUpdated>>,
    services: Arc<IndexMap<ServiceTriple, EpgService>>, // keeps insertion order
    clocks: Arc<HashMap<ServiceTriple, Clock>>,
    // Allocate EpgSchedule in the heap in order to avoid stack overflow in
    // serialization using serde_json.
    schedules: HashMap<ServiceTriple, Box<EpgSchedule>>,
    airtimes: HashMap<ProgramQuad, Airtime>,
}

pub struct Airtime {
    pub start_time: DateTime<Jst>,
    pub duration: Duration,
}

impl<T> Epg<T> {
    pub fn new(config: Arc<Config>, tuner_manager: T) -> Self {
        Epg {
            config,
            tuner_manager,
            services_emitters: Default::default(),
            clocks_emitters: Default::default(),
            programs_emitters: Default::default(),
            services: Default::default(),
            clocks: Default::default(),
            schedules: Default::default(),
            airtimes: Default::default(),
        }
    }

    async fn update_services(
        &mut self,
        results: Vec<(EpgChannel, Option<IndexMap<ServiceTriple, EpgService>>)>,
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
                            services.insert(service.triple(), service.clone());
                        }
                    }
                }
            }
        }

        self.services = Arc::new(services);

        for emitter in self.services_emitters.iter() {
            emitter
                .emit(ServicesUpdated {
                    services: self.services.clone(),
                })
                .await;
        }

        match self.save_services() {
            Ok(_) => (),
            Err(err) => tracing::error!("Failed to save services: {}", err),
        }

        // Remove garbage.
        // clocks will be updated in update_clocks().
        self.schedules
            .retain(|triple, _| self.services.contains_key(triple));
    }

    async fn update_clocks(
        &mut self,
        results: Vec<(EpgChannel, Option<HashMap<ServiceTriple, Clock>>)>,
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
                    for (triple, service) in self.services.iter() {
                        if service.channel == channel {
                            if let Some(clock) = self.clocks.get(triple) {
                                clocks.insert(triple.clone(), clock.clone());
                            }
                        }
                    }
                }
            }
        }

        self.clocks = Arc::new(clocks);

        for emitter in self.clocks_emitters.iter() {
            emitter
                .emit(ClocksUpdated {
                    clocks: self.clocks.clone(),
                })
                .await;
        }

        match self.save_clocks() {
            Ok(_) => (),
            Err(err) => tracing::error!("Failed to save clocks: {}", err),
        }
    }

    fn prepare_schedule(&mut self, service_triple: ServiceTriple, today: NaiveDate) {
        self.schedules
            .entry(service_triple)
            .and_modify(|sched| sched.update_start_index(today))
            .or_insert(Box::new(EpgSchedule::new(service_triple)));
    }

    fn update_schedule(&mut self, section: EitSection) {
        let triple = section.service_triple();
        self.schedules.entry(triple).and_modify(move |sched| {
            sched.update(section);
        });
    }

    async fn flush_schedule(&mut self, service_triple: ServiceTriple) {
        let num_programs = match self.schedules.get_mut(&service_triple) {
            Some(schedule) => {
                schedule.collect_programs();
                schedule.programs.len()
            }
            None => 0,
        };

        let service = self
            .services
            .get(&service_triple)
            .expect("Service must exist");

        if num_programs > 0 {
            tracing::info!(
                "Collected {} programs of {} ({})",
                num_programs,
                service.name,
                service_triple,
            );
        }

        for emitter in self.programs_emitters.iter() {
            emitter.emit(ProgramsUpdated { service_triple }).await;
        }
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

        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("services.json");
                tracing::debug!("Loading schedules from {}...", json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                let services: Vec<(ServiceTriple, EpgService)> = serde_json::from_reader(reader)?;
                // Drop a service if the channel of the service has been
                // changed.
                let iter = services.into_iter().filter(|(_, sv)| {
                    let not_changed = channels.iter().any(|ch| ch == &sv.channel);
                    if !not_changed {
                        // if changed
                        tracing::debug!(
                            "Drop service#{} ({}) due to changes of the channel config",
                            sv.triple(),
                            sv.name
                        );
                    }
                    not_changed
                });
                self.services = Arc::new(IndexMap::from_iter(iter));
                tracing::info!("Loaded {} services", self.services.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to load services.json");
            }
        }
        Ok(())
    }

    fn load_clocks(&mut self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("clocks.json");
                tracing::debug!("Loading clocks from {}...", json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                let clocks: Vec<(ServiceTriple, Clock)> = serde_json::from_reader(reader)?;
                // Drop a clock if the service triple of the clock is not
                // contained in `self::services`.
                let iter = clocks.into_iter().filter(|(triple, _)| {
                    let contained = self.services.contains_key(triple);
                    if !contained {
                        tracing::debug!("Drop clock for missing service#{}", triple);
                    }
                    contained
                });
                self.clocks = Arc::new(HashMap::from_iter(iter));
                tracing::info!("Loaded {} clocks", self.clocks.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to load clocks.json");
            }
        }
        Ok(())
    }

    fn load_schedules(&mut self) -> Result<(), Error> {
        let today = Jst::today();
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("schedules.json");
                tracing::debug!("Loading schedules from {}...", json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                let schedules: Vec<(ServiceTriple, Box<EpgSchedule>)> =
                    serde_json::from_reader(reader)?;
                // Drop a clock if the service triple of the clock is not
                // contained in `self::services`.
                let iter = schedules
                    .into_iter()
                    .filter(|(triple, _)| {
                        let contained = self.services.contains_key(triple);
                        if !contained {
                            tracing::debug!("Drop schedule for missing service#{}", triple);
                        }
                        contained
                    })
                    .map(|(triple, mut sched)| {
                        sched.update_start_index(today);
                        (triple, sched)
                    });
                self.schedules = HashMap::from_iter(iter);
                tracing::info!("Loaded schedules for {} services", self.schedules.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to load schedules.json");
            }
        }
        Ok(())
    }

    fn save_services(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("services.json");
                tracing::debug!("Saving services into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                // Serialize as a list of tuples in order to avoid failures in
                // serialization to JSON.
                //
                // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
                // but we simply create `Vec<(&K, &V)>` in order to reduce
                // maintenance cost.
                let services = self.services.iter().collect_vec();
                serde_json::to_writer(writer, &services)?;
                tracing::info!("Saved {} services", self.services.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to save services");
            }
        }
        Ok(())
    }

    fn save_clocks(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("clocks.json");
                tracing::debug!("Saving clocks into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                // Serialize as a list of tuples in order to avoid failures in
                // serialization to JSON.
                //
                // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
                // but we simply create `Vec<(&K, &V)>` in order to reduce
                // maintenance cost.
                let clocks = self.clocks.iter().collect_vec();
                serde_json::to_writer(writer, &clocks)?;
                tracing::info!("Saved {} clocks", self.clocks.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to save clocks");
            }
        }
        Ok(())
    }

    fn save_schedules(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("schedules.json");
                tracing::debug!("Saving schedules into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                // Serialize as a list of tuples in order to avoid failures in
                // serialization to JSON.
                //
                // We can implement `Serialize` for `Wrapper(Iterator<Item = (&K, &V)>)`,
                // but we simply create `Vec<(&K, &V)>` in order to reduce
                // maintenance cost.
                let schedules = self.schedules.iter().collect_vec();
                serde_json::to_writer(writer, &schedules)?;
                tracing::info!("Saved schedules for {} services", self.schedules.len());
            }
            None => {
                tracing::warn!("No epg.cache-dir specified, skip to save schedules");
            }
        }
        Ok(())
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
    T: Into<Emitter<StopStreaming>>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        // It's guaranteed that no response is sent before cached EPG data is loaded.
        tracing::debug!("Started");
        if let Err(err) = self.load_services() {
            tracing::warn!("Failed to load services: {}", err);
        }
        if let Err(err) = self.load_clocks() {
            tracing::warn!("Failed to load clocks: {}", err);
        }
        if let Err(err) = self.load_schedules() {
            tracing::warn!("Failed to load schedules: {}", err);
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
#[reply("Vec<MirakurunChannel>")]
pub struct QueryChannels;

#[async_trait]
impl<T> Handler<QueryChannels> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
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
#[reply("Result<EpgChannel, Error>")]
pub struct QueryChannel {
    pub channel_type: ChannelType,
    pub channel: String,
}

#[async_trait]
impl<T> Handler<QueryChannel> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
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
#[reply("Arc<IndexMap<ServiceTriple, EpgService>>")]
pub struct QueryServices;

#[async_trait]
impl<T> Handler<QueryServices> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
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
#[reply("Result<EpgService, Error>")]
pub enum QueryService {
    ByMirakurunServiceId(MirakurunServiceId), // For Mirakurun-compatible Web API
    ByServiceTriple(ServiceTriple),
}

#[async_trait]
impl<T> Handler<QueryService> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: QueryService,
        _ctx: &mut Context<Self>,
    ) -> <QueryService as Message>::Reply {
        tracing::debug!(msg.name = "QueryService");
        let (nid, sid) = match msg {
            QueryService::ByMirakurunServiceId(id) => id.into(),
            QueryService::ByServiceTriple(triple) => triple.into(),
        };
        self.services
            .values()
            .find(|sv| sv.nid == nid && sv.sid == sid)
            .cloned()
            .ok_or(Error::ServiceNotFound)
    }
}

// query clock

#[derive(Message)]
#[reply("Result<Clock, Error>")]
pub struct QueryClock {
    pub triple: ServiceTriple,
}

#[async_trait]
impl<T> Handler<QueryClock> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: QueryClock,
        _ctx: &mut Context<Self>,
    ) -> <QueryClock as Message>::Reply {
        tracing::debug!(msg.name = "QueryClock", %msg.triple);
        self.clocks
            .get(&msg.triple)
            .cloned()
            .ok_or(Error::ClockNotSynced)
    }
}

// query programs

#[derive(Message)]
#[reply("Arc<IndexMap<EventId, EpgProgram>>")]
pub struct QueryPrograms {
    pub service_triple: ServiceTriple,
}

#[async_trait]
impl<T> Handler<QueryPrograms> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: QueryPrograms,
        _ctx: &mut Context<Self>,
    ) -> <QueryPrograms as Message>::Reply {
        tracing::debug!(msg.name = "QueryPrograms", %msg.service_triple);
        self.schedules
            .get(&msg.service_triple)
            .map(|sched| sched.programs.clone())
            .unwrap_or_default()
    }
}

// query program

#[derive(Message)]
#[reply("Result<EpgProgram, Error>")]
pub enum QueryProgram {
    ByMirakurunProgramId(MirakurunProgramId), // For Mirakurun-compatible Web API
    ByProgramQuad(ProgramQuad),
}

#[async_trait]
impl<T> Handler<QueryProgram> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: QueryProgram,
        _ctx: &mut Context<Self>,
    ) -> <QueryProgram as Message>::Reply {
        tracing::debug!(msg.name = "QueryProgram");
        let (nid, sid, eid) = match msg {
            QueryProgram::ByMirakurunProgramId(id) => id.into(),
            QueryProgram::ByProgramQuad(quad) => quad.into(),
        };
        let triple = self
            .services
            .values()
            .find(|sv| sv.nid == nid && sv.sid == sid)
            .map(|sv| sv.triple())
            .ok_or(Error::ProgramNotFound)?;
        let schedule = self.schedules.get(&triple).ok_or(Error::ProgramNotFound)?;
        schedule
            .programs
            .get(&eid)
            .cloned()
            .map(|mut prog| {
                if let Some(airtime) = self.airtimes.get(&prog.quad) {
                    prog.start_at = airtime.start_time;
                    prog.duration = airtime.duration;
                }
                prog
            })
            .ok_or(Error::ProgramNotFound)
    }
}

// update services

#[derive(Message)]
pub struct UpdateServices {
    pub results: Vec<(EpgChannel, Option<IndexMap<ServiceTriple, EpgService>>)>,
}

#[async_trait]
impl<T> Handler<UpdateServices> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: UpdateServices, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateServices");
        self.update_services(msg.results).await;
    }
}

// update clocks

#[derive(Message)]
pub struct UpdateClocks {
    pub results: Vec<(EpgChannel, Option<HashMap<ServiceTriple, Clock>>)>,
}

#[async_trait]
impl<T> Handler<UpdateClocks> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: UpdateClocks, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateClocks");
        self.update_clocks(msg.results).await;
    }
}

// prepare schedule

#[derive(Message)]
pub struct PrepareSchedule {
    pub service_triple: ServiceTriple,
}

#[async_trait]
impl<T> Handler<PrepareSchedule> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: PrepareSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "PrepareSchedule", %msg.service_triple);
        self.prepare_schedule(msg.service_triple, Jst::today());
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
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: UpdateSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(
            msg.name = "UpdateSchedule",
            msg.service_triple = %msg.section.service_triple(),
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
    pub service_triple: ServiceTriple,
}

#[async_trait]
impl<T> Handler<FlushSchedule> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: FlushSchedule, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "FlushSchedule", %msg.service_triple);
        self.flush_schedule(msg.service_triple).await;
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
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, _msg: SaveSchedules, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "SaveSchedules");
        match self.save_schedules() {
            Ok(_) => (),
            Err(err) => tracing::error!("Failed to save schedules: {}", err),
        }
    }
}

// update airtime

#[derive(Message)]
#[reply("()")]
pub struct UpdateAirtime {
    pub quad: ProgramQuad,
    pub airtime: Airtime,
}

#[async_trait]
impl<T> Handler<UpdateAirtime> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: UpdateAirtime,
        _ctx: &mut Context<Self>,
    ) -> <UpdateAirtime as Message>::Reply {
        tracing::debug!(
            msg.name = "UpdateAirtime",
            %msg.quad,
            %msg.airtime.start_time,
            %msg.airtime.duration,
        );
        self.airtimes.insert(msg.quad, msg.airtime);
    }
}

// remove airtime

#[derive(Message)]
#[reply("()")]
pub struct RemoveAirtime {
    pub quad: ProgramQuad,
}

#[async_trait]
impl<T> Handler<RemoveAirtime> for Epg<T>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: RemoveAirtime,
        _ctx: &mut Context<Self>,
    ) -> <RemoveAirtime as Message>::Reply {
        tracing::debug!(msg.name = "RemoveAirtime", %msg.quad);
        self.airtimes.remove(&msg.quad);
    }
}

// register emitter

#[derive(Message)]
#[reply("()")]
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
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        _ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        match msg {
            RegisterEmitter::ServicesUpdated(emitter) => {
                emitter
                    .emit(ServicesUpdated {
                        services: self.services.clone(),
                    })
                    .await;
                self.services_emitters.push(emitter);
            }
            RegisterEmitter::ClocksUpdated(emitter) => {
                emitter
                    .emit(ClocksUpdated {
                        clocks: self.clocks.clone(),
                    })
                    .await;
                self.clocks_emitters.push(emitter);
            }
            RegisterEmitter::ProgramsUpdated(emitter) => {
                for service_triple in self.schedules.keys().cloned() {
                    emitter.emit(ProgramsUpdated { service_triple }).await;
                }
                self.programs_emitters.push(emitter);
            }
        }
    }
}

// notifications

#[derive(Message)]
pub struct ServicesUpdated {
    pub services: Arc<IndexMap<ServiceTriple, EpgService>>,
}

#[derive(Message)]
pub struct ClocksUpdated {
    pub clocks: Arc<HashMap<ServiceTriple, Clock>>,
}

#[derive(Message)]
pub struct ProgramsUpdated {
    pub service_triple: ServiceTriple,
}

// EpgSchedule holds sections of H-EIT[schedule basic] and H-EIT[schedule extended]
// for a particular service.  See ARIB TR-B14 for details.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSchedule {
    service_triple: ServiceTriple,
    // EpgScheduleUnits are stored in chronological order.
    units: [EpgScheduleUnit; Self::MAX_DAYS],
    #[serde(skip)]
    start_index: usize, // used for implementing a ring buffer on `units`.
    #[serde(skip)]
    programs: Arc<IndexMap<EventId, EpgProgram>>,
}

impl EpgSchedule {
    const MAX_DAYS: usize = 9;

    fn new(service_triple: ServiceTriple) -> Self {
        EpgSchedule {
            service_triple,
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
        let triple = self.service_triple;
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
            self.units[i].collect_programs(triple, &mut programs);
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

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut IndexMap<EventId, EpgProgram>,
    ) {
        for segment in self.segments.iter() {
            segment.collect_programs(triple, programs)
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

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut IndexMap<EventId, EpgProgram>,
    ) {
        let sections = self
            .extended_sections
            .iter()
            .chain(self.basic_sections.iter());
        for section in sections {
            if let Some(section) = section {
                section.collect_programs(triple, programs)
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

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut IndexMap<EventId, EpgProgram>,
    ) {
        for event in self.events.iter() {
            let quad = ProgramQuad::from((triple, EventId::from(event.event_id)));
            programs
                .entry(event.event_id)
                .or_insert(EpgProgram::new(quad))
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

#[derive(Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(Debug))]
pub struct EpgChannel {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    #[serde(default)]
    pub extra_args: String,
    pub services: Vec<ServiceId>,
    pub excluded_services: Vec<ServiceId>,
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgService {
    pub nid: NetworkId,
    pub tsid: TransportStreamId,
    pub sid: ServiceId,
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
    pub fn triple(&self) -> ServiceTriple {
        ServiceTriple::new(self.nid, self.tsid, self.sid)
    }
}

impl Into<MirakurunChannelService> for EpgService {
    fn into(self) -> MirakurunChannelService {
        MirakurunChannelService {
            id: self.triple().into(),
            service_id: self.sid,
            transport_stream_id: self.tsid,
            network_id: self.nid,
            name: self.name,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EpgProgram {
    pub quad: ProgramQuad,
    #[serde(with = "ts_milliseconds")]
    pub start_at: DateTime<Jst>,
    #[serde(with = "duration_milliseconds")]
    pub duration: Duration,
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
    pub fn new(quad: ProgramQuad) -> Self {
        Self {
            quad,
            start_at: Jst.timestamp_opt(0, 0).unwrap(),
            duration: Duration::minutes(0),
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

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("NO TITLE")
    }

    pub fn end_at(&self) -> DateTime<Jst> {
        self.start_at + self.duration
    }

    pub fn update(&mut self, event: &EitEvent) {
        self.start_at = event.start_time().unwrap().clone();
        self.duration = event.duration().unwrap().clone();
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

    #[tokio::test]
    async fn test_update_services() {
        fn create_service(name: &str, triple: ServiceTriple, channel: EpgChannel) -> EpgService {
            EpgService {
                nid: triple.nid(),
                tsid: triple.tsid(),
                sid: triple.sid(),
                service_type: 1,
                logo_id: 0,
                remote_control_key_id: 0,
                name: name.to_string(),
                channel,
            }
        }

        let mut epg = Epg::new(Arc::new(Default::default()), TunerManagerStub);

        let ch1 = EpgChannel {
            name: "ch1".to_string(),
            channel_type: ChannelType::GR,
            channel: "1".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
        };

        let ch2 = EpgChannel {
            name: "ch2".to_string(),
            channel_type: ChannelType::GR,
            channel: "2".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
        };

        let triple1_1 = ServiceTriple::from((1, 1, 1));
        let triple1_2 = ServiceTriple::from((1, 1, 2));
        let triple2_3 = ServiceTriple::from((2, 1, 3));
        let triple2_4 = ServiceTriple::from((2, 1, 4));

        // initial update

        let results = vec![
            (
                ch1.clone(),
                Some(indexmap::indexmap! {
                    triple1_1 =>
                        create_service("sv1", triple1_1, ch1.clone()),
                    triple1_2 =>
                        create_service("sv2", triple1_2, ch1.clone()),
                }),
            ),
            (
                ch2.clone(),
                Some(indexmap::indexmap! {
                    triple2_3 =>
                        create_service("sv3", triple2_3, ch2.clone()),
                    triple2_4 =>
                        create_service("sv4", triple2_4, ch2.clone()),
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
                    triple1_1 =>
                        create_service("sv1", triple1_1, ch1.clone()),
                    triple1_2 =>
                        create_service("sv2", triple1_2, ch1.clone()),
                }),
            ),
            (
                ch2.clone(),
                Some(indexmap::indexmap! {
                    triple2_3 =>
                        create_service("sv3", triple2_3, ch2.clone()),
                    triple2_4 =>
                        create_service("sv4", triple2_4, ch2.clone()),
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
                    triple2_3 =>
                        create_service("sv3", triple2_3, ch2.clone()),
                    triple2_4 =>
                        create_service("sv4", triple2_4, ch2.clone()),
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
                    triple1_1 =>
                        create_service("sv1.new", triple1_1, ch1.clone()),
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

    #[tokio::test]
    async fn test_update_services_purge_garbage_schedules() {
        let mut epg = Epg::new(Arc::new(Default::default()), TunerManagerStub);
        let triple = ServiceTriple::from((1, 2, 3));
        epg.schedules
            .insert(triple, Box::new(EpgSchedule::new(triple)));
        assert!(!epg.schedules.is_empty());
        let triple = ServiceTriple::from((1, 1, 1));
        let sv = create_epg_service(triple, ChannelType::GR);
        let ch = sv.channel.clone();
        epg.update_services(vec![(
            ch,
            Some(indexmap::indexmap! {
                triple => sv,
            }),
        )])
        .await;
        assert!(epg.schedules.is_empty());
    }

    #[test]
    fn test_epg_schedule_update_start_index() {
        let triple = ServiceTriple::from((1, 2, 3));
        let mut sched = EpgSchedule::new(triple);
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
        let triple = ServiceTriple::from((1, 2, 3));

        let mut sched = EpgSchedule::new(triple);
        sched.update(EitSection {
            original_network_id: triple.nid(),
            transport_stream_id: triple.tsid(),
            service_id: triple.sid(),
            table_id: 0x50, // unit-index(0)
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(sched.units[0].segments[0].basic_sections[0].is_some());

        let mut sched = EpgSchedule::new(triple);
        sched.update(EitSection {
            original_network_id: triple.nid(),
            transport_stream_id: triple.tsid(),
            service_id: triple.sid(),
            table_id: 0x51, // unit-index(4)
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(sched.units[4].segments[0].basic_sections[0].is_some());

        let mut sched = EpgSchedule::new(triple);
        sched.start_index = 5;
        sched.update(EitSection {
            original_network_id: triple.nid(),
            transport_stream_id: triple.tsid(),
            service_id: triple.sid(),
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
            transport_stream_id: 2.into(),
            service_id: 3.into(),
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
            transport_stream_id: 2.into(),
            service_id: 3.into(),
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

    fn create_epg_service(triple: ServiceTriple, channel_type: ChannelType) -> EpgService {
        EpgService {
            nid: triple.nid(),
            tsid: triple.tsid(),
            sid: triple.sid(),
            service_type: 1,
            logo_id: 0,
            remote_control_key_id: 0,
            name: "Service".to_string(),
            channel: EpgChannel {
                name: "Ch".to_string(),
                channel_type,
                channel: "ch".to_string(),
                extra_args: String::new(),
                services: Vec::new(),
                excluded_services: Vec::new(),
            },
        }
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
