use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::PathBuf;

use actix::prelude::*;
use chrono::{Datelike, DateTime, Duration};
use humantime;
use indexmap::IndexMap;
use log;
use mustache;
use serde::{Deserialize, Serialize};

use crate::config::{Config, ChannelConfig};
use crate::datetime_ext::*;
use crate::error::Error;
use crate::messages::{OpenTunerBy, OpenTunerMessage, UpdateEpgMessage};
use crate::models::*;
use crate::resource_manager;
use crate::tuner::{TunerOutput, TunerUser};

pub fn start(arbiter: &Arbiter, config: &Config) {
    let epg = Epg::new(config);
    arbiter.exec_fn(|| { epg.start(); });
}

type EpgServices = Vec<EpgService>;

struct Epg {
    cache_dir: PathBuf,
    scan_services: String,
    collect_eits: String,
    channels: Vec<ChannelConfig>,
    schedules: HashMap<EpgScheduleId, EpgSchedule>,
    updated_at: DateTime<Jst>,
    max_elapsed: Option<Duration>,
}

impl Epg {
    #[inline]
    fn scan_services_time_limit(channel_type: ChannelType) -> Duration {
        match channel_type {
            ChannelType::GR => Duration::seconds(5),
            ChannelType::BS => Duration::seconds(20),
            _ => Duration::minutes(30),
        }
    }

    #[inline]
    fn collect_eits_time_limit(channel_type: ChannelType) -> Duration {
        match channel_type {
            ChannelType::GR => Duration::minutes(1) + Duration::seconds(10),
            ChannelType::BS => Duration::minutes(6) + Duration::seconds(30),
            _ => Duration::minutes(10),
        }
    }

    #[inline]
    fn format_duration(duration: Duration) -> humantime::FormattedDuration {
        humantime::format_duration(duration.to_std().unwrap())
    }

    fn new(config: &Config) -> Self {
        let channels = config.channels
            .iter()
            .filter(|config| !config.disabled)
            .cloned()
            .collect();

        Epg {
            cache_dir: PathBuf::from(&config.epg_cache_dir),
            scan_services: config.tools.scan_services.clone(),
            collect_eits: config.tools.collect_eits.clone(),
            channels,
            schedules: HashMap::new(),
            updated_at: Jst::now(),
            max_elapsed: None,
        }
    }

    fn run_later(&mut self, ctx: &mut Context<Self>, duration: Duration) {
        log::info!("Run after {}", Self::format_duration(duration));
        ctx.run_later(duration.to_std().unwrap(), Self::run);
    }

    fn run(&mut self, ctx: &mut Context<Self>) {
        let now = Jst::now();

        let remaining = now.date().succ().and_hms(0, 0, 0) - now;
        if remaining < self.estimate_time() {
            log::info!("This task may not be completed this day");
            log::info!("Postpone the task until next day \
                        for keeping consistency of EPG data");
            self.run_later(ctx, remaining + Duration::seconds(10));
            return;
        }

        if now.date().day() == self.updated_at.date().day() + 1 {
            // Save overnight events.  Because the overnight events will be lost
            // in `epg.update_schedules()`.
            self.save_overnight_events();
        }

        actix::fut::ok::<_, Error, Self>(())
            .and_then(|_, epg, _ctx| {
                epg.scan_services()
            })
            .and_then(|services, epg, _ctx| {
                epg.update_schedules(services)
            })
            .and_then(|_, epg, _ctx| {
                match epg.save_epg_data() {
                    Ok(_) => actix::fut::ok(()),
                    Err(err) => actix::fut::err(err),
                }
            })
            .and_then(|_, epg, _ctx| {
                epg.send_update_epg_message();
                actix::fut::ok(())
            })
            .then(move |result, epg, ctx| {
                let elapsed = Jst::now() - now;
                let duration = match result {
                    Ok(_) => {
                        log::info!("Done, {} elapsed",
                                   Self::format_duration(elapsed));
                        epg.update_max_elapsed(elapsed);
                        Duration::minutes(15)
                    }
                    Err(err) => {
                        log::error!("Failed: {}", err);
                        Duration::minutes(5)
                    }
                };
                epg.run_later(ctx, duration);
                actix::fut::ok(())
            })
            .spawn(ctx);
    }

