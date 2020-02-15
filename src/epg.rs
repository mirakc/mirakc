use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::iter::FromIterator;
use std::path::PathBuf;
use std::sync::Arc;

use actix::prelude::*;
use chrono::{DateTime, Duration, TimeZone};
use indexmap::IndexMap;
use log;
use serde::{Deserialize, Serialize};

use crate::config::{Config, ChannelConfig};
use crate::datetime_ext::*;
use crate::eit_feeder::*;
use crate::error::Error;
use crate::fs_util;
use crate::job;
use crate::models::*;

pub fn start(config: Arc<Config>) {
    // Start on a new Arbiter instead of the system Arbiter.
    //
    // Epg performs several blocking processes like blow:
    //
    //   * Serialization and deserialization using serde
    //   * Conversions into Mirakurun-compatible models
    //
    let addr = Epg::start_in_arbiter(&Arbiter::new(), |_| Epg::new(config));
    actix::registry::SystemRegistry::set(addr);
}

pub async fn query_channels() -> Result<Vec<MirakurunChannel>, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Ok(Vec::new())
        } else {
            Epg::from_registry().send(QueryChannelsMessage).await?
        }
    }
}

pub async fn query_channel(
    channel_type: ChannelType,
    channel: String
) -> Result<EpgChannel, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            match channel.as_str() {
                "0" => Err(Error::ChannelNotFound),
                _ => Ok(EpgChannel {
                    name: "".to_string(),
                    channel_type,
                    channel,
                    services: Vec::new(),
                    excluded_services: Vec::new(),
                }),
            }
        } else {
            Epg::from_registry().send(QueryChannelMessage {
                channel_type, channel
            }).await?
        }
    }
}

pub async fn query_services() -> Result<Vec<EpgService>, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Ok(Vec::new())
        } else {
            Epg::from_registry().send(QueryServicesMessage).await?
        }
    }
}

pub async fn query_service_by_nid_sid(
    nid: NetworkId,
    sid: ServiceId,
) -> Result<EpgService, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            // for avoiding warn(dead_code)
            let _ = QueryServiceMessage::ByNidSid {
                nid, sid
            };
            match sid.value() {
                0 => Err(Error::ServiceNotFound),
                _ => Ok(EpgService {
                    nid: nid,
                    tsid: 0.into(),
                    sid: sid,
                    service_type: 1,
                    logo_id: 0,
                    remote_control_key_id: 0,
                    name: "test".to_string(),
                    channel: EpgChannel {
                        name: "test".to_string(),
                        channel_type: ChannelType::GR,
                        channel: "test".to_string(),
                        services: Vec::new(),
                        excluded_services: Vec::new(),
                    },
                }),
            }
        } else {
            Epg::from_registry().send(QueryServiceMessage::ByNidSid {
                nid, sid
            }).await?
        }
    }
}

pub async fn query_clock(triple: ServiceTriple) -> Result<Clock, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            match triple.sid().value() {
                0 => Err(Error::ClockNotSynced),
                _ => Ok(Clock { pcr: 0, time: 0 }),
            }
        } else {
            Epg::from_registry().send(QueryClockMessage { triple }).await?
        }
    }
}

pub async fn query_programs() -> Result<Vec<EpgProgram>, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Ok(Vec::new())
        } else {
            Epg::from_registry().send(QueryProgramsMessage).await?
        }
    }
}

pub async fn query_program_by_nid_sid_eid(
    nid: NetworkId,
    sid: ServiceId,
    eid: EventId,
) -> Result<EpgProgram, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            // for avoiding warn(dead_code)
            let _ = QueryProgramMessage::ByNidSidEid {
                nid, sid, eid
            };
            match eid.value() {
                0 => Err(Error::ProgramNotFound),
                _ => Ok(EpgProgram::new((nid, 0.into(), sid, eid).into())),
            }
        } else {
            Epg::from_registry().send(QueryProgramMessage::ByNidSidEid {
                nid, sid, eid
            }).await?
        }
    }
}

pub fn update_services(services: Vec<EpgService>) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = services;
        } else {
            Epg::from_registry().do_send(UpdateServicesMessage { services });
        }
    }
}

pub fn update_clocks(clocks: HashMap<ServiceTriple, Clock>) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = clocks;
        } else {
            Epg::from_registry().do_send(UpdateClocksMessage { clocks });
        }
    }
}

pub fn update_schedules(sections: Vec<EitSection>) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = sections;
        } else {
            Epg::from_registry().do_send(UpdateSchedulesMessage { sections });
        }
    }
}

