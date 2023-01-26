use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::serde::duration_milliseconds;
use chrono_jst::serde::ts_milliseconds;
use chrono_jst::Jst;
use path_dedot::ParseDot;
use serde::Deserialize;
use serde::Serialize;
use utoipa::IntoParams;
use utoipa::ToSchema;

use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::error::Error;
use crate::models::ChannelType;
use crate::models::MirakurunProgram;
use crate::models::MirakurunService;
use crate::models::ProgramId;
use crate::models::ServiceId;
use crate::models::Sid;
use crate::models::TimeshiftRecordId;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::models::TunerUserPriority;
use crate::onair::OnairProgram;
use crate::recording;
use crate::recording::RecordingOptions;
use crate::recording::RecordingScheduleState;
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
    pub state: RecordingScheduleState,
    pub program: MirakurunProgram,
    pub options: RecordingOptions,
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

impl From<recording::RecordingSchedule> for WebRecordingSchedule {
    fn from(value: recording::RecordingSchedule) -> Self {
        WebRecordingSchedule {
            state: value.state,
            program: value.program.as_ref().clone().into(),
            options: value.options,
            tags: value.tags,
        }
    }
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingScheduleInput {
    #[schema(value_type = u64)]
    pub program_id: ProgramId,
    pub options: RecordingOptions,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

impl WebRecordingScheduleInput {
    pub fn validate(&self, config: &Config) -> Result<(), Error> {
        if self.options.content_path.is_absolute() {
            let err = Error::InvalidPath;
            tracing::error!(
                %err,
                input.options.content_path = ?self.options.content_path
            );
            return Err(err);
        }

        let basedir = config.recording.basedir.as_ref().unwrap();
        if !basedir
            .join(&self.options.content_path)
            .parse_dot()?
            .starts_with(basedir)
        {
            let err = Error::InvalidPath;
            tracing::error!(
                %err,
                input.options.content_path = ?self.options.content_path
            );
            return Err(err);
        }

        Ok(())
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebRecordingRecorder {
    #[schema(value_type = u64)]
    pub program_id: ProgramId,
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    pub started_at: DateTime<Jst>,
    pub pipeline: Vec<WebProcessModel>,
}

impl From<recording::RecorderModel> for WebRecordingRecorder {
    fn from(value: recording::RecorderModel) -> Self {
        WebRecordingRecorder {
            program_id: value.program_id.into(),
            started_at: value.started_at,
            pipeline: value
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(in crate::web) struct WebTimeshiftRecorder {
    pub name: String,
    pub service: MirakurunService,
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    pub start_time: DateTime<Jst>,
    #[serde(with = "duration_milliseconds")]
    #[schema(value_type = i64)]
    pub duration: Duration,
    pub pipeline: Vec<WebProcessModel>,
    pub recording: bool,
    #[schema(value_type = Option<u32>)]
    pub current_record_id: Option<TimeshiftRecordId>,
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
            current_record_id: model.current_record_id,
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
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    pub start_time: DateTime<Jst>,
    #[serde(with = "duration_milliseconds")]
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

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebOnairProgram {
    pub service_id: ServiceId,
    pub current: Option<MirakurunProgram>,
    pub next: Option<MirakurunProgram>,
}

impl From<(ServiceId, OnairProgram)> for WebOnairProgram {
    fn from((service_id, data): (ServiceId, OnairProgram)) -> Self {
        Self {
            service_id: service_id.into(),
            current: data.current.map(|p| p.as_ref().clone().into()),
            next: data.next.map(|p| p.as_ref().clone().into()),
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
    pub sid: Sid,
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
