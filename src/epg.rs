use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::iter::FromIterator;
use std::path::PathBuf;
use std::sync::Arc;

use actix::prelude::*;
use chrono::{DateTime, Duration};
use indexmap::IndexMap;
use log;
use serde::{Deserialize, Serialize};

use crate::config::{Config, ChannelConfig};
use crate::datetime_ext::*;
use crate::error::Error;
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

pub async fn query_programs() -> Result<Vec<MirakurunProgram>, Error> {
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
) -> Result<MirakurunProgram, Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            // for avoiding warn(dead_code)
            let _ = QueryProgramMessage::ByNidSidEid {
                nid, sid, eid
            };
            match eid.value() {
                0 => Err(Error::ProgramNotFound),
                _ => Ok(MirakurunProgram::new((nid, 0.into(), sid, eid).into())),
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

pub fn flush_schedules() {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
        } else {
            Epg::from_registry().do_send(FlushSchedulesMessage);
        }
    }
}

struct Epg {
    cache_dir: Option<PathBuf>,
    channels: Vec<ChannelConfig>,
    services: Vec<EpgService>,
    clocks: HashMap<ServiceTriple, Clock>,
    schedules: HashMap<ServiceTriple, EpgSchedule>,
    programs: HashMap<EventQuad, MirakurunProgram>,
}