pub fn flush_schedules(triples: Vec<ServiceTriple>) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = triples;
        } else {
            Epg::from_registry().do_send(FlushSchedulesMessage {
                triples
            });
        }
    }
}

pub fn save_schedules() {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
        } else {
            Epg::from_registry().do_send(SaveSchedulesMessage);
        }
    }
}

pub fn update_airtime(
    quad: EventQuad,
    start_time: DateTime<Jst>,
    duration: Duration
) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = (quad, start_time, duration);
        } else {
            Epg::from_registry().do_send(UpdateAirtimeMessage {
                quad,
                airtime: Airtime { start_time, duration }
            });
        }
    }
}

pub fn remove_airtime(quad: EventQuad) {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            let _ = quad;
        } else {
            Epg::from_registry().do_send(RemoveAirtimeMessage {
                quad
            });
        }
    }
}

struct Epg {
    config: Arc<Config>,
    services: Vec<EpgService>,
    clocks: HashMap<ServiceTriple, Clock>,
    schedules: HashMap<ServiceTriple, EpgSchedule>,
    airtimes: HashMap<EventQuad, Airtime>,
}

struct Airtime {
    start_time: DateTime<Jst>,
    duration: Duration
}

impl Epg {
    fn new(config: Arc<Config>) -> Self {
        Epg {
            config,
            services: Vec::new(),
            clocks: HashMap::new(),
            schedules: HashMap::new(),
            airtimes: HashMap::new(),
        }
    }

    fn find_service_by_triple(
        &self,
        triple: ServiceTriple
    ) -> Option<&EpgService> {
        self.services.iter().find(|sv| sv.triple() == triple)
    }

    fn update_services(&mut self, services: Vec<EpgService>) {
        self.services = services;
        match self.save_services() {
            Ok(_) => (),
            Err(err) => log::error!("Failed to save services: {}", err),
        }
    }

    fn update_clocks(
        &mut self,
        clocks: HashMap<ServiceTriple, Clock>) {
        self.clocks = clocks;
        match self.save_clocks() {
            Ok(_) => (),
            Err(err) => log::error!("Failed to save clocks: {}", err),
        }
    }

    fn update_schedules(&mut self, sections: Vec<EitSection>) {
        self.prepare_schedules(Jst::now());
        for section in sections.into_iter() {
            let triple = section.service_triple();
            self.schedules.entry(triple).and_modify(move |sched| {
                sched.update(section);
            });
        }
    }

    fn flush_schedules(&mut self, triples: Vec<ServiceTriple>) {
        for triple in triples.iter() {
            let num_programs = match self.schedules.get_mut(triple) {
                Some(schedule) => {
                    schedule.collect_programs();
                    schedule.programs.len()
                }
                None => 0,
            };
            if num_programs > 0 {
                let service = self.find_service_by_triple(*triple)
                    .expect("Service must exist");
                log::info!("Collected {} programs of {} ({})",
                           num_programs, service.name, triple);
            }
        }
    }

    fn prepare_schedules(&mut self, timestamp: DateTime<Jst>) {
        let mut unused_ids: HashSet<_> =
            HashSet::from_iter(self.schedules.keys().cloned());

        let midnight = timestamp.date().and_hms(0, 0, 0);

        for service in self.services.iter() {
            let triple = service.triple();
            self.schedules
                .entry(triple)
                .and_modify(|sched| {
                    if sched.updated_at < midnight {
                        // Save overnight events.  The overnight events will be
                        // lost in `update_tables()`.
                        sched.save_overnight_events(midnight);
                    }
                    sched.updated_at = timestamp;
                })
                .or_insert(EpgSchedule::new(triple));
            unused_ids.remove(&triple);
        }

        // Removing "garbage" schedules.
        for id in unused_ids.iter() {
            self.schedules.remove(&id);
            log::debug!("Removed schedule#{}", id);
        }
    }

