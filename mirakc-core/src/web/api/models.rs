use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::FromRequestParts;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::Jst;
use chrono_jst::serde::duration_milliseconds;
use chrono_jst::serde::duration_milliseconds_option;
use chrono_jst::serde::ts_milliseconds;
use chrono_jst::serde::ts_milliseconds_option;
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
use crate::recording::Record;
use crate::recording::RecordId;
use crate::recording::RecordingFailedReason;
use crate::recording::RecordingOptions;
use crate::recording::RecordingScheduleState;
use crate::recording::RecordingStatus;
use crate::timeshift::TimeshiftRecordModel;
use crate::timeshift::TimeshiftRecorderModel;

/// Version information of mirakc currently running.
#[derive(Serialize, ToSchema)]
pub(in crate::web) struct Version {
    /// Current version.
    pub current: &'static str,

    /// Same as `current`.
    pub latest: &'static str,
}

/// State information of mirakc currently running.
#[derive(Serialize, ToSchema)]
pub(in crate::web) struct Status {}

/// A recording schedule model.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordingSchedule")]
pub(in crate::web) struct WebRecordingSchedule {
    /// The current state of the recording schedule.
    pub state: RecordingScheduleState,
    /// Metadata of the target TV program.
    pub program: MirakurunProgram,
    /// Recording options.
    pub options: RecordingOptions,
    /// A list of tags.
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
    /// Reason of the recording failure.
    ///
    /// This property exists only when the recording failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_reason: Option<RecordingFailedReason>,
}

impl From<recording::RecordingSchedule> for WebRecordingSchedule {
    fn from(value: recording::RecordingSchedule) -> Self {
        WebRecordingSchedule {
            state: value.state,
            program: value.program.clone().into(),
            options: value.options,
            tags: value.tags,
            failed_reason: value.failed_reason,
        }
    }
}

/// Input data used when creating a recording schedule.
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordingScheduleInput")]
pub(in crate::web) struct WebRecordingScheduleInput {
    /// A Mirakurun program ID of the target TV program.
    #[schema(value_type = u64)]
    pub program_id: ProgramId,
    /// Recording options.
    pub options: RecordingOptions,
    /// A list of tags.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,
}

impl WebRecordingScheduleInput {
    pub fn validate(&self, config: &Config) -> Result<(), Error> {
        match self.options.content_path.as_ref() {
            Some(content_path) => {
                if content_path.to_str().is_none() {
                    let err = Error::InvalidPath("Must be a valid Unicode string");
                    tracing::error!(
                        %err,
                        input.options.content_path = ?content_path
                    );
                    return Err(err);
                }

                if content_path.is_absolute() {
                    let err = Error::InvalidPath("Must be a relative path");
                    tracing::error!(
                        %err,
                        input.options.content_path = ?content_path
                    );
                    return Err(err);
                }

                let basedir = config.recording.basedir.as_ref().unwrap();
                if !basedir.join(content_path).parse_dot()?.starts_with(basedir) {
                    let err = Error::InvalidPath("Must be under config.recording.basedir");
                    tracing::error!(
                        %err,
                        input.options.content_path = ?content_path
                    );
                    return Err(err);
                }
            }
            None => {
                if !config.recording.is_records_api_enabled() {
                    return Err(Error::InvalidRequest("contentPath is required"));
                }
            }
        }

        Ok(())
    }
}

/// A recorder model.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordingRecorder")]
pub(in crate::web) struct WebRecordingRecorder {
    /// A Mirakurun program ID of the TV program currently being recorded.
    #[schema(value_type = u64)]
    pub program_id: ProgramId,
    /// A time when the recording started.
    ///
    /// It's may not be equal to the start time of the TV program.
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    pub started_at: DateTime<Jst>,
    /// A list of process models constituting the recording pipeline.
    pub pipeline: Vec<WebProcessModel>,
}