    fn estimate_time(&self) -> Duration {
        match self.max_elapsed {
            Some(max_elapsed) => max_elapsed + Duration::seconds(30),
            None => Duration::hours(1),
        }
    }

    fn update_max_elapsed(&mut self, elapsed: Duration) {
        let do_update = match self.max_elapsed {
            Some(max_elapsed) if elapsed <= max_elapsed => false,
            _ => true,
        };
        if do_update {
            log::info!("Updated the max elapsed time");
            self.max_elapsed = Some(elapsed);
        }
    }

    fn scan_services(
        &mut self,
    ) -> impl ActorFuture<Item = EpgServices, Error = Error, Actor = Self> {
        let channels = self.collect_channels_for_scanning_services();
        let stream = futures::stream::iter_ok::<_, Error>(channels);
        let stream = actix::fut::wrap_stream::<_, Self>(stream);

        stream
            .map(|ch, _epg, _ctx| {
                log::info!("Scanning services in {}...", ch.name);
                ch
            })
            .and_then(|ch, _epg, _ctx| {
                let msg = OpenTunerMessage {
                    by: ch.clone().into(),
                    user: TunerUser::background("epg".to_string()),
                    duration: Some(
                        Self::scan_services_time_limit(ch.channel_type)),
                    preprocess: false,
                    postprocess: false,
                };

                let req = resource_manager::open_tuner(msg)
                    .map(|output| (ch, output));

                actix::fut::wrap_future(req)
            })
            .and_then(|(ch, output), epg, _ctx| {
                let cmd = match Self::make_command(&epg.scan_services, &ch) {
                    Ok(cmd) => cmd,
                    Err(err) => return actix::fut::err(err),
                };
                match output.pipe(&cmd) {
                    Ok(output) => actix::fut::ok((ch, output)),
                    Err(err) => actix::fut::err(Error::from(err)),
                }
            })
            .and_then(|(ch, output), _epg, _ctx| {
                let reader = BufReader::new(output);
                match serde_json::from_reader::<_, Vec<TsService>>(reader) {
                    Ok(services) => {
                        log::info!("Found {} services in {}",
                                   services.len(), ch.name);
                        let mut epg_services = Vec::new();
                        for service in services.iter() {
                            epg_services.push(EpgService::from((&ch, service)));
                        }
                        actix::fut::ok(Some(epg_services))
                    }
                    Err(_) => {
                        log::warn!("No service.  Maybe, the broadcast service \
                                    has been suspended.");
                        actix::fut::ok(None)
                    }
                }
            })
            .fold(Vec::new(), |mut result, services, _epg, _ctx| {
                match services {
                    Some(mut services) => result.append(&mut services),
                    None => (),
                }
                actix::fut::ok::<_, Error, Self>(result)
            })
    }

    fn collect_channels_for_scanning_services(&self) -> Vec<EpgChannel> {
        self.channels
            .iter()
            .cloned()
            .map(EpgChannel::from)
            .collect()
    }

    fn update_schedules(
        &mut self,
        services: EpgServices,
    ) -> impl ActorFuture<Item = (), Error = Error, Actor = Self> {
        self.prepare_schedules(&services);

        let channels = self.collect_channels_for_collecting_programs(&services);
        let stream = futures::stream::iter_ok::<_, Error>(channels);
        let stream = actix::fut::wrap_stream::<_, Self>(stream);

        stream
            .map(|(nid, ch), _epg, _ctx| {
                log::info!("Updating schedules broadcasted on {}...", ch.name);
                (nid, ch)
            })
            .and_then(|(nid, ch), _epg, _ctx| {
                let msg = OpenTunerMessage {
                    by: ch.clone().into(),
                    user: TunerUser::background("epg".to_string()),
                    duration: Some(
                        Self::collect_eits_time_limit(ch.channel_type)),
                    preprocess: false,
                    postprocess: false,
                };

                let req = resource_manager::open_tuner(msg)
                    .map(move |output| (nid, ch, output));

                actix::fut::wrap_future(req)
            })
            .and_then(|(_nid, ch, output), epg, _ctx| {
                let cmd = match Self::make_command(&epg.collect_eits, &ch) {
                    Ok(cmd) => cmd,
                    Err(err) => return actix::fut::err(err),
                };
                match output.pipe(&cmd) {
                    Ok(output) => actix::fut::ok(output),
                    Err(err) => actix::fut::err(Error::from(err)),
                }
            })
            .and_then(|output, epg, _ctx| {
                match epg.update_tables(output) {
                    Ok(_) => actix::fut::ok(()),
                    Err(err) => actix::fut::err(err),
                }
            })
            .finish()
    }