    fn load_services(&mut self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("services.json");
                log::debug!("Loading schedules from {}...",
                            json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                self.services = serde_json::from_reader(reader)?;
                log::info!("Loaded {} services", self.services.len());
            }
            None => {
                log::warn!("No epg.cache-dir specified, skip to load services");
            }
        }
        Ok(())
    }

    fn load_clocks(&mut self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("clocks.json");
                log::debug!("Loading clocks from {}...", json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                self.clocks = serde_json::from_reader(reader)?;
                log::info!("Loaded {} clocks", self.clocks.len());
            }
            None => {
                log::warn!("No epg.cache-dir specified, skip to load clock");
            }
        }
        Ok(())
    }

    fn load_schedules(&mut self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("schedules.json");
                log::debug!("Loading schedules from {}...", json_path.display());
                let reader = BufReader::new(File::open(&json_path)?);
                self.schedules = serde_json::from_reader(reader)?;
                log::info!("Loaded schedules for {} services", self.schedules.len());
            }
            None => {
                log::warn!("No epg.cache-dir specified, skip to load");
            }
        }
        Ok(())
    }

    fn need_scaning_services(&self) -> bool {
        match (self.services.is_empty(), &self.config.epg.cache_dir) {
            (false, Some(ref cache_dir)) => {
                let json_path = PathBuf::from(cache_dir).join("services.json");
                fs_util::unmodified_since(&json_path, self.config.last_modified)
            }
            _ => true,
        }
    }

    fn need_synchronizing_clocks(&self) -> bool {
        match (self.clocks.is_empty(), &self.config.epg.cache_dir) {
            (false, Some(ref cache_dir)) => {
                let json_path = PathBuf::from(cache_dir).join("clocks.json");
                fs_util::unmodified_since(&json_path, self.config.last_modified)
            }
            _ => true,
        }
    }

    fn save_services(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("services.json");
                log::debug!("Saving services into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                serde_json::to_writer(writer, &self.services)?;
                log::info!("Saved {} services", self.services.len());
            }
            None => {
                log::warn!("No epg.cache-dir specified, skip to save services");
            }
        }
        Ok(())
    }

    fn save_clocks(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("clocks.json");
                log::debug!("Saving clocks into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                serde_json::to_writer(writer, &self.clocks)?;
                log::info!("Saved {} clocks", self.clocks.len());
            }
            None => {
                log::warn!("No epg.cache-dir specified, skip to save clocks");
            }
        }
        Ok(())
    }

    fn save_schedules(&self) -> Result<(), Error> {
        match self.config.epg.cache_dir {
            Some(ref cache_dir) => {
                let json_path = PathBuf::from(cache_dir).join("schedules.json");
                log::debug!("Saving schedules into {}...", json_path.display());
                let writer = BufWriter::new(File::create(&json_path)?);
                serde_json::to_writer(writer, &self.schedules)?;
                log::info!("Saved schedules for {} services",
                           self.schedules.len());
            }
            None => {
                log::warn!(
                    "No epg.cache-dir specified, skip to save schedules");
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

impl Actor for Epg {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        log::debug!("Started");
        if let Err(err) = self.load_services() {
            log::error!("Failed to load services: {}", err);
        }
        if self.need_scaning_services() {
            log::info!("Scan services immediately");
            job::invoke_scan_services();
        }
        if let Err(err) = self.load_clocks() {
            log::error!("Failed to load clocks: {}", err);
        }
        if self.need_synchronizing_clocks() {
            log::info!("Synchronize clocks immediately");
            job::invoke_sync_clocks();
        }
        if let Err(err) = self.load_schedules() {
            log::error!("Failed to load schedules: {}", err);
        }
        self.collect_programs();
        log::info!("Always update schedules at startup");
        job::invoke_update_schedules();
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::debug!("Stopped");
    }
}

impl Supervised for Epg {}
impl SystemService for Epg {}

impl Default for Epg {
    fn default() -> Self {
        unreachable!();
    }
}

// query channels

struct QueryChannelsMessage;

impl fmt::Display for QueryChannelsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryChannels")
    }
}

impl Message for QueryChannelsMessage {
    type Result = Result<Vec<MirakurunChannel>, Error>;
}

impl Handler<QueryChannelsMessage> for Epg {
    type Result = Result<Vec<MirakurunChannel>, Error>;

    fn handle(
        &mut self,
        msg: QueryChannelsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        let channels = self.config.channels.iter()
            .filter(|config| !config.disabled)
            .map(|config| MirakurunChannel {
                channel_type: config.channel_type,
                channel:  config.channel.clone(),
                name: config.name.clone(),
                services: self.services
                    .iter()
                    .filter(|sv| {
                        sv.channel.channel_type == config.channel_type &&
                            sv.channel.channel == config.channel
                    })
                    .cloned()
                    .map(|sv| sv.into())
                    .collect()
            })
            .collect::<Vec<MirakurunChannel>>();

        Ok(channels)
    }
}

// query channel

struct QueryChannelMessage {
    channel_type: ChannelType,
    channel: String,
}

impl fmt::Display for QueryChannelMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryChannel by {}/{}", self.channel_type, self.channel)
    }
}

impl Message for QueryChannelMessage {
    type Result = Result<EpgChannel, Error>;
}

impl Handler<QueryChannelMessage> for Epg {
    type Result = Result<EpgChannel, Error>;

