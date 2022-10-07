use std::fmt;

use chrono::{DateTime, Duration};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::config::ResourceConfig;
use crate::datetime_ext::{serde_duration_in_millis, serde_jst, Jst};
use crate::epg::{AudioComponentDescriptor, ComponentDescriptor};
use crate::epg::{EpgChannel, EpgProgram, EpgService};
use crate::epg::SeriesDescriptor;
use crate::tuner::TunerSubscriptionId;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub enum ChannelType {
    GR,
    BS,
    CS,
    SKY,
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ChannelType::GR => write!(f, "GR"),
            ChannelType::BS => write!(f, "BS"),
            ChannelType::CS => write!(f, "CS"),
            ChannelType::SKY => write!(f, "SKY"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct NetworkId(u16);

impl NetworkId {
    pub fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NID#{:04X}", self.0)
    }
}

impl From<u16> for NetworkId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct TransportStreamId(u16);

impl TransportStreamId {
    pub fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for TransportStreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TSID#{:04X}", self.0)
    }
}

impl From<u16> for TransportStreamId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct ServiceId(u16);

impl ServiceId {
    pub fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SID#{:04X}", self.0)
    }
}

impl From<u16> for ServiceId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct EventId(u16);

impl EventId {
    pub fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EID#{:04X}", self.0)
    }
}

impl From<u16> for EventId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(Debug))]
pub struct ServiceTriple(u64);

impl ServiceTriple {
    pub fn new(nid: NetworkId, tsid: TransportStreamId, sid: ServiceId) -> Self {
        ServiceTriple(
            (nid.value() as u64) << 48 | (tsid.value() as u64) << 32 | (sid.value() as u64) << 16,
        )
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    pub fn nid(&self) -> NetworkId {
        NetworkId(((self.value() >> 48) & 0xFFFF) as u16)
    }

    pub fn tsid(&self) -> TransportStreamId {
        TransportStreamId(((self.value() >> 32) & 0xFFFF) as u16)
    }

    pub fn sid(&self) -> ServiceId {
        ServiceId(((self.value() >> 16) & 0xFFFF) as u16)
    }
}

impl fmt::Display for ServiceTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016X}", self.value())
    }
}

impl From<(NetworkId, TransportStreamId, ServiceId)> for ServiceTriple {
    fn from(triple: (NetworkId, TransportStreamId, ServiceId)) -> ServiceTriple {
        ServiceTriple::new(triple.0, triple.1, triple.2)
    }
}

impl From<EventQuad> for ServiceTriple {
    fn from(quad: EventQuad) -> Self {
        ServiceTriple::new(quad.nid(), quad.tsid(), quad.sid())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct EventQuad(u64);

impl EventQuad {
    pub fn new(nid: NetworkId, tsid: TransportStreamId, sid: ServiceId, eid: EventId) -> Self {
        EventQuad(
            (nid.value() as u64) << 48
                | (tsid.value() as u64) << 32
                | (sid.value() as u64) << 16
                | (eid.value() as u64),
        )
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    pub fn nid(&self) -> NetworkId {
        NetworkId(((self.value() >> 48) & 0xFFFF) as u16)
    }

    pub fn tsid(&self) -> TransportStreamId {
        TransportStreamId(((self.value() >> 32) & 0xFFFF) as u16)
    }

    pub fn sid(&self) -> ServiceId {
        ServiceId(((self.value() >> 16) & 0xFFFF) as u16)
    }

    pub fn eid(&self) -> EventId {
        EventId((self.value() & 0xFFFF) as u16)
    }
}

impl fmt::Display for EventQuad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016X}", self.value())
    }
}

impl From<(ServiceTriple, EventId)> for EventQuad {
    fn from(tuple: (ServiceTriple, EventId)) -> EventQuad {
        EventQuad::new(tuple.0.nid(), tuple.0.tsid(), tuple.0.sid(), tuple.1)
    }
}

