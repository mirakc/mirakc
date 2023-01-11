mod local;
mod manager;

use std::sync::Arc;

use actlet::prelude::*;

use crate::epg::EpgProgram;
use crate::models::ServiceTriple;

pub use manager::OnairProgram;
pub use manager::OnairProgramManager;
pub use manager::QueryOnairProgram;
pub use manager::QueryOnairPrograms;
pub use manager::RegisterEmitter;

#[cfg(test)]
pub(crate) use manager::stub;

// on-air program changed

#[derive(Clone, Message)]
pub struct OnairProgramChanged {
    pub service_triple: ServiceTriple,
    pub current: Option<Arc<EpgProgram>>,
    pub next: Option<Arc<EpgProgram>>,
}
