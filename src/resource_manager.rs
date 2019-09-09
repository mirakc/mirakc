use std::collections::HashMap;
use std::sync::Arc;

use actix::prelude::*;
use chrono::Duration;
use futures::Future;
use log;
use mustache;

use crate::config::Config;
use crate::epg;
use crate::error::Error;
use crate::messages::*;
use crate::models::*;
use crate::tuner::{Tuner, TunerUser, TunerOutput};

pub fn start(config: &Config) {
    actix::registry::SystemRegistry::set(
        ResourceManager::new(config).start());
}

pub fn query_channels(
    query: QueryChannelsMessage
) -> impl Future<Item = Vec<ChannelModel>, Error = Error> {
    ResourceManager::get().send(query).flatten()
}

pub fn query_services(
    query: QueryServicesMessage
) -> impl Future<Item = Arc<Vec<ServiceModel>>, Error = Error> {
    ResourceManager::get().send(query).flatten()
}

pub fn query_programs(
    query: QueryProgramsMessage
) -> impl Future<Item = Arc<HashMap<u64, ProgramModel>>, Error = Error> {
    ResourceManager::get().send(query).flatten()
}

pub fn query_tuners(
    query: QueryTunersMessage
) -> impl Future<Item = Vec<TunerModel>, Error = Error> {
    ResourceManager::get().send(query).flatten()
}

pub fn open_tuner(
    msg: OpenTunerMessage
) -> impl Future<Item = TunerOutput, Error = Error> {
    ResourceManager::get().send(msg).flatten()
}

pub fn close_tuner(
    msg: CloseTunerMessage
) {
    ResourceManager::get().do_send(msg);
}

pub fn update_epg(
    msg: UpdateEpgMessage
) {
    ResourceManager::get().do_send(msg);
}

struct ResourceManager {
    config: Config,
    epg_arbiter: Arbiter,
    tuners: Vec<Tuner>,
    services: Arc<Vec<ServiceModel>>,
    programs: Arc<HashMap<u64, ProgramModel>>,
}

impl ResourceManager {
    pub fn new(config: &Config) -> Self {
        ResourceManager {
            config: config.clone(),
            epg_arbiter: Arbiter::new(),
            tuners: Vec::new(),
            services: Arc::new(Vec::new()),
            programs: Arc::new(HashMap::new()),
        }
    }

    pub fn get() -> Addr<ResourceManager> {
        Self::from_registry()
    }

    fn init_tuners(&mut self) -> Result<(), Error> {
        log::info!("Init local tuners...");
        let tuners: Vec<Tuner> = self.config
            .tuners
            .iter()
            .filter(|config| !config.disabled)
            .enumerate()
            .map(|(i, config)| Tuner::new(i, config))
            .collect();
        let nlocal = tuners.len();
        log::info!("{} local tuners have been added", nlocal);

        self.tuners = tuners;
        Ok(())
    }

    fn open_tuner(
        &mut self,
        by: OpenTunerBy,
        user: TunerUser,
        duration: Option<Duration>,
        preprocess: bool,
        postprocess: bool,
    ) -> Result<TunerOutput, Error> {
        let output = match by {
            OpenTunerBy::Channel { channel_type, channel } =>
                self.open_tuner_by_channel(
                    channel_type, channel, user, duration, preprocess)?,
            OpenTunerBy::Service { id } =>
                self.open_tuner_by_service(id, user, duration, preprocess)?,
            OpenTunerBy::Program { id } =>
                self.open_tuner_by_program(id, user, duration, preprocess)?,
        };

        if postprocess && !self.config.tools.postprocess.is_empty() {
            Ok(output.pipe(&self.config.tools.postprocess)?)
        } else {
            Ok(output)
        }
    }

    fn open_tuner_by_channel(
        &mut self,
        channel_type: ChannelType,
        channel: String,
        user: TunerUser,
        duration: Option<Duration>,
        preprocess: bool,
    ) -> Result<TunerOutput, Error> {
        // Unlike Mirakurun, no packet is dropped even though it's a NULL
        // packet.

        let found = self.tuners
            .iter_mut()
            .find(|tuner| tuner.is_available_for(channel_type));
        let output = match found {
            Some(tuner) => {
                tuner.open(channel_type, channel, user, duration)?
            }
            None => {
                // No available tuner at this point.  Take over the right to use
                // a tuner used by a low priority user.
                let found = self.tuners
                    .iter_mut()
                    .filter(|tuner| tuner.is_supported_type(channel_type))
                    .find(|tuner| tuner.can_take_over(&user));
                match found {
                    Some(tuner) =>
                        tuner.take_over(channel_type, channel, user, duration)?,
                    None => return Err(Error::Unavailable),
                }
            }
        };

        if preprocess && !self.config.tools.preprocess.is_empty() {
            Ok(output.pipe(&self.config.tools.preprocess)?)
        } else {
            Ok(output)
        }
    }

    fn open_tuner_by_service(
        &mut self,
        id: u64,
        user: TunerUser,
        duration: Option<Duration>,
        preprocess: bool,
    ) -> Result<TunerOutput, Error> {
        let (sid, channel_type, channel) =
            match self.services.iter().find(|sv| sv.id == id) {
                Some(sv) => (sv.service_id, sv.channel.channel_type,
                             sv.channel.channel.clone()),
                None => return Err(Error::ServiceNotFound),
            };

        let output = self.open_tuner_by_channel(
            channel_type, channel, user, duration, preprocess)?;

        let template =
            mustache::compile_str(&self.config.tools.filter_service)?;
        let data = mustache::MapBuilder::new()
            .insert_str("sid", sid.to_string())
            .build();
        let cmd = template.render_data_to_string(&data)?;
        Ok(output.pipe(&cmd)?)
    }

