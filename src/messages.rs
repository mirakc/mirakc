use std::collections::HashMap;

use chrono::Duration;

use crate::models::*;
use crate::tuner::TunerUser;

// query channels

pub struct QueryChannelsMessage;

// query services

pub struct QueryServicesMessage;

// query programs

pub struct QueryProgramsMessage;

// query tuners

pub struct QueryTunersMessage;

// open tuner

pub enum OpenTunerBy {
    Channel { channel_type: ChannelType, channel: String },
    Service { id: u64 },
    Program { id: u64 },
}

pub struct OpenTunerMessage {
    pub by: OpenTunerBy,
    pub user: TunerUser,
    pub duration: Option<Duration>,
    pub preprocess: bool,
    pub postprocess: bool,
}

// close tuner

pub struct CloseTunerMessage {
    pub tuner_index: usize,
    pub session_id: u64,
}

// update epg

pub struct UpdateEpgMessage {
    pub services: Vec<ServiceModel>,
    pub programs: HashMap<u64, ProgramModel>,
}
