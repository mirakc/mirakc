use std::fmt;

use chrono::DateTime;
use chrono::Duration;
use chrono_jst::jst::Jst;
use chrono_jst::serde::duration_milliseconds_option;
use chrono_jst::serde::ts_milliseconds_option;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use smallstr::SmallString;
use utoipa::ToSchema;

use crate::config::ResourceConfig;
use crate::epg::AudioComponentDescriptor;
use crate::epg::ComponentDescriptor;
use crate::epg::EpgChannel;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::SeriesDescriptor;
use crate::tuner::TunerSubscriptionId;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, ToSchema)]
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

// Historical Note
// ---------------
// Initially, ServiceTriple and ProgramQuad types wrap an u64 value and its
// value is serialized as a Number in JSON.  However, this causes problems when
// parsing a serialized JSON with tools such as jq which uses `double` (C++) in
// representation of a Number.  As a result, a value outside -2^53..2^53 cannot
// be represented properly.
//
// * https://github.com/stedolan/jq/issues/369
// * https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Number/MAX_SAFE_INTEGER
//
// Afterward, we changed definitions of those types in order to avoid the
// problems.

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[cfg_attr(test, derive(Debug))]
pub struct ServiceTriple(NetworkId, TransportStreamId, ServiceId);

impl ServiceTriple {
    pub fn new(nid: NetworkId, tsid: TransportStreamId, sid: ServiceId) -> Self {
        ServiceTriple(nid, tsid, sid)
    }

    pub fn nid(&self) -> NetworkId {
        self.0
    }

    pub fn tsid(&self) -> TransportStreamId {
        self.1
    }

    pub fn sid(&self) -> ServiceId {
        self.2
    }