    fn prepare_schedules(&mut self, services: &[EpgService]) {
        for service in services {
            let id = EpgScheduleId::from(
                (service.nid, service.tsid, service.sid));
            self.schedules.entry(id).or_insert(EpgSchedule::new(&service));
        }
    }

    fn collect_channels_for_collecting_programs(
        &self,
        services: &[EpgService],
    ) -> HashMap<u16, EpgChannel> {
        let mut map: HashMap<u16, EpgChannel> = HashMap::new();
        for sv in services.iter() {
            map.entry(sv.nid).and_modify(|ch| {
                ch.excluded_services.extend(&sv.channel.excluded_services);
            }).or_insert(sv.channel.clone());
        }
        map
    }

    fn update_tables(&mut self, output: TunerOutput) -> Result<(), Error> {
        // TODO: use async/await
        let mut reader = BufReader::new(output);
        let mut json = String::new();
        let mut num_sections = 0;
        while reader.read_line(&mut json)? > 0 {
            let section = serde_json::from_str::<EitSection>(&json)?;
            let sched_id = section.epg_schedule_id();
            self.schedules.entry(sched_id).and_modify(|sched| {
                sched.update(section);
            });
            json.clear();
            num_sections += 1;
        }
        log::debug!("Collected {} EIT sections", num_sections);
        return Ok(());
    }

    fn save_overnight_events(&mut self) {
        for sched in self.schedules.values_mut() {
            sched.save_overnight_events();
        }
        log::info!("Saved overnight events");
    }

    fn load_epg_data(&mut self) -> Result<(), Error> {
        self.load_schedules()?;
        Ok(())
    }

    fn load_schedules(&mut self) -> Result<(), Error> {
        let json_path = self.cache_dir.join("schedules.json");
        log::info!("Loading schedules from {}...", json_path.display());
        let reader = BufReader::new(File::open(json_path)?);
        self.schedules = serde_json::from_reader(reader)?;
        log::info!("Loaded");
        Ok(())
    }

    fn save_epg_data(&self) -> Result<(), Error> {
        self.save_schedules()?;
        Ok(())
    }

    fn save_schedules(&self) -> Result<(), Error> {
        let json_path = self.cache_dir.join("schedules.json");
        log::info!("Saving schedules into {}...", json_path.display());
        let writer = BufWriter::new(File::create(json_path)?);
        serde_json::to_writer(writer, &self.schedules)?;
        log::info!("Saved");
        Ok(())
    }

    fn send_update_epg_message(&self) {
        let msg = UpdateEpgMessage {
            services: self.collect_epg_services(),
            programs: self.collect_epg_programs(),
        };
        resource_manager::update_epg(msg);
    }

    fn collect_epg_services(&self) -> Vec<ServiceModel> {
        let mut services: Vec<ServiceModel> = Vec::new();
        for sched in self.schedules.values() {
            sched.collect_epg_service(&mut services);
        }
        log::info!("{} services have been collected", services.len());
        services
    }

    fn collect_epg_programs(&self) -> HashMap<u64, ProgramModel> {
        let mut programs: HashMap<u64, ProgramModel> = HashMap::new();
        for (&sched_id, sched) in self.schedules.iter() {
            sched.collect_epg_programs(sched_id, &mut programs);
        }
        log::info!("{} programs have been collected", programs.len());
        programs
    }