impl From<(NetworkId, TransportStreamId, ServiceId, EventId)> for EventQuad {
    fn from(quad: (NetworkId, TransportStreamId, ServiceId, EventId)) -> EventQuad {
        EventQuad::new(quad.0, quad.1, quad.2, quad.3)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clock {
    // PID of PCR packets
    pub pid: u16,
    // 27MHz, 42bits PCR value corresponding to the time
    pub pcr: i64,
    // UNIX time in ms
    pub time: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EpgGenre {
    pub lv1: u8,
    pub lv2: u8,
    pub un1: u8,
    pub un2: u8,
}

impl EpgGenre {
    pub fn new((lv1, lv2, un1, un2): (u8, u8, u8, u8)) -> EpgGenre {
        EpgGenre { lv1, lv2, un1, un2 }
    }
}

// user

#[derive(Clone)]
pub enum TunerUserInfo {
    Job { name: String },
    Recorder { name: String },
    Tracker { stream_id: TunerSubscriptionId },
    Web { id: String, agent: Option<String> },
}

impl TunerUserInfo {
    fn get_mirakurun_model(&self) -> (String, Option<String>) {
        match self {
            Self::Job { name } => (format!("job:{}", name), None),
            Self::Recorder { name } => (format!("recorder:{}", name), None),
            Self::Tracker { stream_id } => (format!("tracker:{}", stream_id), None),
            Self::Web { id, agent } => (id.clone(), agent.clone()),
        }
    }
}

impl fmt::Display for TunerUserInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Job { name } => write!(f, "Job({})", name),
            Self::Recorder { name } => write!(f, "Recorder({})", name),
            Self::Tracker { stream_id } => write!(f, "Tracker({})", stream_id),
            Self::Web { id, agent: None } => write!(f, r#"Web(id="{}")"#, id),
            Self::Web {
                id,
                agent: Some(agent),
            } => write!(f, r#"Web(id="{}" agent="{}")"#, id, agent),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct TunerUserPriority(i32);

impl TunerUserPriority {
    pub const MIN: i32 = -128;
    pub const MAX: i32 = 128;
    pub const GRAB: TunerUserPriority = TunerUserPriority(Self::MAX);

    pub fn is_grab(&self) -> bool {
        *self == Self::GRAB
    }
}

impl From<i32> for TunerUserPriority {
    fn from(value: i32) -> Self {
        TunerUserPriority(value.max(Self::MIN).min(Self::MAX))
    }
}

impl fmt::Display for TunerUserPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone)]
pub struct TunerUser {
    pub info: TunerUserInfo,
    pub priority: TunerUserPriority,
}

impl TunerUser {
    pub fn get_mirakurun_model(&self) -> MirakurunTunerUser {
        let (id, agent) = self.info.get_mirakurun_model();
        MirakurunTunerUser {
            id,
            agent,
            priority: self.priority.0,
        }
    }
}

impl fmt::Display for TunerUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (priority={})", self.info, self.priority)
    }
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct TimeshiftRecordId(u32);

impl TimeshiftRecordId {
    pub fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for TimeshiftRecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Record#{:08X}", self.0)
    }
}

// TimeshiftRecordId is created from a 64-bit UNIX timestamp.  Lower 32-bit is used as an
// identifier of a timeshift record.  It's ensured that a record ID is unique for timeshift
// recording within 136 years.
impl From<i64> for TimeshiftRecordId {
    fn from(timestamp: i64) -> Self {
        TimeshiftRecordId(timestamp as u32)
    }
}

impl From<u32> for TimeshiftRecordId {
    fn from(value: u32) -> Self {
        TimeshiftRecordId(value)
    }
}

// Mirakurun-compatible models

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct MirakurunServiceId(u64);

impl MirakurunServiceId {
    const MAGIC_NUMBER: u64 = 100_000;

    fn new(nid: NetworkId, sid: ServiceId) -> Self {
        // An unique identifier compatible with Mirakurun.
        // See src/Mirakurun/ServiceItem.ts in Chinachu/Mirakurun.
        MirakurunServiceId(nid.value() as u64 * Self::MAGIC_NUMBER + sid.value() as u64)
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    pub fn nid(&self) -> NetworkId {
        NetworkId::from((self.0 / Self::MAGIC_NUMBER) as u16)
    }

    pub fn sid(&self) -> ServiceId {
        ServiceId::from((self.0 % Self::MAGIC_NUMBER) as u16)
    }
}

impl fmt::Display for MirakurunServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({} {})", self.0, self.nid(), self.sid())
    }
}

impl From<(NetworkId, ServiceId)> for MirakurunServiceId {
    fn from((nid, sid): (NetworkId, ServiceId)) -> Self {
        Self::new(nid, sid)
    }
}