    fn handle(
        &mut self,
        msg: QueryChannelMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.config.channels.iter()
            .filter(|config| !config.disabled)
            .find(|config| {
                  config.channel_type == msg.channel_type &&
                    config.channel == msg.channel
            })
            .cloned()
            .map(EpgChannel::from)
            .ok_or(Error::ChannelNotFound)
    }
}

// query services

struct QueryServicesMessage;

impl fmt::Display for QueryServicesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryServices")
    }
}

impl Message for QueryServicesMessage {
    type Result = Result<Vec<EpgService>, Error>;
}

impl Handler<QueryServicesMessage> for Epg {
    type Result = Result<Vec<EpgService>, Error>;

    fn handle(
        &mut self,
        msg: QueryServicesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        Ok(self.services.iter()
           .filter(|sv| sv.is_exportable())
           .cloned()
           .collect())
    }
}

// query service

enum QueryServiceMessage {
    // For Mirakurun-compatible Web API
    ByNidSid { nid: NetworkId, sid: ServiceId },
}

impl fmt::Display for QueryServiceMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryServiceMessage::ByNidSid { nid, sid } =>
                write!(f, "QueryService By ({}, {})", nid, sid),
        }
    }
}

impl Message for QueryServiceMessage {
    type Result = Result<EpgService, Error>;
}

impl Handler<QueryServiceMessage> for Epg {
    type Result = Result<EpgService, Error>;

    fn handle(
        &mut self,
        msg: QueryServiceMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        match msg {
            QueryServiceMessage::ByNidSid { nid, sid } => {
                self.services
                    .iter()
                    .find(|sv| sv.nid == nid && sv.sid == sid)
                    .cloned()
                    .ok_or(Error::ServiceNotFound)
            }
        }
    }
}

// query clock

struct QueryClockMessage {
    triple: ServiceTriple,
}

impl fmt::Display for QueryClockMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryClock by {}", self.triple)
    }
}

impl Message for QueryClockMessage {
    type Result = Result<Clock, Error>;
}

impl Handler<QueryClockMessage> for Epg {
    type Result = Result<Clock, Error>;

    fn handle(
        &mut self,
        msg: QueryClockMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.clocks.get(&msg.triple).cloned().ok_or(Error::ClockNotSynced)
    }
}

// query programs

struct QueryProgramsMessage;

impl fmt::Display for QueryProgramsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryPrograms")
    }
}

impl Message for QueryProgramsMessage {
    type Result = Result<Vec<EpgProgram>, Error>;
}

impl Handler<QueryProgramsMessage> for Epg {
    type Result = Result<Vec<EpgProgram>, Error>;

    fn handle(
        &mut self,
        msg: QueryProgramsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        let mut programs = Vec::new();
        for schedule in self.schedules.values() {
            programs.extend(schedule.programs.values().cloned())
        }
        Ok(programs)
    }
}

// query program

enum QueryProgramMessage {
    // For Mirakurun-compatible Web API
    ByNidSidEid { nid: NetworkId, sid: ServiceId, eid: EventId },
}

impl fmt::Display for QueryProgramMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryProgramMessage::ByNidSidEid { nid, sid, eid } =>
                write!(f, "QueryProgram By ({}, {}, {})", nid, sid, eid),
        }
    }
}

impl Message for QueryProgramMessage {
    type Result = Result<EpgProgram, Error>;
}

impl Handler<QueryProgramMessage> for Epg {
    type Result = Result<EpgProgram, Error>;

    fn handle(
        &mut self,
        msg: QueryProgramMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        match msg {
            QueryProgramMessage::ByNidSidEid { nid, sid, eid } => {
                let triple = self.services
                    .iter()
                    .find(|sv| sv.nid == nid && sv.sid == sid)
                    .map(|sv| sv.triple())
                    .ok_or(Error::ProgramNotFound)?;
                let schedule = self.schedules.get(&triple)
                    .ok_or(Error::ProgramNotFound)?;
                schedule.programs.get(&eid).cloned()
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
    }
}

// update services

struct UpdateServicesMessage {
    services: Vec<EpgService>,
}

impl fmt::Display for UpdateServicesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UpdateServices with {} services", self.services.len())
    }
}

impl Message for UpdateServicesMessage {
    type Result = ();
}

impl Handler<UpdateServicesMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: UpdateServicesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.update_services(msg.services);
    }
}

// update clocks

struct UpdateClocksMessage {
    clocks: HashMap<ServiceTriple, Clock>,
}

impl fmt::Display for UpdateClocksMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UpdateClocks with {} clocks", self.clocks.len())
    }
}