    fn make_command(src: &str, channel: &EpgChannel) -> Result<String, Error> {
        let template = mustache::compile_str(src)?;
        let data = mustache::MapBuilder::new()
            .insert("xsids", &channel.excluded_services)?.build();
        Ok(template.render_data_to_string(&data)?)
    }
}

impl Actor for Epg {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::info!("Started");
        match self.load_epg_data() {
            Ok(_) => self.send_update_epg_message(),
            Err(err) => log::error!("Failed to load EPG data: {}", err),
        }
        ctx.run_later(Duration::minutes(0).to_std().unwrap(), Self::run);
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::info!("Stopped");
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
struct EpgScheduleId(u64);

impl EpgScheduleId {
    #[inline]
    fn nid(&self) -> u16 {
        ((self.0 >> 32) & 0xFFFF) as u16
    }

    #[inline]
    fn tsid(&self) -> u16 {
        ((self.0 >> 16) & 0xFFFF) as u16
    }

    #[inline]
    fn sid(&self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }
}

impl fmt::Display for EpgScheduleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:012X}", self.0)
    }
}

impl From<(u16, u16, u16)> for EpgScheduleId {
    #[inline]
    fn from(triple: (u16, u16, u16)) -> Self {
        EpgScheduleId(
            (triple.0 as u64) << 32 |
            (triple.1 as u64) << 16 |
            (triple.2 as u64)       )
    }
}

#[derive(Deserialize, Serialize)]
struct EpgSchedule {
    service: EpgService,
    tables: [Option<Box<EpgTable>>; 32],
    overnight_events: Vec<EitEvent>,
}

impl EpgSchedule {
    fn new(service: &EpgService) -> EpgSchedule {
        EpgSchedule {
            service: service.clone(),
            tables: Default::default(),
            overnight_events: Vec::new(),
        }
    }

    fn update(&mut self, section: EitSection) {
        let i = section.table_index();
        if self.tables[i].is_none() {
            self.tables[i] = Some(Box::new(EpgTable::default()));
        }
        self.tables[i].as_mut().unwrap().update(section);
    }

    fn save_overnight_events(&mut self) {
        let mut events = Vec::new();
        for i in (0..32).step_by(8) {
            if let Some(ref table) = self.tables[i] {
                events = table.collect_overnight_events(events);
            }
        }
        log::debug!("Saved {} overnight events of service#{:04X}",
                    events.len(), self.service.sid);
        self.overnight_events = events;
    }

    fn collect_epg_service(&self, services: &mut Vec<ServiceModel>) {
        services.push(self.service.clone().into());
    }

    fn collect_epg_programs(
        &self,
        sched_id: EpgScheduleId,
        programs: &mut HashMap<u64, ProgramModel>) {
        let sid = sched_id.sid();
        let nid = sched_id.nid();
        for event in self.overnight_events.iter() {
            let eid = event.event_id;
            programs
                .entry(ProgramModel::make_id(eid, sid, nid))
                .or_insert(ProgramModel::new(eid, sid, nid))
                .update(event);
        }
        for table in self.tables.iter() {
            if let Some(table) = table {
                table.collect_epg_programs(sched_id, programs)
            }
        }
    }
}

#[derive(Default)]
#[derive(Deserialize, Serialize)]
struct EpgTable {
    segments: [EpgSegment; 32],
}

impl EpgTable {
    fn update(&mut self, section: EitSection) {
        let i = section.segment_index();
        self.segments[i].update(section);
    }

    fn collect_overnight_events(
        &self,
        mut events: Vec<EitEvent>
    ) -> Vec<EitEvent> {
        for i in 0..8 {
            events = self.segments[i].collect_overnight_events(events);
        }
        events
    }

    fn collect_epg_programs(
        &self,
        sched_id: EpgScheduleId,
        programs: &mut HashMap<u64, ProgramModel>) {
        for segment in self.segments.iter() {
            segment.collect_epg_programs(sched_id, programs)
        }
    }
}

