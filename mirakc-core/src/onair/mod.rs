mod local;
mod manager;
mod remote;

use std::sync::Arc;

use actlet::prelude::*;

use crate::epg::EpgProgram;
use crate::models::ServiceId;

pub use manager::OnairProgram;
pub use manager::OnairProgramManager;
pub use manager::QueryOnairProgram;
pub use manager::QueryOnairPrograms;
pub use manager::RegisterEmitter;
pub use manager::SpawnTemporalTracker;

#[cfg(test)]
pub(crate) use manager::stub;

// on-air program changed

#[derive(Clone, Message)]
pub struct OnairProgramChanged {
    pub service_id: ServiceId,
    pub current: Option<Arc<EpgProgram>>,
    pub next: Option<Arc<EpgProgram>>,
}

// tracker stopped

#[derive(Message)]
pub struct TrackerStopped {
    pub tracker: String,
}