impl Message for UpdateClocksMessage {
    type Result = ();
}

impl Handler<UpdateClocksMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: UpdateClocksMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.update_clocks(msg.clocks);
    }
}

// update schedules

struct UpdateSchedulesMessage {
    sections: Vec<EitSection>,
}

impl fmt::Display for UpdateSchedulesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UpdateSchedules with {} EIT sections", self.sections.len())
    }
}

impl Message for UpdateSchedulesMessage {
    type Result = ();
}

impl Handler<UpdateSchedulesMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: UpdateSchedulesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.update_schedules(msg.sections);
    }
}

// flush schedules

struct FlushSchedulesMessage {
    triples: Vec<ServiceTriple>,
}

impl fmt::Display for FlushSchedulesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let triples: Vec<String> = self.triples
            .iter()
            .map(|triple| triple.to_string())
            .collect();
        write!(f, "FlushSchedules for [{}]", triples.as_slice().join(", "))
    }
}

impl Message for FlushSchedulesMessage {
    type Result = ();
}

impl Handler<FlushSchedulesMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: FlushSchedulesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.flush_schedules(msg.triples);
    }
}

// save schedules

struct SaveSchedulesMessage;

impl fmt::Display for SaveSchedulesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SaveSchedules")
    }
}

impl Message for SaveSchedulesMessage {
    type Result = ();
}

impl Handler<SaveSchedulesMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: SaveSchedulesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        match self.save_schedules() {
            Ok(_) => (),
            Err(err) => log::error!("Failed to save schedules: {}", err),
        }
    }
}

// update airtime

struct UpdateAirtimeMessage {
    quad: EventQuad,
    airtime: Airtime,
}

impl fmt::Display for UpdateAirtimeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UpdateAirtime of {}", self.quad)
    }
}

impl Message for UpdateAirtimeMessage {
    type Result = ();
}

impl Handler<UpdateAirtimeMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: UpdateAirtimeMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.airtimes.insert(msg.quad, msg.airtime);
    }
}

// remove airtime

struct RemoveAirtimeMessage {
    quad: EventQuad,
}

impl fmt::Display for RemoveAirtimeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RemoveAirtime of {}", self.quad)
    }
}

impl Message for RemoveAirtimeMessage {
    type Result = ();
}

impl Handler<RemoveAirtimeMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        msg: RemoveAirtimeMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.airtimes.remove(&msg.quad);
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSchedule {
    service_triple: ServiceTriple,
    // In Japan, only the following indexes are used:
    //
    //    0 | 8 | 16 | 24 => the former 4 days of 8 days schedule
    //    1 | 9 | 17 | 25 => the later 4 days of 8 days schedule
    tables: [Option<Box<EpgTable>>; 32],
    overnight_events: Vec<EitEvent>,
    #[serde(with = "serde_jst")]
    updated_at: DateTime<Jst>,
    #[serde(skip)]
    programs: HashMap<EventId, EpgProgram>,
}

impl EpgSchedule {
    fn new(triple: ServiceTriple) -> EpgSchedule {
        EpgSchedule {
            service_triple: triple,
            tables: Default::default(),
            overnight_events: Vec::new(),
            updated_at: Jst::now(),
            programs: HashMap::new(),
        }
    }

    fn update(&mut self, section: EitSection) {
        let i = section.table_index();
        if self.tables[i].is_none() {
            self.tables[i] = Some(Box::new(EpgTable::default()));
        }
        self.tables[i].as_mut().unwrap().update(section);
    }

    fn save_overnight_events(&mut self, midnight: DateTime<Jst>) {
        let mut events = Vec::new();
        for table in self.tables.iter() {
            if let Some(ref table) = table {
                events = table.collect_overnight_events(midnight, events);
            }
        }
        log::debug!("Saved {} overnight events of schedule#{}",
                    events.len(), self.service_triple);
        self.overnight_events = events;
    }

    fn collect_programs(&mut self) {
        let mut programs = HashMap::new();
        for event in self.overnight_events.iter() {
            let quad = EventQuad::from(
                (self.service_triple, EventId::from(event.event_id)));
            programs
                .entry(event.event_id)
                .or_insert(EpgProgram::new(quad))
                .update(event);
        }
        for table in self.tables.iter() {
            if let Some(table) = table {
                table.collect_programs(self.service_triple, &mut programs)
            }
        }
        self.programs = programs;
    }
}

#[derive(Default)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
// A table contains TV program information about 4 days of a TV program
// schedule.
struct EpgTable {
    // Segments are stored in chronological order.
    //
    // The first 8 consecutive segments contains TV program information for the
    // first day.
    segments: [EpgSegment; 32],
}

