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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelServiceModel {
    pub id: u64,
    pub service_id: u16,
    pub network_id: u16,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceModel {
    pub id: u64,
    pub service_id: u16,
    pub network_id: u16,
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

impl ServiceModel {
    pub fn make_id(sid: u16, nid: u16) -> u64 {
        nid as u64 * 100_000 + sid as u64
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceChannelModel {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramModel {
    pub id: u64,
    pub event_id: u16,
    pub service_id: u16,
    pub network_id: u16,
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
    #[inline]
    pub fn make_id(eid: u16, sid: u16, nid: u16) -> u64 {
        // An unique identifier compatible with Mirakurun.
        // See src/Mirakurun/ProgramItem.ts#L28 in Chinachu/Mirakurun.
        nid as u64 * 100_000 * 100_000 + sid as u64 * 100_000 + eid as u64
    }

    pub fn new(eid: u16, sid: u16, onid: u16) -> ProgramModel {
        ProgramModel {
            id: Self::make_id(eid, sid, onid),
            event_id: eid,
            service_id: sid,
            network_id: onid,
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