    fn open_tuner_by_program(
        &mut self,
        id: u64,
        user: TunerUser,
        duration: Option<Duration>,
        preprocess: bool,
    ) -> Result<TunerOutput, Error> {
        let (sid, eid, until) = match self.programs.get(&id) {
            Some(prog) => (prog.service_id, prog.event_id, prog.end_at()),
            None => return Err(Error::ProgramNotFound),
        };

        let (channel_type, channel) = match self.find_channel_by_sid(sid) {
            Some(pair) => pair,
            None => return Err(Error::ServiceNotFound),
        };

        let output = self.open_tuner_by_channel(
            channel_type, channel, user, duration, preprocess)?;

        let template =
            mustache::compile_str(&self.config.tools.filter_program)?;
        let data = mustache::MapBuilder::new()
            .insert_str("sid", sid.to_string())
            .insert_str("eid", eid.to_string())
            .insert_str("until", until.to_string())
            .build();
        let cmd = template.render_data_to_string(&data)?;
        Ok(output.pipe(&cmd)?)
    }

    fn close_tuner(
        &mut self,
        tuner_index: usize,
        session_id: u64,
    ) {
        match self.tuners[tuner_index].close(session_id) {
            Ok(_) => (),
            Err(_) => (),
        }
    }

    fn find_channel_by_sid(
        &self,
        sid: u16,
    ) -> Option<(ChannelType, String)> {
        self.services
            .iter()
            .find(|sv| sv.service_id == sid)
            .map(|sv| (sv.channel.channel_type, sv.channel.channel.clone()))
    }
}

impl Actor for ResourceManager {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Context<Self>) {
        log::info!("Started");
        self.init_tuners().unwrap();
        epg::start(&self.epg_arbiter, &self.config);
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        // TODO: Stop background jobs
        // TODO: Close tuners if opened
        log::info!("Stopped");
    }
}

impl Supervised for ResourceManager {}
impl SystemService for ResourceManager {}

impl Default for ResourceManager {
    fn default() -> Self {
        unreachable!("Should be instantiated explicitely");
    }
}

// query channels

impl Message for QueryChannelsMessage {
    type Result = Result<Vec<ChannelModel>, Error>;
}

impl Handler<QueryChannelsMessage> for ResourceManager {
    type Result = Result<Vec<ChannelModel>, Error>;

    fn handle(
        &mut self,
        _: QueryChannelsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        let channels: Vec<ChannelModel> = self.config
            .channels
            .iter()
            .filter(|config| !config.disabled)
            .map(|config| ChannelModel {
                channel_type: config.channel_type,
                channel:  config.channel.clone(),
                name: config.name.clone(),
                services: self.services
                    .iter()
                    .filter(|sv| {
                        sv.channel.channel_type == config.channel_type &&
                            sv.channel.channel == config.channel
                    })
                    .map(|sv| ChannelServiceModel {
                        id: sv.id,
                        service_id: sv.service_id,
                        network_id: sv.network_id,
                        name: sv.name.clone(),
                    })
                    .collect()
            })
            .collect();

        Ok(channels)
    }
}

// query services

impl Message for QueryServicesMessage {
    type Result = Result<Arc<Vec<ServiceModel>>, Error>;
}

impl Handler<QueryServicesMessage> for ResourceManager {
    type Result = Result<Arc<Vec<ServiceModel>>, Error>;

    fn handle(
        &mut self,
        _: QueryServicesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        Ok(self.services.clone())
    }
}

// query programs

impl Message for QueryProgramsMessage {
    type Result = Result<Arc<HashMap<u64, ProgramModel>>, Error>;
}

impl Handler<QueryProgramsMessage> for ResourceManager {
    type Result = Result<Arc<HashMap<u64, ProgramModel>>, Error>;

    fn handle(
        &mut self,
        _: QueryProgramsMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        Ok(self.programs.clone())
    }
}

// query tuners

impl Message for QueryTunersMessage {
    type Result = Result<Vec<TunerModel>, Error>;
}

impl Handler<QueryTunersMessage> for ResourceManager {
    type Result = Result<Vec<TunerModel>, Error>;

    fn handle(
        &mut self,
        _: QueryTunersMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        let tuners: Vec<TunerModel> = self.tuners
            .iter()
            .map(|tuner| tuner.get_model())
            .collect();

        Ok(tuners)
    }
}

// open tuner

impl Message for OpenTunerMessage {
    type Result = Result<TunerOutput, Error>;
}

impl Handler<OpenTunerMessage> for ResourceManager {
    type Result = Result<TunerOutput, Error>;

    fn handle(
        &mut self,
        msg: OpenTunerMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.open_tuner(msg.by, msg.user, msg.duration,
                        msg.preprocess, msg.postprocess)
    }
}

// close tuner

impl Message for CloseTunerMessage {
    type Result = ();
}

impl Handler<CloseTunerMessage> for ResourceManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: CloseTunerMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.close_tuner(msg.tuner_index, msg.session_id)
    }
}

// update epg

impl Message for UpdateEpgMessage {
    type Result = ();
}

impl Handler<UpdateEpgMessage> for ResourceManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: UpdateEpgMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::info!("Updated EPG data");
        self.services = Arc::new(msg.services);
        self.programs = Arc::new(msg.programs);
    }
}