impl EpgTable {
    fn update(&mut self, section: EitSection) {
        let i = section.segment_index();
        self.segments[i].update(section);
    }

    fn collect_overnight_events(
        &self,
        midnight: DateTime<Jst>,
        mut events: Vec<EitEvent>
    ) -> Vec<EitEvent> {
        for segment in self.segments.iter() {
            events = segment.collect_overnight_events(midnight, events);
        }
        events
    }

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventId, EpgProgram>) {
        for segment in self.segments.iter() {
            segment.collect_programs(triple, programs)
        }
    }
}

#[derive(Default)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
// A segment contains TV program information about 3 hours of a TV program
// schedule.
struct EpgSegment {
    // Sections are stored in chronological order.
    sections: [Option<EpgSection>; 8],
}

impl EpgSegment {
    fn update(&mut self, section: EitSection) {
        let n = section.last_section_index() + 1;
        for i in n..8 {
            self.sections[i] = None;
        }

        let i = section.section_index();
        self.sections[i] = Some(EpgSection::from(section));
    }

    fn collect_overnight_events(
        &self,
        midnight: DateTime<Jst>,
        events: Vec<EitEvent>
    ) -> Vec<EitEvent> {
        self.sections
            .iter()
            .filter(|section| section.is_some())
            .map(|section| section.as_ref().unwrap())
            .fold(events, |events_, section| {
                section.collect_overnight_events(midnight, events_)
            })
    }

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventId, EpgProgram>) {
        for section in self.sections.iter() {
            if let Some(section) = section {
                section.collect_programs(triple, programs)
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgSection {
    version: u8,
    // Events are stored in chronological order.
    events: Vec<EitEvent>,
}

impl EpgSection {
    fn collect_overnight_events(
        &self,
        midnight: DateTime<Jst>,
        mut events: Vec<EitEvent>
    ) -> Vec<EitEvent> {
        for event in self.events.iter() {
            if event.is_overnight_event(midnight) {
                events.push(event.clone());
            }
        }
        events
    }

    fn collect_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventId, EpgProgram>) {
        for event in self.events.iter() {
            let quad = EventQuad::from((triple, EventId::from(event.event_id)));
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

#[derive(Clone)]
#[derive(Deserialize, Serialize)]
pub struct EpgChannel {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    pub services: Vec<ServiceId>,
    pub excluded_services: Vec<ServiceId>,
}

impl From<ChannelConfig> for EpgChannel {
    fn from(config: ChannelConfig) -> Self {
        EpgChannel {
            name: config.name,
            channel_type: config.channel_type,
            channel: config.channel,
            services: config.services,
            excluded_services: config.excluded_services,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsService {
    nid: NetworkId,
    tsid: TransportStreamId,
    sid: ServiceId,
    #[serde(rename = "type")]
    service_type: u16,
    #[serde(default)]
    logo_id: i16,
    #[serde(default)]
    remote_control_key_id: u16,
    name: String,
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

    fn is_exportable(&self) -> bool {
        if !self.channel.services.is_empty() {
            if !self.channel.services.contains(&self.sid) {
                return false;
            }
        }
        if self.channel.excluded_services.contains(&self.sid) {
            return false;
        }
        true
    }
}

impl From<(&EpgChannel, &TsService)> for EpgService {
    fn from((ch, sv): (&EpgChannel, &TsService)) -> Self {
        EpgService {
            nid: sv.nid,
            tsid: sv.tsid,
            sid: sv.sid,
            service_type: sv.service_type,
            logo_id: sv.logo_id,
            remote_control_key_id: sv.remote_control_key_id,
            name: sv.name.clone(),
            channel: ch.clone(),
        }
    }
}

impl Into<MirakurunChannelService> for EpgService {
    fn into(self) -> MirakurunChannelService {
        MirakurunChannelService {
            id: self.triple().into(),
            service_id: self.sid,
            network_id: self.nid,
            name: self.name,
        }
    }
}

#[derive(Clone)]
pub struct EpgProgram {
    pub quad: EventQuad,
    pub start_at: DateTime<Jst>,
    pub duration: Duration,
    pub scrambled: bool,
    pub name: Option<String>,
    pub description: Option<String>,
    pub extended: Option<IndexMap<String, String>>,
    pub video: Option<EpgVideoInfo>,
    pub audio: Option<EpgAudioInfo>,
    pub genres: Option<Vec<EpgGenre>>,
}

impl EpgProgram {
    fn new(quad: EventQuad) -> Self {
        Self {
            quad: quad,
            start_at: Jst.timestamp(0, 0),
            duration: Duration::minutes(0),
            scrambled: false,
            name: None,
            description: None,
            extended: None,
            video: None,
            audio: None,
            genres: None,
        }
    }

    fn _end_at(&self) -> DateTime<Jst> {
        self.start_at + self.duration
    }

    fn update(&mut self, event: &EitEvent) {
        self.start_at = event.start_time.clone();
        self.duration = event.duration.clone();
        self.scrambled = event.scrambled;
        for desc in event.descriptors.iter() {
            match desc {
                EitDescriptor::ShortEvent { event_name, text } => {
                    self.name = Some(event_name.clone());
                    self.description = Some(text.clone());
                }
                EitDescriptor::Component { stream_content, component_type } => {
                    self.video = Some(
                        EpgVideoInfo::new(*stream_content, *component_type));
                }
                EitDescriptor::AudioComponent {
                    component_type, sampling_rate } => {
                    self.audio = Some(
                        EpgAudioInfo::new(*component_type, *sampling_rate));
                }
                EitDescriptor::Content { nibbles } => {
                    self.genres = Some(nibbles.iter()
                                       .map(|nibble| EpgGenre::new(*nibble))
                                       .collect());
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Date, TimeZone};

    #[test]
    fn test_epg_service_is_exportable() {
        let triple = ServiceTriple::from((1, 2, 3));

        let service = create_epg_service(triple, ChannelType::GR);
        assert!(service.is_exportable());

        let mut service = create_epg_service(triple, ChannelType::GR);
        service.channel.services = vec![3.into()];
        assert!(service.is_exportable());

        let mut service = create_epg_service(triple, ChannelType::GR);
        service.channel.services = vec![4.into()];
        assert!(!service.is_exportable());

        let mut service = create_epg_service(triple, ChannelType::GR);
        service.channel.excluded_services = vec![3.into()];
        assert!(!service.is_exportable());

        let mut service = create_epg_service(triple, ChannelType::GR);
        service.channel.excluded_services = vec![4.into()];
        assert!(service.is_exportable());
    }

    #[test]
    fn test_epg_prepare_schedule() {
        let triple = ServiceTriple::from((1, 2, 3));
        let channel_type = ChannelType::GR;
        let config = Arc::new(Config::default());

        let mut epg = Epg::new(config.clone());
        epg.services = vec![create_epg_service(triple, channel_type)];
        epg.prepare_schedules(Jst::now());
        assert_eq!(epg.schedules.len(), 1);
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        let mut epg = Epg::new(config.clone());
        epg.services = vec![create_epg_service(triple, channel_type)];
        let sched = create_epg_schedule_with_overnight_events(triple);
        epg.schedules.insert(triple, sched);

        epg.prepare_schedules(Jst.ymd(2019, 10, 13).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 14).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 4);

        epg.prepare_schedules(Jst.ymd(2019, 10, 15).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 16).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 17).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 18).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 1);

        epg.prepare_schedules(Jst.ymd(2019, 10, 19).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 20).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 21).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);

        epg.prepare_schedules(Jst.ymd(2019, 10, 22).and_hms(0, 0, 0));
        assert_eq!(epg.schedules[&triple].overnight_events.len(), 0);
    }

    #[test]
    fn test_epg_schedule_update() {
        let triple = ServiceTriple::from((1, 2, 3));
        let mut sched = create_epg_schedule(triple);

        sched.update(EitSection {
            original_network_id: triple.nid(),
            transport_stream_id: triple.tsid(),
            service_id: triple.sid(),
            table_id: 0x50,
            section_number: 0x00,
            last_section_number: 0xF8,
            segment_last_section_number: 0x00,
            version_number: 1,
            events: Vec::new(),
        });
        assert!(sched.tables[0].is_some());
    }

    #[test]
    fn test_epg_schedule_save_overnight_events() {
        let mut sched = create_epg_schedule_with_overnight_events(
            ServiceTriple::from((1, 2, 3)));
        sched.save_overnight_events(Jst.ymd(2019, 10, 13).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 14).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 4);

        sched.save_overnight_events(Jst.ymd(2019, 10, 15).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 16).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 17).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 18).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 1);

        sched.save_overnight_events(Jst.ymd(2019, 10, 19).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 20).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 21).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);

        sched.save_overnight_events(Jst.ymd(2019, 10, 22).and_hms(0, 0, 0));
        assert_eq!(sched.overnight_events.len(), 0);
    }

    #[test]
    fn test_epg_table_update() {
        let mut table: EpgTable = Default::default();

        table.update(EitSection {
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
        assert!(table.segments[0].sections[0].is_some());
    }

    #[test]
    fn test_epg_table_collect_overnight_events() {
        let table = create_epg_table_with_overnight_events(
            Jst.ymd(2019, 10, 13));
        let events = table.collect_overnight_events(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0), Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2.into());
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
        assert!(segment.sections[0].is_none());
        assert!(segment.sections[1].is_some());

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
        assert!(segment.sections[0].is_some());
        assert!(segment.sections[1].is_none());
    }

    #[test]
    fn test_epg_segment_collect_overnight_events() {
        let segment = create_epg_segment_with_overnight_events(
            Jst.ymd(2019, 10, 13));
        let events = segment.collect_overnight_events(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0), Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2.into());
    }

    #[test]
    fn test_epg_section_collect_overnight_events() {
        let section = create_epg_section_with_overnight_events(
            Jst.ymd(2019, 10, 13));
        let events = section.collect_overnight_events(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0), Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2.into());
    }

    #[test]
    fn test_eit_event_is_overnight_event() {
        let event = EitEvent {
            event_id: 0.into(),
            start_time: Jst.ymd(2019, 10, 13).and_hms(23, 59, 59),
            duration: Duration::seconds(2),
            scrambled: false,
            descriptors: Vec::new(),
        };
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 12).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 13).and_hms(0, 0, 0)));
        assert!(event.is_overnight_event(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 15).and_hms(0, 0, 0)));

        let event = EitEvent {
            event_id: 0.into(),
            start_time: Jst.ymd(2019, 10, 13).and_hms(23, 59, 59),
            duration: Duration::seconds(1),
            scrambled: false,
            descriptors: Vec::new(),
        };
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 12).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 13).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 15).and_hms(0, 0, 0)));

        let event = EitEvent {
            event_id: 0.into(),
            start_time: Jst.ymd(2019, 10, 13).and_hms(23, 59, 58),
            duration: Duration::seconds(1),
            scrambled: false,
            descriptors: Vec::new(),
        };
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 12).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 13).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 14).and_hms(0, 0, 0)));
        assert!(!event.is_overnight_event(
            Jst.ymd(2019, 10, 15).and_hms(0, 0, 0)));
    }

    fn create_epg_service(
        triple: ServiceTriple,
        channel_type: ChannelType
    ) -> EpgService {
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
                services: Vec::new(),
                excluded_services: Vec::new(),
            }
        }
    }

    fn create_epg_schedule(triple: ServiceTriple) -> EpgSchedule {
        EpgSchedule::new(triple)
    }

    fn create_epg_schedule_with_overnight_events(
        triple: ServiceTriple
    ) -> EpgSchedule {
        let mut sched = create_epg_schedule(triple);
        sched.updated_at = Jst.ymd(2019, 10, 13).and_hms(0, 0, 0);
        sched.tables[0] = Some(Box::new(
            create_epg_table_with_overnight_events(Jst.ymd(2019, 10, 13))));
        sched.tables[1] = Some(Box::new(
            create_epg_table_with_overnight_events(Jst.ymd(2019, 10, 17))));
        sched.tables[8] = Some(Box::new(
            create_epg_table_with_overnight_events(Jst.ymd(2019, 10, 13))));
        sched.tables[16] = Some(Box::new(
            create_epg_table_with_overnight_events(Jst.ymd(2019, 10, 13))));
        sched.tables[24] = Some(Box::new(
            create_epg_table_with_overnight_events(Jst.ymd(2019, 10, 13))));
        sched
    }

    fn create_epg_table_with_overnight_events(date: Date<Jst>) -> EpgTable {
        let mut table = EpgTable::default();
        table.segments[7] = create_epg_segment_with_overnight_events(date);
        table
    }

    fn create_epg_segment_with_overnight_events(date: Date<Jst>) -> EpgSegment {
        let mut segment = EpgSegment::default();
        segment.sections[0] =
            Some(EpgSection { version: 1, events: Vec::new() });
        segment.sections[1] =
            Some(create_epg_section_with_overnight_events(date));
        segment
    }

    fn create_epg_section_with_overnight_events(date: Date<Jst>) -> EpgSection {
        EpgSection {
            version: 1,
            events: vec![
                EitEvent {
                    event_id: 1.into(),
                    start_time: date.and_hms(23, 0, 0),
                    duration: Duration::minutes(30),
                    scrambled: false,
                    descriptors: Vec::new(),
                },
                EitEvent {
                    event_id: 2.into(),
                    start_time: date.and_hms(23, 30, 0),
                    duration: Duration::hours(1),
                    scrambled: false,
                    descriptors: Vec::new(),
                },
            ]
        }
    }
}
