use chrono::DateTime;
use chrono::Duration;
use chrono_jst::serde::duration_milliseconds;
use chrono_jst::serde::ts_milliseconds_option;
use chrono_jst::Jst;
use serde::Deserialize;
use serde::Serialize;

use crate::models::ProgramId;
use crate::models::ServiceId;
use crate::models::TimeshiftRecordId;
use crate::recording::RecordingFailedReason;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerStatusChanged {
    pub tuner_index: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgProgramsUpdated {
    pub service_id: ServiceId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStarted {
    pub program_id: ProgramId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStopped {
    pub program_id: ProgramId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingFailed {
    pub program_id: ProgramId,
    pub reason: RecordingFailedReason,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRescheduled {
    pub program_id: ProgramId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftTimeline {
    pub recorder: String,
    #[serde(with = "ts_milliseconds_option")]
    pub start_time: Option<DateTime<Jst>>,
    #[serde(with = "ts_milliseconds_option")]
    pub end_time: Option<DateTime<Jst>>,
    #[serde(with = "duration_milliseconds")]
    pub duration: Duration,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftStarted {
    pub recorder: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftStopped {
    pub recorder: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftRecordStarted {
    pub recorder: String,
    pub record_id: TimeshiftRecordId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftRecordUpdated {
    pub recorder: String,
    pub record_id: TimeshiftRecordId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeshiftRecordEnded {
    pub recorder: String,
    pub record_id: TimeshiftRecordId,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnairProgramChanged {
    pub service_id: ServiceId,
}