impl From<recording::RecorderModel> for WebRecordingRecorder {
    fn from(value: recording::RecorderModel) -> Self {
        WebRecordingRecorder {
            program_id: value.program_id,
            started_at: value.started_at,
            pipeline: value
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
        }
    }
}

/// A record model.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "Record")]
#[cfg_attr(test, derive(Deserialize))]
pub(in crate::web) struct WebRecord {
    /// The record ID.
    #[schema(value_type = u64)]
    pub id: RecordId,

    /// Metadata of the TV program.
    pub program: MirakurunProgram,

    /// Metadata of the service.
    pub service: MirakurunService,

    /// A list of tags copied from the recording schedule.
    #[schema(value_type = Vec<String>)]
    pub tags: HashSet<String>,

    /// Information about the recording.
    pub recording: WebRecordingInfo,

    /// Information about the content.
    pub content: WebContentInfo,
}

impl From<(Record, Option<u64>)> for WebRecord {
    fn from((record, content_length): (Record, Option<u64>)) -> Self {
        let (status, failed_reason) = match record.recording_status {
            RecordingStatus::Recording => (WebRecordingStatus::Recording, None),
            RecordingStatus::Finished => (WebRecordingStatus::Finished, None),
            RecordingStatus::Canceled => (WebRecordingStatus::Canceled, None),
            RecordingStatus::Failed { reason } => (WebRecordingStatus::Failed, Some(reason)),
        };
        Self {
            id: record.id,
            program: record.program.into(),
            service: record.service.into(),
            tags: record.tags,
            recording: WebRecordingInfo {
                options: record.options,
                status,
                start_time: record.recording_start_time,
                end_time: record.recording_end_time,
                duration: record.recording_duration,
                failed_reason,
            },
            content: WebContentInfo {
                path: record.content_path,
                r#type: record.content_type,
                length: content_length,
            },
        }
    }
}

/// A recording information model.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordingInfo")]
#[cfg_attr(test, derive(Deserialize))]
pub(in crate::web) struct WebRecordingInfo {
    /// Recording options.
    pub options: RecordingOptions,

    /// The current status of the record.
    pub status: WebRecordingStatus,

    /// The start time of the **actual** recording in UNIX time (milliseconds).
    ///
    /// The value may not equal to the start time of the TV program.
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    pub start_time: DateTime<Jst>,

    /// The end time of the **actual** recording in UNIX time (milliseconds).
    ///
    /// The value may not equal to the end time of the TV program.
    ///
    /// Undefined during recording.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "ts_milliseconds_option")]
    #[schema(value_type = i64)]
    pub end_time: Option<DateTime<Jst>>,

    /// The duration of the **actual** recording in milliseconds.
    ///
    /// The value may not equal to the duration of the TV program.
    ///
    /// Undefined during recording.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "duration_milliseconds_option")]
    #[schema(value_type = i64)]
    pub duration: Option<Duration>,

    /// The reason for the recording failure.
    ///
    /// This property is available only when the `status` is `failed`.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_reason: Option<RecordingFailedReason>,
}

/// A recording status.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
#[schema(title = "RecordingStatus")]
pub enum WebRecordingStatus {
    Recording,
    Finished,
    Canceled,
    Failed,
}

impl From<RecordingStatus> for WebRecordingStatus {
    fn from(value: RecordingStatus) -> Self {
        match value {
            RecordingStatus::Recording => Self::Recording,
            RecordingStatus::Finished => Self::Finished,
            RecordingStatus::Canceled => Self::Canceled,
            RecordingStatus::Failed { .. } => Self::Failed,
        }
    }
}

/// A content information model.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "ContentInfo")]
#[cfg_attr(test, derive(Deserialize))]
pub(in crate::web) struct WebContentInfo {
    /// The path of the content file relative to `config.recording.basedir`.
    #[schema(value_type = String)]
    pub path: PathBuf,

    /// The MIME type of the content.
    pub r#type: String,