impl From<ServiceTriple> for MirakurunServiceId {
    fn from(triple: ServiceTriple) -> Self {
        Self::new(triple.nid(), triple.sid())
    }
}

impl From<EventQuad> for MirakurunServiceId {
    fn from(quad: EventQuad) -> Self {
        Self::new(quad.nid(), quad.sid())
    }
}

impl From<MirakurunProgramId> for MirakurunServiceId {
    fn from(id: MirakurunProgramId) -> Self {
        Self::new(id.nid(), id.sid())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct MirakurunProgramId(u64);

impl MirakurunProgramId {
    const MAGIC_NUMBER: u64 = 100_000;

    pub fn new(nid: NetworkId, sid: ServiceId, eid: EventId) -> Self {
        // An unique identifier compatible with Mirakurun.
        // See src/Mirakurun/ProgramItem.ts#L28 in Chinachu/Mirakurun.
        MirakurunProgramId(
            nid.value() as u64 * Self::MAGIC_NUMBER * Self::MAGIC_NUMBER
                + sid.value() as u64 * Self::MAGIC_NUMBER
                + eid.value() as u64,
        )
    }

    pub fn nid(&self) -> NetworkId {
        NetworkId::from((self.0 / (Self::MAGIC_NUMBER * Self::MAGIC_NUMBER)) as u16)
    }

    pub fn sid(&self) -> ServiceId {
        ServiceId::from(((self.0 / Self::MAGIC_NUMBER) % Self::MAGIC_NUMBER) as u16)
    }

    pub fn eid(&self) -> EventId {
        EventId::from((self.0 % Self::MAGIC_NUMBER) as u16)
    }
}

impl fmt::Display for MirakurunProgramId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} {} {})",
            self.0,
            self.nid(),
            self.sid(),
            self.eid()
        )
    }
}

