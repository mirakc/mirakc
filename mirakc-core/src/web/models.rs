use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use chrono::DateTime;
use chrono::Duration;
use serde::Deserialize;
use serde::Serialize;

use crate::command_util::CommandPipelineProcessModel;
use crate::datetime_ext::serde_duration_in_millis;
use crate::datetime_ext::serde_jst;
use crate::datetime_ext::Jst;
use crate::models::ChannelType;
use crate::models::MirakurunProgram;
use crate::models::MirakurunProgramId;
use crate::models::MirakurunService;
use crate::models::ServiceId;
use crate::models::TimeshiftRecordId;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::models::TunerUserPriority;
use crate::recording::Record;
use crate::timeshift::TimeshiftRecordModel;
use crate::timeshift::TimeshiftRecorderModel;

#[derive(Serialize)]
pub(in crate::web) struct Version {
    pub current: &'static str,
    pub latest: &'static str,
}

#[derive(Serialize)]
pub(in crate::web) struct Status {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingSchedule {
    pub program: MirakurunProgram,
    pub content_path: PathBuf,
    pub priority: i32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pre_filters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post_filters: Vec<String>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    pub tags: HashSet<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingScheduleInput {
    pub program_id: MirakurunProgramId,
    pub content_path: PathBuf,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub tags: HashSet<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingRecorder {
    pub program: MirakurunProgram,
    pub content_path: PathBuf,
    pub priority: i32,
    pub pipeline: Vec<WebProcessModel>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    pub tags: HashSet<String>,
    #[serde(with = "serde_jst")]
    pub start_time: DateTime<Jst>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingRecord {
    pub id: String,
    pub program: MirakurunProgram,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pre_filters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post_filters: Vec<String>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    pub tags: HashSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_size: Option<u64>,
}

impl From<Record> for WebRecordingRecord {
    fn from(record: Record) -> Self {
        let content_size = record
            .content_path
            .metadata()
            .ok()
            .map(|metadata| metadata.len());
        let (content_path, content_type) = if record.content_path.exists() {
            (Some(record.content_path), Some(record.content_type))
        } else {
            (None, None)
        };
        WebRecordingRecord {
            id: record.program.record_id(),
            program: record.program.into(),
            pre_filters: record.pre_filters,
            post_filters: record.post_filters,
            tags: record.tags,
            content_path,
            content_type,
            content_size,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebTimeshiftRecorder {
    pub name: String,
    pub service: MirakurunService,
    #[serde(with = "serde_jst")]
    pub start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    pub duration: Duration,
    pub pipeline: Vec<WebProcessModel>,
    pub recording: bool,
}

impl From<TimeshiftRecorderModel> for WebTimeshiftRecorder {
    fn from(model: TimeshiftRecorderModel) -> Self {
        Self {
            name: model.name,
            service: model.service.into(),
            start_time: model.start_time.clone(),
            duration: model.end_time - model.start_time,
            pipeline: model
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            recording: model.recording,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebProcessModel {
    pub command: String,
    pub pid: Option<u32>,
}

impl From<CommandPipelineProcessModel> for WebProcessModel {
    fn from(model: CommandPipelineProcessModel) -> Self {
        Self {
            command: model.command,
            pid: model.pid,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebTimeshiftRecord {
    pub id: TimeshiftRecordId,
    pub program: MirakurunProgram,
    #[serde(with = "serde_jst")]
    pub start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    pub duration: Duration,
    pub size: u64,
    pub recording: bool,
}

impl From<TimeshiftRecordModel> for WebTimeshiftRecord {
    fn from(model: TimeshiftRecordModel) -> Self {
        Self {
            id: model.id,
            program: model.program.into(),
            start_time: model.start_time.clone(),
            duration: model.end_time - model.start_time,
            size: model.size,
            recording: model.recording,
        }
    }
}

// extractors

#[derive(Deserialize)]
pub(in crate::web) struct ChannelPath {
    pub channel_type: ChannelType,
    pub channel: String,
}

#[derive(Deserialize)]
pub(in crate::web) struct ChannelServicePath {
    pub channel_type: ChannelType,
    pub channel: String,
    pub sid: ServiceId,
}

#[derive(Deserialize)]
pub(in crate::web) struct TimeshiftRecordPath {
    pub recorder: String,
    pub id: TimeshiftRecordId,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(in crate::web) struct FilterSetting {
    #[serde(default = "FilterSetting::default_decode")]
    #[serde(deserialize_with = "FilterSetting::deserialize_stream_decode_query")]
    pub decode: bool, // default: true
    #[serde(default)]
    pub pre_filters: Vec<String>, // default: empty
    #[serde(default)]
    pub post_filters: Vec<String>, // default: empty
}

impl FilterSetting {
    fn default_decode() -> bool {
        true
    }

    fn deserialize_stream_decode_query<'de, D>(deserializer: D) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "0" || s == "false" {
            return Ok(false);
        }
        if s == "1" || s == "true" {
            return Ok(true);
        }
        Err(serde::de::Error::custom(
            "The value of the decode query must be 0, 1, false or true",
        ))
    }
}

#[derive(Deserialize)]
pub(in crate::web) struct IptvEpgQuery {
    #[serde(default = "IptvEpgQuery::default_days")]
    pub days: u8,
}

impl IptvEpgQuery {
    fn default_days() -> u8 {
        3
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for TunerUser
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::ConnectInfo;

        fn ms_since_unix_epoch() -> u128 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|dur| dur.as_millis())
                .unwrap_or(0)
        }

        let id = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.to_string())
            .unwrap_or_else(|| format!("unix:{}", ms_since_unix_epoch()));

        let agent = parts
            .headers
            .get_all(USER_AGENT)
            .iter()
            .last()
            .map(|value| value.to_str().ok().map_or(String::new(), |s| s.to_string()));

        let info = TunerUserInfo::Web { id, agent };

        let priority = parts
            .headers
            .get_all(super::X_MIRAKURUN_TUNER_USER_ID)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.parse::<i32>().ok())
            .max()
            .map(|value| value.max(0))
            .map(TunerUserPriority::from)
            .unwrap_or_default();

        Ok(TunerUser { info, priority })
    }
}