    /// The size of the content.
    ///
    /// `null` if there is no content file at the location specified by `content_path` of the
    /// recording schedule.
    ///
    /// `0` will be set if failed getting the size of the content file even though the file exists.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, IntoParams)]
#[serde(rename_all = "kebab-case")]
#[into_params(parameter_in = Query)]
pub(in crate::web) struct WebRecordRemovalSetting {
    /// `1` or `true` will purge the content file.
    ///
    /// The content file won't be purged by default.
    #[serde(default = "WebRecordRemovalSetting::default_purge")]
    #[serde(deserialize_with = "WebRecordRemovalSetting::deserialize_purge")]
    pub purge: bool, // default: false
}

impl WebRecordRemovalSetting {
    fn default_purge() -> bool {
        false
    }

    // TODO(refactor): took from FilterSetting::deserialize_stream_decode_query()
    fn deserialize_purge<'de, D>(deserializer: D) -> Result<bool, D::Error>
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

/// The result of a record removal request.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "RecordRemovalResult")]
#[cfg_attr(test, derive(Deserialize))]
pub(in crate::web) struct WebRecordRemovalResult {
    /// `true` when the record file has been removed successfully.
    pub record_removed: bool,
    /// `true` when the content file has been removed successfully.
    /// `true` if there is no content file and the `purge` query parameter is set.
    /// Otherwise `false`.
    pub content_removed: bool,
}

/// A timeshift recorder model.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "TimeshiftRecorder")]
pub(in crate::web) struct WebTimeshiftRecorder {
    /// The timeshift recorder name defined in `config.yml`.
    pub name: String,
    /// Metadata of the service to be recorded.
    pub service: MirakurunService,
    /// The start time of the timeshift timeline.
    ///
    /// `null` when there is no record.
    #[serde(with = "ts_milliseconds_option")]
    #[schema(value_type = Option<i64>)]
    pub start_time: Option<DateTime<Jst>>,
    /// The end time of the timeshift timeline.
    ///
    /// `null` when there is no record.
    #[serde(with = "ts_milliseconds_option")]
    #[schema(value_type = Option<i64>)]
    pub end_time: Option<DateTime<Jst>>,
    /// The duration of the timeshift timeline.
    ///
    /// `0` when there is no record.
    #[serde(with = "duration_milliseconds")]
    #[schema(value_type = i64)]
    pub duration: Duration,
    /// The number of records available for playback.
    ///
    /// The number will change over the recording.  For example,
    /// [/timeshift/{recorder}/records](#/timeshift::records/getTimeshiftRecords) may
    /// return different number of records from this value.
    pub num_records: usize,
    /// A list of process models constituting the timeshift pipeline currently
    /// running.
    pub pipeline: Vec<WebProcessModel>,
    /// `true` while recording, `false` otherwise.
    ///
    /// Users can still access the records even if this property returns
    /// `false`.
    pub recording: bool,
    /// An ID of the record currently being recorded.
    #[schema(value_type = Option<u32>)]
    pub current_record_id: Option<TimeshiftRecordId>,
}

impl From<TimeshiftRecorderModel> for WebTimeshiftRecorder {
    fn from(model: TimeshiftRecorderModel) -> Self {
        Self {
            name: model.name,
            service: model.service.into(),
            start_time: model.start_time,
            end_time: model.end_time,
            duration: model.duration,
            num_records: model.num_records,
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

/// A process model constituting a pipeline.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "ProcessModel")]
pub(in crate::web) struct WebProcessModel {
    /// A command currently running in the pipeline.
    pub command: String,
    /// The process ID of a process running the command.
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

/// Metadata of a timeshift record.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "TimeshiftRecord")]
pub(in crate::web) struct WebTimeshiftRecord {
    /// A timeshift record ID.
    #[schema(value_type = u32)]
    pub id: TimeshiftRecordId,
    /// Metadata of the TV program.
    pub program: MirakurunProgram,
    #[serde(with = "ts_milliseconds")]
    #[schema(value_type = i64)]
    /// The start time of the timeshift record in UNIX time (milliseconds).
    pub start_time: DateTime<Jst>,
    #[serde(with = "duration_milliseconds")]
    #[schema(value_type = i64)]
    /// The duration of the timeshift record in milliseconds.
    pub duration: Duration,
    /// The size of the timeshift record in bytes.
    pub size: u64,
    /// `true` while recording, `false` otherwise.
    pub recording: bool,
}

impl From<TimeshiftRecordModel> for WebTimeshiftRecord {
    fn from(model: TimeshiftRecordModel) -> Self {
        Self {
            id: model.id,
            program: model.program.into(),
            start_time: model.start_time,
            duration: model.end_time - model.start_time,
            size: model.size,
            recording: model.recording,
        }
    }
}

/// Metadata of TV program that is now on-air in a service.
///
/// The metadata is collected from EIT[p/f] sections, not from EIT[schedule]
/// sections.
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(title = "OnairProgram")]
pub(crate) struct WebOnairProgram {
    /// Mirakurun service ID.
    #[schema(value_type = u64)]
    pub service_id: ServiceId,
    /// A TV program that is now on-air.
    ///
    /// `null` when no TV program is broadcasted.
    pub current: Option<MirakurunProgram>,
    /// A TV program that will start next.
    ///
    /// `null` when there is no next TV program.
    pub next: Option<MirakurunProgram>,
}

impl From<(ServiceId, OnairProgram)> for WebOnairProgram {
    fn from((service_id, data): (ServiceId, OnairProgram)) -> Self {
        Self {
            service_id,
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
    /// A timeshift recorder name.
    pub recorder: String,

    /// A timeshift record ID.
    #[param(value_type = u32)]
    pub id: TimeshiftRecordId,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(in crate::web) struct TimeshiftRecordQuery {
    /// A timeshift record ID.
    #[param(value_type = Option<u32>)]
    pub record: Option<TimeshiftRecordId>,
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

    /// A list of pre-filters to use.
    #[serde(default)]
    pub pre_filters: Vec<String>, // default: empty

    /// A list of post-filters to use.
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
            .next_back()
            .map(|value| value.to_str().ok().map_or(String::new(), |s| s.to_string()));

        let info = TunerUserInfo::Web { id, agent };

        let priority = parts
            .headers
            .get_all(super::X_MIRAKURUN_PRIORITY)
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

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_web_recording_schedule_input_valudate() {
        let mut config = Config::default();
        config.recording.basedir = Some("/tmp".into());

        let input = WebRecordingScheduleInput {
            program_id: (0, 1, 2).into(),
            options: recording_options!("1.m2ts", 1),
            tags: Default::default(),
        };
        assert_matches!(input.validate(&config), Ok(()));

        let input = WebRecordingScheduleInput {
            program_id: (0, 1, 2).into(),
            options: recording_options!("/1.m2ts", 1),
            tags: Default::default(),
        };
        assert_matches!(input.validate(&config), Err(err) => {
            assert_matches!(err, Error::InvalidPath(_));
        });

        let input = WebRecordingScheduleInput {
            program_id: (0, 1, 2).into(),
            options: recording_options!("../1.m2ts", 1),
            tags: Default::default(),
        };
        assert_matches!(input.validate(&config), Err(err) => {
            assert_matches!(err, Error::InvalidPath(_));
        });

        let input = WebRecordingScheduleInput {
            program_id: (0, 1, 2).into(),
            options: recording_options!(1),
            tags: Default::default(),
        };
        assert_matches!(input.validate(&config), Err(err) => {
            assert_matches!(err, Error::InvalidRequest(_));
        });

        config.recording.records_dir = Some("/tmp".into());

        let input = WebRecordingScheduleInput {
            program_id: (0, 1, 2).into(),
            options: recording_options!(1),
            tags: Default::default(),
        };
        assert_matches!(input.validate(&config), Ok(()));
    }
}