impl From<EventQuad> for MirakurunProgramId {
    fn from(quad: EventQuad) -> Self {
        MirakurunProgramId::new(quad.nid(), quad.sid(), quad.eid())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunTuner {
    pub index: usize,
    pub name: String,
    #[serde(rename = "types")]
    pub channel_types: Vec<ChannelType>,
    pub command: Option<String>,
    pub pid: Option<u32>,
    pub users: Vec<MirakurunTunerUser>,
    pub is_available: bool,
    pub is_remote: bool,
    pub is_free: bool,
    pub is_using: bool,
    pub is_fault: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunTunerUser {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub priority: i32,
    // url, disableDecoder, streamSetting and streamInfo properties are not supported.
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunChannel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    pub name: String,
    pub services: Vec<MirakurunChannelService>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunChannelService {
    pub id: MirakurunServiceId,
    pub service_id: ServiceId,
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    pub network_id: NetworkId,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunService {
    pub id: MirakurunServiceId,
    pub service_id: ServiceId,
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    pub network_id: NetworkId,
    #[serde(rename = "type")]
    pub service_type: u16,
    #[serde(default)]
    pub logo_id: i16,
    #[serde(default)]
    pub remote_control_key_id: u16,
    pub name: String,
    pub channel: MirakurunServiceChannel,
    pub has_logo_data: bool,
}

impl MirakurunService {
    pub fn check_logo_existence(&mut self, config: &ResourceConfig) {
        let triple = ServiceTriple::new(self.network_id, self.transport_stream_id, self.service_id);
        self.has_logo_data = config.logos.contains_key(&triple)
    }
}

impl From<EpgService> for MirakurunService {
    fn from(sv: EpgService) -> Self {
        Self {
            id: (sv.nid, sv.sid).into(),
            service_id: sv.sid,
            transport_stream_id: sv.tsid,
            network_id: sv.nid,
            service_type: sv.service_type,
            logo_id: sv.logo_id,
            remote_control_key_id: sv.remote_control_key_id,
            name: sv.name,
            channel: sv.channel.into(),
            has_logo_data: false,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunServiceChannel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
}

impl From<EpgChannel> for MirakurunServiceChannel {
    fn from(ch: EpgChannel) -> Self {
        Self {
            channel_type: ch.channel_type,
            channel: ch.channel,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgram {
    pub id: MirakurunProgramId,
    pub event_id: EventId,
    pub service_id: ServiceId,
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    pub network_id: NetworkId,
    #[serde(with = "serde_jst")]
    pub start_at: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    pub duration: Duration,
    pub is_free: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<MirakurunProgramVideo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<MirakurunProgramAudio>, // for backward compatibility
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audios: Vec<MirakurunProgramAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<Vec<EpgGenre>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<MirakurunProgramSeries>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_items: Vec<MirakurunProgramRelatedItem>,
}

impl From<EpgProgram> for MirakurunProgram {
    fn from(program: EpgProgram) -> Self {
        Self {
            id: program.quad.into(),
            event_id: program.quad.eid(),
            service_id: program.quad.sid(),
            transport_stream_id: program.quad.tsid(),
            network_id: program.quad.nid(),
            start_at: program.start_at,
            duration: program.duration,
            is_free: !program.scrambled,
            name: program.name,
            description: program.description,
            extended: program.extended,
            video: program.video.map(MirakurunProgramVideo::from),
            // Unkike Mirakurun, return properties of the main audio.
            // Mirakurun returns the last audio info in descriptors.
            // See src/Mirakurun/epg.ts#L373 in Chinachu/Mirakurun.
            audio: program
                .audios
                .values()
                .find(|audio| audio.main_component_flag)
                .cloned()
                .map(MirakurunProgramAudio::from),
            audios: program
                .audios
                .values()
                .cloned()
                .map(MirakurunProgramAudio::from)
                .collect(),
            genres: program.genres,
            series: program.series.map(MirakurunProgramSeries::from),
            related_items: program.event_group.map_or(vec![], |event_group| {
                event_group
                    .events
                    .iter()
                    .map(|event| MirakurunProgramRelatedItem {
                        group_type: MirakurunProgramRelatedItem::get_type(event_group.group_type),
                        network_id: event.original_network_id.clone(),
                        service_id: event.service_id,
                        event_id: event.event_id,
                    })
                    .collect()
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramVideo {
    #[serde(rename = "type")]
    video_type: Option<String>,
    resolution: Option<String>,
    stream_content: u8,
    component_type: u8,
}

impl MirakurunProgramVideo {
    fn new(stream_content: u8, component_type: u8) -> Self {
        Self {
            video_type: Self::get_video_type(stream_content),
            resolution: Self::get_resolution(component_type),
            stream_content,
            component_type,
        }
    }

    fn get_video_type(stream_content: u8) -> Option<String> {
        match stream_content {
            0x01 => Some("mpeg2".to_string()),
            0x05 => Some("h.264".to_string()),
            0x09 => Some("h.265".to_string()),
            _ => None,
        }
    }

    fn get_resolution(component_type: u8) -> Option<String> {
        match component_type {
            0x01..=0x04 => Some("480i".to_string()),
            0x83 => Some("4320p".to_string()),
            0x91..=0x94 => Some("2160p".to_string()),
            0xA1..=0xA4 => Some("480p".to_string()),
            0xB1..=0xB4 => Some("1080i".to_string()),
            0xC1..=0xC4 => Some("720p".to_string()),
            0xD1..=0xD4 => Some("240p".to_string()),
            0xE1..=0xE4 => Some("1080p".to_string()),
            0xF1..=0xF4 => Some("180p".to_string()),
            _ => None,
        }
    }
}

impl From<ComponentDescriptor> for MirakurunProgramVideo {
    fn from(video: ComponentDescriptor) -> Self {
        Self::new(video.stream_content, video.component_type)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramAudio {
    component_type: u8,
    is_main: bool,
    sampling_rate: i32,
    langs: Vec<MirakurunLangCode>,
}

impl MirakurunProgramAudio {
    fn new(
        component_type: u8,
        is_main: bool,
        sampling_rate: i32,
        langs: Vec<MirakurunLangCode>,
    ) -> Self {
        Self {
            component_type,
            is_main,
            sampling_rate,
            langs,
        }
    }

    fn convert_sampling_rate(sampling_rate: u8) -> i32 {
        match sampling_rate {
            1 => 16_000,
            2 => 22_050,
            3 => 24_000,
            5 => 32_000,
            6 => 44_100,
            7 => 48_000,
            _ => -1,
        }
    }
}

impl From<AudioComponentDescriptor> for MirakurunProgramAudio {
    fn from(audio: AudioComponentDescriptor) -> Self {
        let mut langs = vec![MirakurunLangCode(audio.language_code)];
        if let Some(lang) = audio.language_code2 {
            langs.push(MirakurunLangCode(lang));
        }
        Self::new(
            audio.component_type,
            audio.main_component_flag,
            Self::convert_sampling_rate(audio.sampling_rate),
            langs,
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramSeries {
    id: u16,
    repeat: u8,
    pattern: u8,
    expire_at: i64,
    episode: u16,
    last_episode: u16,
    name: String,
}

impl From<SeriesDescriptor> for MirakurunProgramSeries {
    fn from(series: SeriesDescriptor) -> Self {
        Self {
            id: series.series_id,
            repeat: series.repeat_label,
            pattern: series.program_pattern,
            expire_at: series.expire_date.map_or(-1, |unix_time_ms| unix_time_ms),
            episode: series.episode_number,
            last_episode: series.last_episode_number,
            name: series.series_name.unwrap_or("".to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramRelatedItem {
    #[serde(rename = "type")]
    group_type: &'static str,
    network_id: Option<NetworkId>,
    service_id: ServiceId,
    event_id: EventId,
}

impl MirakurunProgramRelatedItem {
    fn get_type(group_type: u8) -> &'static str {
        match group_type {
            1 => "shared",
            2 | 4 => "relay",
            _ => "movement",
        }
    }
}

#[derive(Clone, Debug)]
struct MirakurunLangCode(u32);

// See LanguageCodeToText() in LibISDB.
impl Serialize for MirakurunLangCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self.0 {
            0 => serializer.serialize_str(""),
            _ => {
                let char0 = (((self.0 >> 16) & 0xFF) as u8) as char;
                let char1 = (((self.0 >> 8) & 0xFF) as u8) as char;
                let char2 = (((self.0 >> 0) & 0xFF) as u8) as char;
                let lang: String = [char0, char1, char2].iter().collect();
                serializer.serialize_str(&lang)
            }
        }
    }
}

#[cfg(test)]
mod test_helper {
    use super::*;

    // Don't move the implementation outside the module.  That break the
    // type-safeness of integral identifiers like NetworkId.

    impl From<(u16, u16, u16)> for ServiceTriple {
        fn from(triple: (u16, u16, u16)) -> ServiceTriple {
            ServiceTriple::new(triple.0.into(), triple.1.into(), triple.2.into())
        }
    }

    impl From<(u16, u16, u16, u16)> for EventQuad {
        fn from(quad: (u16, u16, u16, u16)) -> EventQuad {
            EventQuad::new(quad.0.into(), quad.1.into(), quad.2.into(), quad.3.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_type() {
        assert!(serde_json::from_str::<ChannelType>(r#""WOWOW""#).is_err());

        assert_eq!(
            serde_json::from_str::<ChannelType>(r#""GR""#).unwrap(),
            ChannelType::GR
        );

        assert_eq!(
            serde_json::from_str::<Vec<ChannelType>>(r#"["GR", "BS", "CS", "SKY"]"#).unwrap(),
            vec![
                ChannelType::GR,
                ChannelType::BS,
                ChannelType::CS,
                ChannelType::SKY
            ]
        );
    }

    #[test]
    fn test_tuner_user_priority() {
        assert_eq!(TunerUserPriority::default(), 0.into());
        assert_eq!(TunerUserPriority::from(128), 128.into());
        assert_eq!(TunerUserPriority::from(129), 128.into());
        assert_eq!(TunerUserPriority::from(-128), (-128).into());
        assert_eq!(TunerUserPriority::from(-129), (-128).into());

        assert!(TunerUserPriority::from(128).is_grab());
    }

    #[test]
    fn test_mirakurun_service_id() {
        let nid = 1.into();
        let sid = 2.into();
        let id = MirakurunServiceId::new(nid, sid);
        assert_eq!(MirakurunServiceId(1_00002), id);
        assert_eq!(nid, id.nid());
        assert_eq!(sid, id.sid());
    }

    #[test]
    fn test_mirakurun_program_id() {
        let nid = 1.into();
        let sid = 2.into();
        let eid = 3.into();
        let id = MirakurunProgramId::new(nid, sid, eid);
        assert_eq!(MirakurunProgramId(1_00002_00003), id);
        assert_eq!(nid, id.nid());
        assert_eq!(sid, id.sid());
        assert_eq!(eid, id.eid());
    }
}
