#[cfg(test)]
#[macro_use]
mod vecdeque_lit;

mod filereader;
mod manager;
mod models;
mod recorder;

#[cfg(test)]
pub(crate) mod stub;

use actlet::prelude::*;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::Jst;

use crate::epg::EpgService;
use crate::error::Error;
use crate::models::TimeshiftRecordId;

pub use manager::TimeshiftManager;
pub use models::*;

// Used only for suppressing noisy logs in the rebuild mode.
pub(crate) fn is_rebuild_mode() -> bool {
    match std::env::var_os("MIRAKC_REBUILD_TIMESHIFT") {
        Some(v) => v == "1",
        None => false,
    }
}

// messages

#[derive(Message)]
#[reply(usize)]
pub struct RegisterEmitter(pub Emitter<TimeshiftEvent>);

#[derive(Message)]
pub struct UnregisterEmitter(pub usize);

#[derive(Message)]
#[reply(Result<Vec<TimeshiftRecorderModel>, Error>)]
pub struct QueryTimeshiftRecorders;

pub enum TimeshiftRecorderQuery {
    ByIndex(usize),
    ByName(String),
}

#[derive(Message)]
#[reply(Result<TimeshiftRecorderModel, Error>)]
pub struct QueryTimeshiftRecorder {
    pub recorder: TimeshiftRecorderQuery,
}

#[derive(Message)]
#[reply(Result<Vec<TimeshiftRecordModel>, Error>)]
pub struct QueryTimeshiftRecords {
    pub recorder: TimeshiftRecorderQuery,
}

#[derive(Message)]
#[reply(Result<TimeshiftRecordModel, Error>)]
pub struct QueryTimeshiftRecord {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: TimeshiftRecordId,
}

#[derive(Message)]
#[reply(Result<TimeshiftLiveStreamSource, Error>)]
pub struct CreateTimeshiftLiveStreamSource {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: Option<TimeshiftRecordId>,
}

#[derive(Message)]
#[reply(Result<TimeshiftRecordStreamSource, Error>)]
pub struct CreateTimeshiftRecordStreamSource {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: TimeshiftRecordId,
    pub start_pos: Option<u64>,
}

#[derive(Clone, Message)]
pub enum TimeshiftEvent {
    Timeline {
        recorder: String,
        start_time: Option<DateTime<Jst>>,
        end_time: Option<DateTime<Jst>>,
        duration: Duration,
    },
    Started {
        recorder: String,
    },
    Stopped {
        recorder: String,
    },
    RecordStarted {
        recorder: String,
        record_id: TimeshiftRecordId,
    },
    RecordUpdated {
        recorder: String,
        record_id: TimeshiftRecordId,
    },
    RecordEnded {
        recorder: String,
        record_id: TimeshiftRecordId,
    },
}

// private messages

#[derive(Message)]
#[reply()]
struct HealthCheck;

#[derive(Message)]
struct ServiceUpdated {
    service: Option<EpgService>,
}
