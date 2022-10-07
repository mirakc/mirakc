use chrono::DateTime;
use chrono::Duration;
use serde::Deserialize;
use serde::Serialize;

use crate::datetime_ext::serde_jst;
use crate::datetime_ext::serde_duration_in_millis;
use crate::datetime_ext::Jst;
use crate::models::EventId;
use crate::models::NetworkId;
use crate::models::ServiceId;
use crate::models::ServiceTriple;
use crate::models::TransportStreamId;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EitSection {
    pub original_network_id: NetworkId,
    pub transport_stream_id: TransportStreamId,
    pub service_id: ServiceId,
    pub table_id: u16,
    pub section_number: u8,
    pub last_section_number: u8,
    pub segment_last_section_number: u8,
    pub version_number: u8,
    pub events: Vec<EitEvent>,
}

impl EitSection {
    pub fn is_valid(&self) -> bool {
        match self.table_id {
            0x50 | 0x51 | 0x58 | 0x59 => true,
            _ => false,
        }
    }

    pub fn is_basic(&self) -> bool {
        match self.table_id {
            0x50..=0x57 => true,
            0x58..=0x5F => false,
            _ => panic!("Invalid table_id"),
        }
    }

    pub fn table_index(&self) -> usize {
        self.table_id as usize - 0x50
    }

    pub fn segment_index(&self) -> usize {
        self.section_number as usize / 8
    }

    pub fn section_index(&self) -> usize {
        self.section_number as usize % 8
    }

    pub fn last_section_index(&self) -> usize {
        self.segment_last_section_number as usize % 8
    }

    pub fn service_triple(&self) -> ServiceTriple {
        (
            self.original_network_id,
            self.transport_stream_id,
            self.service_id,
        )
            .into()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EitEvent {
    pub event_id: EventId,
    #[serde(with = "serde_jst")]
    pub start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    pub duration: Duration,
    pub scrambled: bool,
    pub descriptors: Vec<EitDescriptor>,
}

impl EitEvent {
    pub fn end_time(&self) -> DateTime<Jst> {
        self.start_time + self.duration
    }

    pub fn is_overnight_event(&self, midnight: DateTime<Jst>) -> bool {
        self.start_time < midnight && self.end_time() > midnight
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "$type")]
pub enum EitDescriptor {
    #[serde(rename_all = "camelCase")]
    ShortEvent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Component(ComponentDescriptor),
    AudioComponent(AudioComponentDescriptor),
    #[serde(rename_all = "camelCase")]
    Content {
        nibbles: Vec<(u8, u8, u8, u8)>,
    },
    Series(SeriesDescriptor),
    EventGroup(EventGroupDescriptor),
    #[serde(rename_all = "camelCase")]
    ExtendedEvent {
        items: Vec<(String, String)>,
    },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDescriptor {
    pub stream_content: u8,
    pub component_type: u8,
    pub component_tag: u8,
    pub language_code: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioComponentDescriptor {
    pub stream_content: u8,
    pub component_type: u8,
    pub component_tag: u8,
    pub simulcast_group_tag: u8,
    pub es_multi_lingual_flag: bool,
    pub main_component_flag: bool,
    pub quality_indicator: u8,
    pub sampling_rate: u8,
    pub language_code: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_code2: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDescriptor {
    pub series_id: u16,
    pub repeat_label: u8,
    pub program_pattern: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_date: Option<i64>,
    pub episode_number: u16,
    pub last_episode_number: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_name: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventGroupDescriptor {
    pub group_type: u8,
    pub events: Vec<EventGroupEvent>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventGroupEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_network_id: Option<NetworkId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_stream_id: Option<TransportStreamId>,
    pub service_id: ServiceId,
    pub event_id: EventId,
}
