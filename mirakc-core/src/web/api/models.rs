use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use chrono::DateTime;
use chrono::Duration;
use serde::Deserialize;
use serde::Serialize;
use utoipa::IntoParams;
use utoipa::ToSchema;

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
use crate::recording;
use crate::timeshift::TimeshiftRecordModel;
use crate::timeshift::TimeshiftRecorderModel;

#[derive(Serialize, ToSchema)]
pub(in crate::web) struct Version {
    /// Current version.
    pub current: &'static str,

    /// Same as `current`.
    pub latest: &'static str,
}

#[derive(Serialize, ToSchema)]
pub(in crate::web) struct Status {}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordingSchedule")]
pub(in crate::web) struct WebRecordingSchedule {
    #[schema(value_type = u64)]
    pub program_id: MirakurunProgramId,
    #[serde(with = "serde_jst")]
    #[schema(value_type = u64)]
    pub start_at: DateTime<Jst>,
    #[serde(with = "serde_jst")]
    #[schema(value_type = u64)]
    pub end_at: DateTime<Jst>,
    #[schema(value_type = String)]
    pub content_path: PathBuf,
    pub priority: i32,
    pub pre_filters: Vec<String>,
    pub post_filters: Vec<String>,
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

impl From<Arc<recording::Schedule>> for WebRecordingSchedule {
    fn from(value: Arc<recording::Schedule>) -> Self {
        WebRecordingSchedule {
            program_id: value.program_quad.into(),
            start_at: value.start_at,
            end_at: value.end_at,
            content_path: value.content_path.clone(),
            priority: value.priority,
            pre_filters: value.pre_filters.clone(),
            post_filters: value.post_filters.clone(),
            tags: value.tags.clone(),
        }
    }
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingScheduleInput {
    #[schema(value_type = u64)]
    pub program_id: MirakurunProgramId,
    #[schema(value_type = String)]
    pub content_path: PathBuf,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub pre_filters: Vec<String>,
    #[serde(default)]
    pub post_filters: Vec<String>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingRecorder {
    #[schema(value_type = u64)]
    pub program_id: MirakurunProgramId,
    #[serde(with = "serde_jst")]
    #[schema(value_type = u64)]
    pub started_at: DateTime<Jst>,
    #[serde(with = "serde_jst")]
    #[schema(value_type = u64)]
    pub end_at: DateTime<Jst>,
    #[schema(value_type = String)]
    pub content_path: PathBuf,
    pub priority: i32,
    pub pipeline: Vec<WebProcessModel>,
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

impl From<recording::RecorderModel> for WebRecordingRecorder {
    fn from(value: recording::RecorderModel) -> Self {
        WebRecordingRecorder {
            program_id: value.schedule.program_quad.into(),
            started_at: value.started_at,
            end_at: value.schedule.end_at,
            content_path: value.schedule.content_path.clone(),
            priority: value.schedule.priority,
            pipeline: value
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            tags: value.schedule.tags.clone(),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebTimeshiftRecorder {
    pub name: String,
    pub service: MirakurunService,
    #[serde(with = "serde_jst")]
    #[schema(value_type = i64)]
    pub start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    #[schema(value_type = i64)]
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

#[derive(Serialize, ToSchema)]
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

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebTimeshiftRecord {
    #[schema(value_type = u32)]
    pub id: TimeshiftRecordId,
    pub program: MirakurunProgram,
    #[serde(with = "serde_jst")]
    #[schema(value_type = i64)]
    pub start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    #[schema(value_type = i64)]
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Path)]
pub(in crate::web) struct TimeshiftRecordPath {
    /// Timeshift recorder name.
    pub recorder: String,

    /// Timeshift record ID.
    #[param(value_type = u32)]
    pub id: TimeshiftRecordId,
}

#[derive(Debug, Deserialize, Serialize, IntoParams)]
#[serde(rename_all = "kebab-case")]
#[into_params(parameter_in = Query)]
pub(in crate::web) struct FilterSetting {
    /// `0` or `false` disables decoding.
    ///
    /// The stream will be decoded by default if a decoder is specified in the
    /// `config.yml`.
    #[serde(default = "FilterSetting::default_decode")]
    #[serde(deserialize_with = "FilterSetting::deserialize_stream_decode_query")]
    pub decode: bool, // default: true

    /// Pre-filters.
    #[serde(default)]
    pub pre_filters: Vec<String>, // default: empty

    /// Post-filters.
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