#[derive(Default)]
#[derive(Deserialize, Serialize)]
struct EpgSegment {
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

    fn collect_overnight_events(&self, events: Vec<EitEvent>) -> Vec<EitEvent> {
        self.sections
            .iter()
            .filter(|section| section.is_some())
            .map(|section| section.as_ref().unwrap())
            .fold(events, |events_, section| {
                section.collect_overnight_events(events_)
            })
    }

    fn collect_epg_programs(
        &self,
        sched_id: EpgScheduleId,
        programs: &mut HashMap<u64, ProgramModel>) {
        for section in self.sections.iter() {
            if let Some(section) = section {
                section.collect_epg_programs(sched_id, programs)
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
struct EpgSection {
    version: u8,
    events: Vec<EitEvent>,
}

impl EpgSection {
    fn collect_overnight_events(&self, events: Vec<EitEvent>) -> Vec<EitEvent> {
        self.events
            .iter()
            .fold(events, |mut events_, event| {
                if event.is_overnight_event() {
                    events_.push(event.clone());
                }
                events_
            })
    }

    fn collect_epg_programs(
        &self,
        sched_id: EpgScheduleId,
        programs: &mut HashMap<u64, ProgramModel>) {
        let sid = sched_id.sid();
        let nid = sched_id.nid();
        for event in self.events.iter() {
            let eid = event.event_id;
            programs
                .entry(ProgramModel::make_id(eid, sid, nid))
                .or_insert(ProgramModel::new(eid, sid, nid))
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
struct EitSection {
    original_network_id: u16,
    transport_stream_id: u16,
    service_id: u16,
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

    fn epg_schedule_id(&self) -> EpgScheduleId {
        (self.original_network_id,
         self.transport_stream_id,
         self.service_id).into()
    }
}

#[derive(Clone)]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EitEvent {
    event_id: u16,
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

    fn is_overnight_event(&self) -> bool {
        self.start_time.date().succ().day() == self.end_time().day()
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
    pub excluded_services: Vec<u16>,
}

impl From<ChannelConfig> for EpgChannel {
    fn from(config: ChannelConfig) -> Self {
        EpgChannel {
            name: config.name,
            channel_type: config.channel_type,
            channel: config.channel,
            excluded_services: config.excluded_services,
        }
    }
}

impl Into<OpenTunerBy> for EpgChannel {
    fn into(self) -> OpenTunerBy {
        OpenTunerBy::Channel {
            channel_type: self.channel_type,
            channel: self.channel,
        }
    }
}

impl Into<ServiceChannelModel> for EpgChannel {
    fn into(self) -> ServiceChannelModel {
        ServiceChannelModel {
            channel_type: self.channel_type,
            channel: self.channel,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsService {
    nid: u16,
    tsid: u16,
    sid: u16,
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
    nid: u16,
    tsid: u16,
    sid: u16,
    #[serde(rename = "type")]
    service_type: u16,
    #[serde(default)]
    logo_id: i16,
    #[serde(default)]
    remote_control_key_id: u16,
    name: String,
    channel: EpgChannel,
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

impl Into<ServiceModel> for EpgService {
    fn into(self) -> ServiceModel {
        ServiceModel {
            id: ServiceModel::make_id(self.sid, self.nid),
            service_id: self.sid,
            network_id: self.nid,
            service_type: self.service_type,
            logo_id: self.logo_id,
            remote_control_key_id: self.remote_control_key_id,
            name: self.name,
            channel: self.channel.into(),
            has_logo_data: false,
        }
    }
}

impl ProgramModel {
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
    use chrono::TimeZone;

    fn create_epg_service(
        id: EpgScheduleId, channel_type: ChannelType) -> EpgService {
        EpgService {
            nid: id.nid(),
            tsid: id.tsid(),
            sid: id.sid(),
            service_type: 1,
            logo_id: 0,
            remote_control_key_id: 0,
            name: "Service".to_string(),
            channel: EpgChannel {
                name: "Ch".to_string(),
                channel_type,
                channel: "ch".to_string(),
                excluded_services: Vec::new(),
            }
        }
    }

    fn create_epg_schedule(
        id: EpgScheduleId, channel_type: ChannelType) -> EpgSchedule {
        let sv = create_epg_service(id, channel_type);
        EpgSchedule::new(&sv)
    }

    #[test]
    fn test_epg_schedule_update() {
        let id = EpgScheduleId::from((1, 2, 3));
        let mut sched = create_epg_schedule(id, ChannelType::GR);

        sched.update(EitSection {
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 3,
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
        let mut sched = create_epg_schedule_for_collect_overnight_events_test();
        sched.save_overnight_events();
        assert_eq!(sched.overnight_events.len(), 4);
    }

    #[test]
    fn test_epg_table_update() {
        let mut table: EpgTable = Default::default();

        table.update(EitSection {
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 3,
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
        let table = create_epg_table_for_collect_overnight_events_test();
        let events = table.collect_overnight_events(Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2);
    }

    #[test]
    fn test_epg_segment_update() {
        let mut segment: EpgSegment = Default::default();

        segment.update(EitSection {
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 3,
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
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 3,
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
        let segment = create_epg_segment_for_collect_overnight_events_test();
        let events = segment.collect_overnight_events(Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2);
    }

    #[test]
    fn test_epg_section_collect_overnight_events() {
        let section = create_epg_section_for_collect_overnight_events_test();
        let events = section.collect_overnight_events(Vec::new());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 2);
    }

    #[test]
    fn test_eit_event_is_overnight_event() {
        let event = EitEvent {
            event_id: 0,
            start_time: Jst.ymd(2019, 10, 13).and_hms(23, 59, 59),
            duration: Duration::seconds(1),
            scrambled: false,
            descriptors: Vec::new(),
        };
        assert!(event.is_overnight_event());

        let event = EitEvent {
            event_id: 0,
            start_time: Jst.ymd(2019, 10, 13).and_hms(23, 59, 58),
            duration: Duration::seconds(1),
            scrambled: false,
            descriptors: Vec::new(),
        };
        assert!(!event.is_overnight_event());
    }

    fn create_epg_schedule_for_collect_overnight_events_test() -> EpgSchedule {
        let id = EpgScheduleId::from((1, 2, 3));
        let mut sched = create_epg_schedule(id, ChannelType::GR);
        sched.tables[0] = Some(Box::new(
            create_epg_table_for_collect_overnight_events_test()));
        sched.tables[1] = Some(Box::new(
            create_epg_table_for_collect_overnight_events_test()));
        sched.tables[8] = Some(Box::new(
            create_epg_table_for_collect_overnight_events_test()));
        sched.tables[16] = Some(Box::new(
            create_epg_table_for_collect_overnight_events_test()));
        sched.tables[24] = Some(Box::new(
            create_epg_table_for_collect_overnight_events_test()));
        sched
    }

    fn create_epg_table_for_collect_overnight_events_test() -> EpgTable {
        let mut table = EpgTable::default();
        table.segments[7] =
            create_epg_segment_for_collect_overnight_events_test();
        table
    }

    fn create_epg_segment_for_collect_overnight_events_test() -> EpgSegment {
        let mut segment = EpgSegment::default();
        segment.sections[0] =
            Some(EpgSection { version: 1, events: Vec::new() });
        segment.sections[1] =
            Some(create_epg_section_for_collect_overnight_events_test());
        segment
    }

    fn create_epg_section_for_collect_overnight_events_test() -> EpgSection {
        EpgSection {
            version: 1,
            events: vec![
                EitEvent {
                    event_id: 1,
                    start_time: Jst.ymd(2019, 10, 13).and_hms(23, 0, 0),
                    duration: Duration::minutes(30),
                    scrambled: false,
                    descriptors: Vec::new(),
                },
                EitEvent {
                    event_id: 2,
                    start_time: Jst.ymd(2019, 10, 13).and_hms(23, 30, 0),
                    duration: Duration::hours(1),
                    scrambled: false,
                    descriptors: Vec::new(),
                },
            ]
        }
    }
}