    fn value(&self) -> u64 {
        // The least 16 bits are reserved for eid.
        (self.0.value() as u64) << 48
            | (self.1.value() as u64) << 32
            | (self.2.value() as u64) << 16
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

impl From<ProgramQuad> for ServiceTriple {
    fn from(quad: ProgramQuad) -> Self {
        ServiceTriple::new(quad.nid(), quad.tsid(), quad.sid())
    }
}

impl Into<(NetworkId, ServiceId)> for ServiceTriple {
    fn into(self) -> (NetworkId, ServiceId) {
        (self.nid(), self.sid())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct ProgramQuad(NetworkId, TransportStreamId, ServiceId, EventId);

impl ProgramQuad {
    pub fn new(nid: NetworkId, tsid: TransportStreamId, sid: ServiceId, eid: EventId) -> Self {
        ProgramQuad(nid, tsid, sid, eid)
    }

    pub fn nid(&self) -> NetworkId {
        self.0
    }

    pub fn tsid(&self) -> TransportStreamId {
        self.1
    }

    pub fn sid(&self) -> ServiceId {
        self.2
    }

    pub fn eid(&self) -> EventId {
        self.3
    }

    fn value(&self) -> u64 {
        (self.0.value() as u64) << 48
            | (self.1.value() as u64) << 32
            | (self.2.value() as u64) << 16
            | (self.3.value() as u64)
    }
}

impl fmt::Display for ProgramQuad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016X}", self.value())
    }
}

impl From<u64> for ProgramQuad {
    fn from(v: u64) -> Self {
        ProgramQuad(
            ((v >> 48) as u16).into(),
            ((v >> 32) as u16).into(),
            ((v >> 16) as u16).into(),
            ((v >> 0) as u16).into(),
        )
    }
}

impl From<(ServiceTriple, EventId)> for ProgramQuad {
    fn from(tuple: (ServiceTriple, EventId)) -> ProgramQuad {
        ProgramQuad::new(tuple.0.nid(), tuple.0.tsid(), tuple.0.sid(), tuple.1)
    }
}

impl From<(NetworkId, TransportStreamId, ServiceId, EventId)> for ProgramQuad {
    fn from(quad: (NetworkId, TransportStreamId, ServiceId, EventId)) -> ProgramQuad {
        ProgramQuad::new(quad.0, quad.1, quad.2, quad.3)
    }
}

impl Into<(NetworkId, ServiceId, EventId)> for ProgramQuad {
    fn into(self) -> (NetworkId, ServiceId, EventId) {
        (self.nid(), self.sid(), self.eid())
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

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
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

#[derive(Clone, PartialEq)]
pub enum TunerUserInfo {
    Job { name: String },
    OnairProgramTracker(String),
    Recorder { name: String },
    Tracker { stream_id: TunerSubscriptionId },
    Web { id: String, agent: Option<String> },
}

impl TunerUserInfo {
    fn get_mirakurun_model(&self) -> (String, Option<String>) {
        match self {
            Self::Job { name } => (format!("job:{}", name), None),
            Self::OnairProgramTracker(name) => (format!("onair-program-tracker:{name}"), None),
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
            Self::OnairProgramTracker(name) => write!(f, "OnairProgramTracker({name})"),
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

impl From<u64> for MirakurunServiceId {
    fn from(value: u64) -> Self {
        MirakurunServiceId(value)
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

impl From<ProgramQuad> for MirakurunServiceId {
    fn from(quad: ProgramQuad) -> Self {
        Self::new(quad.nid(), quad.sid())
    }
}

impl From<MirakurunProgramId> for MirakurunServiceId {
    fn from(id: MirakurunProgramId) -> Self {
        Self::new(id.nid(), id.sid())
    }
}

impl Into<(NetworkId, ServiceId)> for MirakurunServiceId {
    fn into(self) -> (NetworkId, ServiceId) {
        (self.nid(), self.sid())
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
        write!(f, "{}", self.0)
    }
}

impl From<ProgramQuad> for MirakurunProgramId {
    fn from(quad: ProgramQuad) -> Self {
        MirakurunProgramId::new(quad.nid(), quad.sid(), quad.eid())
    }
}

impl Into<(NetworkId, ServiceId, EventId)> for MirakurunProgramId {
    fn into(self) -> (NetworkId, ServiceId, EventId) {
        (self.nid(), self.sid(), self.eid())
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "Tuner")]
pub struct MirakurunTuner {
    /// The index of the tuner defined in `config.yml`.
    pub index: usize,

    /// The name of the tuner defined in `config.yml`.
    pub name: String,

    /// Channel types supported by the tuner.
    #[serde(rename = "types")]
    pub channel_types: Vec<ChannelType>,

    /// A command to use getting a media stream from the tuner.
    pub command: Option<String>,

    /// PID of a process to run the command.
    pub pid: Option<u32>,

    /// Users of the tuner.
    #[schema(inline)]
    pub users: Vec<MirakurunTunerUser>,

    /// Always `true`.
    pub is_available: bool,

    /// Always `false`.
    pub is_remote: bool,

    /// `true` if the tuner is free, `false` otherwise.
    pub is_free: bool,

    /// `false` if the tuner is free, `true` otherwise.
    pub is_using: bool,

    /// Always `false`.
    pub is_fault: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunTunerUser {
    /// User ID.
    pub id: String,

    /// User-Agent string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,

    /// Priority.
    pub priority: i32,
    // url, disableDecoder, streamSetting and streamInfo properties are not supported.
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "Channel")]
pub struct MirakurunChannel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    pub name: String,
    #[schema(inline)]
    pub services: Vec<MirakurunChannelService>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunChannelService {
    #[schema(value_type = u64)]
    pub id: MirakurunServiceId,
    #[schema(value_type = u16)]
    pub service_id: ServiceId,
    #[schema(value_type = u16)]
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    #[schema(value_type = u16)]
    pub network_id: NetworkId,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "Service")]
pub struct MirakurunService {
    #[schema(value_type = u64)]
    pub id: MirakurunServiceId,
    #[schema(value_type = u16)]
    pub service_id: ServiceId,
    #[schema(value_type = u16)]
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    #[schema(value_type = u16)]
    pub network_id: NetworkId,
    #[serde(rename = "type")]
    pub service_type: u16,
    #[serde(default)]
    pub logo_id: i16,
    #[serde(default)]
    pub remote_control_key_id: u16,
    pub name: String,
    #[schema(inline)]
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

#[derive(Debug, Serialize, ToSchema)]
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

// Don't use MirakurunProgram for saving information.
// See MirakurunProgramRelatedItem::get_type().
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "Program")]
pub struct MirakurunProgram {
    #[schema(value_type = u64)]
    pub id: MirakurunProgramId,
    #[schema(value_type = u16)]
    pub event_id: EventId,
    #[schema(value_type = u16)]
    pub service_id: ServiceId,
    #[schema(value_type = u16)]
    pub transport_stream_id: TransportStreamId, // incompatible with Mirakurun
    #[schema(value_type = u16)]
    pub network_id: NetworkId,
    #[serde(with = "ts_milliseconds_option")]
    #[schema(value_type = i64)]
    pub start_at: Option<DateTime<Jst>>,
    #[serde(with = "duration_milliseconds_option")]
    #[schema(value_type = i64)]
    pub duration: Option<Duration>,
    pub is_free: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub extended: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub video: Option<MirakurunProgramVideo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub audio: Option<MirakurunProgramAudio>, // for backward compatibility
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(inline)]
    pub audios: Vec<MirakurunProgramAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub genres: Option<Vec<EpgGenre>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(inline)]
    pub series: Option<MirakurunProgramSeries>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(inline)]
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

#[derive(Clone, Debug, Serialize, ToSchema)]
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

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramAudio {
    component_type: u8,
    is_main: bool,
    sampling_rate: i32,
    #[schema(value_type = Vec<String>)]
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
        let mut langs = vec![audio.language_code.into()];
        if let Some(lang) = audio.language_code2 {
            langs.push(lang.into());
        }
        Self::new(
            audio.component_type,
            audio.main_component_flag,
            Self::convert_sampling_rate(audio.sampling_rate),
            langs,
        )
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
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

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MirakurunProgramRelatedItem {
    #[serde(rename = "type")]
    group_type: &'static str,
    #[schema(value_type = Option<u16>)]
    network_id: Option<NetworkId>,
    #[schema(value_type = u16)]
    service_id: ServiceId,
    #[schema(value_type = u16)]
    event_id: EventId,
}

impl MirakurunProgramRelatedItem {
    fn get_type(group_type: u8) -> &'static str {
        match group_type {
            1 => "shared",
            // Don't use MirakurunProgram for saving information.
            // Information is lost as you can see below.
            2 | 4 => "relay",
            _ => "movement",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct MirakurunLangCode(SmallString<[u8; 3]>); // ISO 639-3 language code

impl From<u32> for MirakurunLangCode {
    fn from(value: u32) -> Self {
        // See LanguageCodeToText() in LibISDB.
        let c0 = ((value >> 16) & 0xFF) as u8;
        let c1 = ((value >> 8) & 0xFF) as u8;
        let c2 = ((value >> 0) & 0xFF) as u8;
        match SmallString::from_buf([c0, c1, c2]) {
            Ok(lang) => MirakurunLangCode(lang),
            Err(_) => {
                tracing::warn!("Invalid lang code, replace it with `und`");
                MirakurunLangCode(SmallString::from_str("und"))
            }
        }
    }
}

// events
pub mod events {
    use super::*;
    use crate::recording::RecordingFailedReason;

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct EpgProgramsUpdated {
        pub service_id: MirakurunServiceId,
    }

    impl EpgProgramsUpdated {
        pub const fn name() -> &'static str {
            "epg.programs-updated"
        }
    }

    impl From<crate::epg::ProgramsUpdated> for EpgProgramsUpdated {
        fn from(msg: crate::epg::ProgramsUpdated) -> Self {
            EpgProgramsUpdated {
                service_id: msg.service_triple.into(),
            }
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RecordingStarted {
        pub program_id: MirakurunProgramId,
    }

    impl RecordingStarted {
        pub const fn name() -> &'static str {
            "recording.started"
        }
    }

    impl From<crate::recording::RecordingStarted> for RecordingStarted {
        fn from(msg: crate::recording::RecordingStarted) -> Self {
            RecordingStarted {
                program_id: msg.program_quad.into(),
            }
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RecordingStopped {
        pub program_id: MirakurunProgramId,
    }

    impl RecordingStopped {
        pub const fn name() -> &'static str {
            "recording.stopped"
        }
    }

    impl From<crate::recording::RecordingStopped> for RecordingStopped {
        fn from(msg: crate::recording::RecordingStopped) -> Self {
            RecordingStopped {
                program_id: msg.program_quad.into(),
            }
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RecordingFailed {
        pub program_id: MirakurunProgramId,
        pub reason: RecordingFailedReason,
    }

    impl RecordingFailed {
        pub const fn name() -> &'static str {
            "recording.failed"
        }
    }

    impl From<crate::recording::RecordingFailed> for RecordingFailed {
        fn from(msg: crate::recording::RecordingFailed) -> Self {
            RecordingFailed {
                program_id: msg.program_quad.into(),
                reason: msg.reason,
            }
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RecordingRescheduled {
        pub program_id: MirakurunProgramId,
    }

    impl RecordingRescheduled {
        pub const fn name() -> &'static str {
            "recording.rescheduled"
        }
    }

    impl From<crate::recording::RecordingRescheduled> for RecordingRescheduled {
        fn from(msg: crate::recording::RecordingRescheduled) -> Self {
            RecordingRescheduled {
                program_id: msg.program_quad.into(),
            }
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct OnairProgramChanged {
        pub service_id: MirakurunServiceId,
    }

    impl OnairProgramChanged {
        pub const fn name() -> &'static str {
            "onair.program-changed"
        }
    }

    impl From<crate::onair::OnairProgramChanged> for OnairProgramChanged {
        fn from(msg: crate::onair::OnairProgramChanged) -> Self {
            OnairProgramChanged {
                service_id: msg.service_triple.into(),
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

    impl From<(u16, u16, u16, u16)> for ProgramQuad {
        fn from(quad: (u16, u16, u16, u16)) -> ProgramQuad {
            ProgramQuad::new(quad.0.into(), quad.1.into(), quad.2.into(), quad.3.into())
        }
    }

    impl From<(u16, u16)> for MirakurunServiceId {
        fn from(tuple: (u16, u16)) -> Self {
            MirakurunServiceId::new(tuple.0.into(), tuple.1.into())
        }
    }

    impl From<(u16, u16, u16)> for MirakurunProgramId {
        fn from(triple: (u16, u16, u16)) -> Self {
            MirakurunProgramId::new(triple.0.into(), triple.1.into(), triple.2.into())
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