impl Epg {
    fn new(config: Arc<Config>) -> Self {
        let channels = config.channels
            .iter()
            .filter(|config| !config.disabled)
            .cloned()
            .collect();

        let cache_dir = config.epg.cache_dir.clone().map(PathBuf::from);
        Epg {
            cache_dir,
            channels,
            services: Vec::new(),
            clocks: HashMap::new(),
            schedules: HashMap::new(),
            programs: HashMap::new(),
        }
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

    fn flush_schedules(&mut self) {
        self.programs = self.export_programs();
        match self.save_schedules() {
            Ok(_) => (),
            Err(err) => log::error!("Failed to save schedules: {}", err),
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
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("services.json");
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
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("clocks.json");
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
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("schedules.json");
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

    fn save_services(&self) -> Result<(), Error> {
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("services.json");
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
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("clocks.json");
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
        match self.cache_dir {
            Some(ref cache_dir) => {
                let json_path = cache_dir.join("schedules.json");
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

    fn export_programs(&self) -> HashMap<EventQuad, MirakurunProgram> {
        let mut programs = HashMap::new();
        for service in self.services.iter().filter(|sv| sv.is_exportable()) {
            let triple = service.triple();
            match self.schedules.get(&triple) {
                Some(sched) =>
                    sched.collect_epg_programs(triple, &mut programs),
                None => log::warn!("Schedule not found for {}", triple),
            }
        }
        programs
    }
}

impl Actor for Epg {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        log::debug!("Started");
        if let Err(err) = self.load_services() {
            log::error!("Failed to load services: {}", err);
        }
        if self.services.is_empty() {
            log::info!("No services are avaiable, scan services immediately");
            job::invoke_scan_services();
        }
        if let Err(err) = self.load_clocks() {
            log::error!("Failed to load clocks: {}", err);
        }
        if self.clocks.is_empty() {
            log::info!("No clocks are available, sync clocks immediately");
            job::invoke_sync_clocks();
        }
        if let Err(err) = self.load_schedules() {
            log::error!("Failed to load schedules: {}", err);
        }
        self.programs = self.export_programs();
        log::info!("Loaded {} programs", self.programs.len());
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

impl Message for QueryChannelsMessage {
    type Result = Result<Vec<MirakurunChannel>, Error>;
}

impl Handler<QueryChannelsMessage> for Epg {
    type Result = Result<Vec<MirakurunChannel>, Error>;

    fn handle(
        &mut self,
        _: QueryChannelsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("Handle QueryChannelMessage");
        let channels = self.channels.iter()
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

// query services

struct QueryServicesMessage;

impl Message for QueryServicesMessage {
    type Result = Result<Vec<EpgService>, Error>;
}

impl Handler<QueryServicesMessage> for Epg {
    type Result = Result<Vec<EpgService>, Error>;

    fn handle(
        &mut self,
        _: QueryServicesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("Handle QueryServicesMessage");
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
        log::debug!("Handle QueryClockMessage");
        self.clocks.get(&msg.triple).cloned().ok_or(Error::ClockNotSynced)
    }
}

// query programs

struct QueryProgramsMessage;

impl Message for QueryProgramsMessage {
    type Result = Result<Vec<MirakurunProgram>, Error>;
}

impl Handler<QueryProgramsMessage> for Epg {
    type Result = Result<Vec<MirakurunProgram>, Error>;

    fn handle(
        &mut self,
        _: QueryProgramsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("Handle QueryProgramsMessage");
        Ok(self.programs.values().cloned().collect())
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
    type Result = Result<MirakurunProgram, Error>;
}

impl Handler<QueryProgramMessage> for Epg {
    type Result = Result<MirakurunProgram, Error>;

    fn handle(
        &mut self,
        msg: QueryProgramMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        match msg {
            QueryProgramMessage::ByNidSidEid { nid, sid, eid } => {
                let tsid = self.services
                    .iter()
                    .find(|sv| sv.nid == nid && sv.sid == sid)
                    .map(|sv| sv.tsid)
                    .ok_or(Error::ProgramNotFound)?;
                self.programs
                    .get(&EventQuad::new(nid, tsid, sid, eid))
                    .cloned()
                    .ok_or(Error::ProgramNotFound)
            }
        }
    }
}

// update services

struct UpdateServicesMessage {
    services: Vec<EpgService>,
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
        log::debug!("Handle UpdateServicesMessage");
        self.update_services(msg.services);
    }
}

// update clocks

struct UpdateClocksMessage {
    clocks: HashMap<ServiceTriple, Clock>,
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
        log::debug!("Handle UpdateClocksMessage");
        self.update_clocks(msg.clocks);
    }
}

// update schedules

struct UpdateSchedulesMessage {
    sections: Vec<EitSection>,
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
        log::debug!("Handle UpdateSchedulesMessage");
        self.update_schedules(msg.sections);
    }
}

// flush schedules

struct FlushSchedulesMessage;

impl Message for FlushSchedulesMessage {
    type Result = ();
}

impl Handler<FlushSchedulesMessage> for Epg {
    type Result = ();

    fn handle(
        &mut self,
        _: FlushSchedulesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("Handle FlushSchedulesMessage");
        self.flush_schedules();
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
}

impl EpgSchedule {
    fn new(triple: ServiceTriple) -> EpgSchedule {
        EpgSchedule {
            service_triple: triple,
            tables: Default::default(),
            overnight_events: Vec::new(),
            updated_at: Jst::now(),
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

    fn collect_epg_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventQuad, MirakurunProgram>) {
        for event in self.overnight_events.iter() {
            let quad = EventQuad::from((triple, EventId::from(event.event_id)));
            programs
                .entry(quad)
                .or_insert(MirakurunProgram::new(quad))
                .update(event);
        }
        for table in self.tables.iter() {
            if let Some(table) = table {
                table.collect_epg_programs(triple, programs)
            }
        }
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

    fn collect_epg_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventQuad, MirakurunProgram>) {
        for segment in self.segments.iter() {
            segment.collect_epg_programs(triple, programs)
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

    fn collect_epg_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventQuad, MirakurunProgram>) {
        for section in self.sections.iter() {
            if let Some(section) = section {
                section.collect_epg_programs(triple, programs)
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

    fn collect_epg_programs(
        &self,
        triple: ServiceTriple,
        programs: &mut HashMap<EventQuad, MirakurunProgram>) {
        for event in self.events.iter() {
            let quad = EventQuad::from((triple, EventId::from(event.event_id)));
            programs
                .entry(quad)
                .or_insert(MirakurunProgram::new(quad))
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
#[serde(rename_all = "camelCase")]
pub struct EitSection {
    original_network_id: NetworkId,
    transport_stream_id: TransportStreamId,
    service_id: ServiceId,
    table_id: u16,
    section_number: u8,
    last_section_number: u8,
    segment_last_section_number: u8,
    version_number: u8,
    events: Vec<EitEvent>,
}

impl EitSection {
    fn table_index(&self) -> usize {
        self.table_id as usize - 0x50
    }

    fn segment_index(&self) -> usize {
        self.section_number as usize / 8
    }

    fn section_index(&self) -> usize {
        self.section_number as usize % 8
    }

    fn last_section_index(&self) -> usize {
        self.segment_last_section_number as usize % 8
    }

    fn service_triple(&self) -> ServiceTriple {
        (self.original_network_id,
         self.transport_stream_id,
         self.service_id).into()
    }
}

#[derive(Clone)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EitEvent {
    event_id: EventId,
    #[serde(with = "serde_jst")]
    start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    duration: Duration,
    scrambled: bool,
    descriptors: Vec<EitDescriptor>,
}

impl EitEvent {
    fn end_time(&self) -> DateTime<Jst> {
        self.start_time + self.duration
    }

    fn is_overnight_event(&self, midnight: DateTime<Jst>) -> bool {
        self.start_time < midnight && self.end_time() > midnight
    }
}

#[derive(Clone)]
#[derive(Deserialize, Serialize)]
#[serde(tag = "$type")]
enum EitDescriptor {
    #[serde(rename_all = "camelCase")]
    ShortEvent {
        event_name: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Component {
        stream_content: u8,
        component_type: u8,
    },
    #[serde(rename_all = "camelCase")]
    AudioComponent {
        component_type: u8,
        sampling_rate: u8,
    },
    #[serde(rename_all = "camelCase")]
    Content {
        nibbles: Vec<(u8, u8, u8, u8)>,
    },
    #[serde(rename_all = "camelCase")]
    ExtendedEvent {
        items: Vec<(String, String)>,
    },
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

impl MirakurunProgram {
    fn update(&mut self, event: &EitEvent) {
        self.start_at = event.start_time.clone();
        self.duration = event.duration.clone();
        self.is_free = !event.scrambled;
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
