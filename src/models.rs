use std::fmt;

use chrono::{DateTime, Duration, TimeZone};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::datetime_ext::{serde_jst, serde_duration_in_millis, Jst};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub enum ChannelType { GR, BS, CS, SKY }

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelModel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    pub name: String,
    pub services: Vec<ChannelServiceModel>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct MirakurunServiceId(u64);

impl MirakurunServiceId {
    #[inline]
    pub fn new(triple: ServiceTriple) -> Self {
        // An unique identifier compatible with Mirakurun.
        // See src/Mirakurun/ServiceItem.ts in Chinachu/Mirakurun.
        MirakurunServiceId(
            triple.nid().value() as u64 * 100_000
                + triple.sid().value() as u64)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelServiceModel {
    pub id: MirakurunServiceId,
    pub service_id: ServiceId,
    pub network_id: NetworkId,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceModel {
    pub id: MirakurunServiceId,
    pub service_id: ServiceId,
    pub network_id: NetworkId,
    #[serde(rename = "type")]
    pub service_type: u16,
    #[serde(default)]
    pub logo_id: i16,
    #[serde(default)]
    pub remote_control_key_id: u16,
    pub name: String,
    pub channel: ServiceChannelModel,
    pub has_logo_data: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceChannelModel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct NetworkId(u16);

impl NetworkId {
    pub fn value(&self) -> u16 { self.0 }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct TransportStreamId(u16);

impl TransportStreamId {
    pub fn value(&self) -> u16 { self.0 }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct ServiceId(u16);

impl ServiceId {
    pub fn value(&self) -> u16 { self.0 }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct EventId(u16);

impl EventId {
    pub fn value(&self) -> u16 { self.0 }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct ServiceTriple(u64);

impl ServiceTriple {
    #[inline]
    pub fn new(
        nid: NetworkId,
        tsid: TransportStreamId,
        sid: ServiceId,
    ) -> Self {
        ServiceTriple(
            (nid.value() as u64)  << 48 |
            (tsid.value() as u64) << 32 |
            (sid.value() as u64)  << 16 )
    }

    #[inline]
    pub fn value(&self) -> u64 {
        self.0
    }

    #[inline]
    pub fn nid(&self) -> NetworkId {
        NetworkId(((self.value() >> 48) & 0xFFFF) as u16)
    }

    #[inline]
    pub fn tsid(&self) -> TransportStreamId {
        TransportStreamId(((self.value() >> 32) & 0xFFFF) as u16)
    }

    #[inline]
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
    #[inline]
    fn from(
        triple: (NetworkId, TransportStreamId, ServiceId)
    ) -> ServiceTriple {
        ServiceTriple::new(triple.0, triple.1, triple.2)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct EventQuad(u64);

impl EventQuad {
    #[inline]
    pub fn new(
        nid: NetworkId,
        tsid: TransportStreamId,
        sid: ServiceId,
        eid: EventId
    ) -> Self {
        EventQuad(
            (nid.value() as u64)  << 48 |
            (tsid.value() as u64) << 32 |
            (sid.value() as u64)  << 16 |
            (eid.value() as u64)
        )
    }

    #[inline]
    pub fn value(&self) -> u64 {
        self.0
    }

    #[inline]
    pub fn nid(&self) -> NetworkId {
        NetworkId(((self.value() >> 48) & 0xFFFF) as u16)
    }

    #[inline]
    pub fn tsid(&self) -> TransportStreamId {
        TransportStreamId(((self.value() >> 32) & 0xFFFF) as u16)
    }

    #[inline]
    pub fn sid(&self) -> ServiceId {
        ServiceId(((self.value() >> 16) & 0xFFFF) as u16)
    }

    #[inline]
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
    #[inline]
    fn from(tuple: (ServiceTriple, EventId)) -> EventQuad {
        EventQuad::new(tuple.0.nid(), tuple.0.tsid(), tuple.0.sid(), tuple.1)
    }
}

impl From<(NetworkId, TransportStreamId, ServiceId, EventId)> for EventQuad {
    #[inline]
    fn from(
        quad: (NetworkId, TransportStreamId, ServiceId, EventId)
    ) -> EventQuad {
        EventQuad::new(quad.0, quad.1, quad.2, quad.3)
    }
}

impl Into<ServiceTriple> for EventQuad {
    #[inline]
    fn into(self) -> ServiceTriple {
        (self.nid(), self.tsid(), self.sid()).into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clock {
    // 27MHz, 42bits PCR value corresponding to the time
    pub pcr: i64,
    // UNIX time in ms
    pub time: i64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[derive(Deserialize, Serialize)]
pub struct MirakurunProgramId(u64);

impl MirakurunProgramId {
    #[inline]
    pub fn new(quad: EventQuad) -> Self {
        // An unique identifier compatible with Mirakurun.
        // See src/Mirakurun/ProgramItem.ts#L28 in Chinachu/Mirakurun.
        MirakurunProgramId(
            quad.nid().value() as u64 * 100_000 * 100_000
                + quad.sid().value() as u64 * 100_000
                + quad.eid().value() as u64)
    }

    pub fn value(&self) -> u64 { self.0 }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramModel {
    #[serde(skip)]  // for compatibility with Mirakurun
    pub event_quad: EventQuad,
    pub id: MirakurunProgramId,
    pub event_id: EventId,
    pub service_id: ServiceId,
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
    pub video: Option<EpgVideoInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<EpgAudioInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<Vec<EpgGenre>>,
}

impl ProgramModel {
    pub fn new(quad: EventQuad) -> ProgramModel {
        ProgramModel {
            event_quad: quad,
            id: MirakurunProgramId::new(quad),
            event_id: quad.eid(),
            service_id: quad.sid(),
            network_id: quad.nid(),
            start_at: Jst.timestamp(0, 0),
            duration: Duration::minutes(0),
            is_free: false,
            name: None,
            description: None,
            extended: None,
            video: None,
            audio: None,
            genres: None,
        }
    }

    #[inline]
    pub fn end_at(&self) -> DateTime<Jst> {
        self.start_at + self.duration
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgVideoInfo {
    #[serde(rename = "type")]
    pub video_type: Option<String>,
    pub resolution: Option<String>,
    pub stream_content: u8,
    pub component_type: u8,
}

impl EpgVideoInfo {
    pub fn new(stream_content: u8, component_type: u8) -> EpgVideoInfo {
        EpgVideoInfo {
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
            _    => None,
        }
    }

    fn get_resolution(component_type: u8) -> Option<String> {
        match component_type {
            0x01..=0x04 => Some("480i".to_string()),
            0x83        => Some("4320p".to_string()),
            0x91..=0x94 => Some("2160p".to_string()),
            0xA1..=0xA4 => Some("480p".to_string()),
            0xB1..=0xB4 => Some("1080i".to_string()),
            0xC1..=0xC4 => Some("720p".to_string()),
            0xD1..=0xD4 => Some("240p".to_string()),
            0xE1..=0xE4 => Some("1080p".to_string()),
            0xF1..=0xF4 => Some("180p".to_string()),
            _           => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgAudioInfo {
    pub sampling_rate: i32,
    pub component_type: u8,
}

impl EpgAudioInfo {
    pub fn new(component_type: u8, sampling_rate: u8) -> EpgAudioInfo {
        EpgAudioInfo {
            component_type,
            sampling_rate: Self::get_sampling_rate(sampling_rate),
        }
    }

    fn get_sampling_rate(sampling_rate: u8) -> i32 {
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerModel {
    pub index: usize,
    pub name: String,
    #[serde(rename = "types")]
    pub channel_types: Vec<ChannelType>,
    pub command: Option<String>,
    pub pid: Option<u32>,
    pub users: Vec<TunerUserModel>,
    pub is_available: bool,
    pub is_remote: bool,
    pub is_free: bool,
    pub is_using: bool,
    pub is_fault: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerUserModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub priority: i32,
}

#[cfg(test)]
mod test_helper {
    use super::*;

    // Don't move the implementation outside the module.  That break the
    // type-safeness of integral identifiers like NetworkId.

    impl From<u16> for NetworkId {
        fn from(value: u16) -> NetworkId { NetworkId(value) }
    }

    impl From<u16> for TransportStreamId {
        fn from(value: u16) -> TransportStreamId { TransportStreamId(value) }
    }

    impl From<u16> for ServiceId {
        fn from(value: u16) -> ServiceId { ServiceId(value) }
    }

    impl From<u16> for EventId {
        fn from(value: u16) -> EventId { EventId(value) }
    }

    impl From<(u16, u16, u16)> for ServiceTriple {
        fn from(triple: (u16, u16, u16)) -> ServiceTriple {
            ServiceTriple::new(
                triple.0.into(), triple.1.into(), triple.2.into())
        }
    }

    impl From<(u16, u16, u16, u16)> for EventQuad {
        fn from(quad: (u16, u16, u16, u16)) -> EventQuad {
            EventQuad::new(
                quad.0.into(), quad.1.into(), quad.2.into(), quad.3.into())
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
            ChannelType::GR);

        assert_eq!(
            serde_json::from_str::<Vec<ChannelType>>(
                r#"["GR", "BS", "CS", "SKY"]"#).unwrap(),
            vec![ChannelType::GR, ChannelType::BS,
                 ChannelType::CS, ChannelType::SKY]);
    }
}
