use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use actix::prelude::*;
use log;
use mustache;

use crate::broadcaster::*;
use crate::command_util::{spawn_pipeline, CommandPipeline};
use crate::config::{Config, TunerConfig};
use crate::error::Error;
use crate::models::*;
use crate::mpeg_ts_stream::MpegTsStream;

pub fn start(config: Arc<Config>) -> Addr<TunerManager> {
    TunerManager::new(config).start()
}

// identifiers

#[derive(Clone, Copy, Default, PartialEq)]
pub struct TunerSessionId {
    tuner_index: usize,
}

impl fmt::Display for TunerSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tuner#{}", self.tuner_index)
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct TunerSubscriptionId {
    session_id: TunerSessionId,
    serial_number: u32,
}

impl TunerSubscriptionId {
    pub fn new(session_id: TunerSessionId, serial_number: u32) -> Self {
        Self { session_id, serial_number }
    }
}

impl fmt::Display for TunerSubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.session_id, self.serial_number)
    }
}

// tuner manager

pub struct TunerManager {
    config: Arc<Config>,
    tuners: Vec<Tuner>,
}

struct TunerSubscription {
    id: TunerSubscriptionId,
    broadcaster: Addr<Broadcaster>,
}

impl TunerManager {
    pub fn new(config: Arc<Config>) -> Self {
        TunerManager { config, tuners: Vec::new() }
    }

    fn load_tuners(&mut self) {
        log::info!("Loading tuners...");
        let tuners: Vec<Tuner> = self.config
            .tuners
            .iter()
            .filter(|config| !config.disabled)
            .enumerate()
            .map(|(i, config)| Tuner::new(i, config))
            .collect();
        log::info!("Loaded {} tuners", tuners.len());
        self.tuners = tuners;
    }

    fn activate_tuner(
        &mut self,
        channel_type: ChannelType,
        channel: String,
        user: TunerUser,
    ) -> Result<TunerSubscription, Error> {
        if let TunerUserInfo::Tracker { stream_id } = user.info {
            let tuner = &mut self.tuners[stream_id.session_id.tuner_index];
            if tuner.is_active() {
                return Ok(tuner.subscribe(user));
            }
            return Err(Error::TunerUnavailable);
        }

        let found = self.tuners
            .iter_mut()
            .find(|tuner| tuner.is_reuseable(channel_type, &channel));
        if let Some(tuner) = found {
            log::info!("tuner#{}: Reuse tuner already activated with {} {}",
                       tuner.index, channel_type, channel);
            return Ok(tuner.subscribe(user));
        }

        let found = self.tuners
            .iter()
            .position(|tuner| tuner.is_available_for(channel_type));
        if let Some(index) = found {
            log::info!("tuner#{}: Activate with {} {}",
                       index, channel_type, channel);
            let filter = self.make_filter_command(
                index, channel_type, &channel)?;
            let tuner = &mut self.tuners[index];
            tuner.activate(channel_type, channel, filter)?;
            return Ok(tuner.subscribe(user));
        }

        // No available tuner at this point.  Take over the right to use
        // a tuner used by a low priority user.
        let found = self.tuners
            .iter()
            .filter(|tuner| tuner.is_supported_type(channel_type))
            .position(|tuner| tuner.can_grab(user.priority));
        if let Some(index) = found {
            log::info!("tuner#{}: Grab tuner, rectivate with {} {}",
                       index, channel_type, channel);
            let filter = self.make_filter_command(
                index, channel_type, &channel)?;
            let tuner = &mut self.tuners[index];
            tuner.deactivate();
            tuner.activate(channel_type, channel, filter)?;
            return Ok(tuner.subscribe(user));
        }

        log::warn!("No tuner available for {} {} {}",
                   channel_type, channel, user);
        Err(Error::TunerUnavailable)
    }

    fn deactivate_tuner(&mut self, id: TunerSubscriptionId) {
        log::info!("tuner#{}: Deactivate", id.session_id.tuner_index);
        self.tuners[id.session_id.tuner_index].deactivate();
    }

    fn stop_streaming(&mut self, id: TunerSubscriptionId) {
        log::info!("{}: Stop streaming", id);
        let _ = self.tuners[id.session_id.tuner_index].stop_streaming(id);
    }


    fn make_filter_command(
        &self,
        tuner_index: usize,
        channel_type: ChannelType,
        channel: &str,
    ) -> Result<String, Error> {
        let template = mustache::compile_str(
            &self.config.filters.tuner_filter)?;
        let data = mustache::MapBuilder::new()
            .insert("index", &tuner_index)?
            .insert("channel_type", &channel_type)?
            .insert_str("channel", channel)
            .build();
        Ok(template.render_data_to_string(&data)?)
    }
}

impl Actor for TunerManager {
    type Context = actix::Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        log::debug!("Started");
        self.load_tuners();
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        for tuner in self.tuners.iter_mut() {
            tuner.deactivate();
        }
        log::debug!("Stopped");
    }
}

// query tuners

pub struct QueryTunersMessage;

impl fmt::Display for QueryTunersMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryTuners")
    }
}

impl Message for QueryTunersMessage {
    type Result = Result<Vec<MirakurunTuner>, Error>;
}

impl Handler<QueryTunersMessage> for TunerManager {
    type Result = Result<Vec<MirakurunTuner>, Error>;

    fn handle(
        &mut self,
        msg: QueryTunersMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        let tuners: Vec<MirakurunTuner> = self.tuners
            .iter()
            .map(|tuner| tuner.get_model())
            .collect();
        Ok(tuners)
    }
}

// start streaming

pub struct StartStreamingMessage {
    pub channel_type: ChannelType,
    pub channel: String,
    pub user: TunerUser,
}

impl fmt::Display for StartStreamingMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StartStreaming {}/{} to {}",
               self.channel_type, self.channel, self.user)
    }
}

impl Message for StartStreamingMessage {
    type Result = Result<MpegTsStream, Error>;
}

impl Handler<StartStreamingMessage> for TunerManager {
    type Result = ActorResponse<Self, MpegTsStream, Error>;

    fn handle(
        &mut self,
        msg: StartStreamingMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);

        let subscription = match self.activate_tuner(
            msg.channel_type, msg.channel, msg.user) {
            Ok(broadcaster) => broadcaster,
            Err(err) => return ActorResponse::reply(Err(Error::from(err))),
        };

        let fut = actix::fut::wrap_future::<_, Self>(
            subscription.broadcaster.send(SubscribeMessage {
                id: subscription.id
            }))
            .map(move |result, act, ctx| {
                if result.is_ok() {
                    log::info!("{}: Started streaming", subscription.id);
                } else {
                    log::error!("{}: Broadcaster may have stopped",
                                subscription.id);
                    act.deactivate_tuner(subscription.id);
                }
                result
                    .map(|stream| {
                        MpegTsStream::new(
                            subscription.id, stream, ctx.address().recipient())
                    })
                    .map_err(Error::from)
            });

        ActorResponse::r#async(fut)
    }
}

// stop streaming

pub struct StopStreamingMessage {
    pub id: TunerSubscriptionId,
}

impl fmt::Display for StopStreamingMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StopStreaming {}", self.id)
    }
}

impl Message for StopStreamingMessage {
    type Result = ();
}

impl Handler<StopStreamingMessage> for TunerManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: StopStreamingMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.stop_streaming(msg.id)
    }
}

// tuner

struct Tuner {
    index: usize,
    name: String,
    channel_types: Vec<ChannelType>,
    command: String,
    time_limit: u64,
    activity: TunerActivity,
}

impl Tuner {
    fn new(
        index: usize,
        config: &TunerConfig,
    ) -> Self {
        Tuner {
            index,
            name: config.name.clone(),
            channel_types: config.channel_types.clone(),
            command: config.command.clone(),
            time_limit: config.time_limit,
            activity: TunerActivity::Inactive,
        }
    }

    fn is_active(&self) -> bool {
        self.activity.is_active()
    }

    fn is_available(&self) -> bool {
        self.activity.is_inactive()
    }

    fn is_supported_type(&self, channel_type: ChannelType) -> bool {
        self.channel_types.contains(&channel_type)
    }

    fn is_available_for(&self, channel_type: ChannelType) -> bool {
        self.is_available() && self.is_supported_type(channel_type)
    }

    fn is_reuseable(
        &self, channel_type: ChannelType, channel: &str) -> bool {
        self.activity.is_reuseable(channel_type, channel)
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        priority.is_grab() || self.activity.can_grab(priority)
    }

    fn activate(
        &mut self,
        channel_type: ChannelType,
        channel: String,
        filter: String,
    ) -> Result<(), Error> {
        let command = self.make_command(channel_type, &channel)?;
        self.activity.activate(
            self.index, channel_type, channel.clone(), command, filter,
            self.time_limit)
    }

    fn deactivate(&mut self) {
        self.activity.deactivate();
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        self.activity.subscribe(user)
    }

    fn stop_streaming(
        &mut self,
        id: TunerSubscriptionId,
    ) -> Result<(), Error> {
        let num_users = self.activity.stop_streaming(id)?;
        if num_users == 0 {
            self.deactivate();
        }
        Ok(())
    }

    fn get_model(&self) -> MirakurunTuner {
        let (command, pid, users) = self.activity.get_models();

        MirakurunTuner {
            index: self.index,
            name: self.name.clone(),
            channel_types: self.channel_types.clone(),
            command,
            pid,
            users,
            is_available: true,
            is_remote: false,
            is_free: self.is_available(),
            is_using: !self.is_available(),
            is_fault: false,
        }
    }

    fn make_command(
        &self,
        channel_type: ChannelType,
        channel: &str,
    ) -> Result<String, Error> {
        let template = mustache::compile_str(&self.command)?;
        let data = mustache::MapBuilder::new()
            .insert("channel_type", &channel_type)?
            .insert_str("channel", channel)
            .insert_str("duration", "-")
            .build();
        Ok(template.render_data_to_string(&data)?)
    }
}

// activity

enum TunerActivity {
    Inactive,
    Active(TunerSession),
}

impl TunerActivity {
    fn activate(
        &mut self,
        tuner_index: usize,
        channel_type: ChannelType,
        channel: String,
        command: String,
        filter: String,
        time_limit: u64,
    ) -> Result<(), Error> {
        match self {
            Self::Inactive => {
                let session = TunerSession::new(
                    tuner_index, channel_type, channel, command, filter,
                    time_limit)?;
                *self = Self::Active(session);
                Ok(())
            }
            Self::Active(_) => panic!("Must be deactivated before activating"),
        }
    }

    fn deactivate(&mut self) {
        *self = Self::Inactive;
    }

    fn is_active(&self) -> bool {
        match self {
            Self::Inactive => false,
            Self::Active(_) => true,
        }
    }

    fn is_inactive(&self) -> bool {
        !self.is_active()
    }

    fn is_reuseable(&self, channel_type: ChannelType, channel: &str) -> bool {
        match self {
            Self::Inactive => false,
            Self::Active(session) =>
                session.is_reuseable(channel_type, channel),
        }
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        match self {
            Self::Inactive => panic!("Must be activated before subscribing"),
            Self::Active(session) => session.subscribe(user),
        }
    }

    fn stop_streaming(
        &mut self,
        id: TunerSubscriptionId,
    ) -> Result<usize, Error> {
        match self {
            Self::Inactive => Err(Error::SessionNotFound),
            Self::Active(session) => session.stop_streaming(id),
        }
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        match self {
            Self::Inactive => true,
            Self::Active(session) => session.can_grab(priority),
        }
    }

    fn get_models(
        &self
    ) -> (Option<String>, Option<u32>, Vec<MirakurunTunerUser>) {
        match self {
            Self::Inactive => (None, None, Vec::new()),
            Self::Active(session) => session.get_models(),
        }
    }
}

// session

struct TunerSession {
    id: TunerSessionId,
    channel_type: ChannelType,
    channel: String,
    command: String,
    // Used for closing the tuner in order to take over the right to use it.
    pipeline: CommandPipeline<TunerSessionId>,
    broadcaster: Addr<Broadcaster>,
    subscribers: HashMap<u32, TunerUser>,
    next_serial_number: u32,
}

impl TunerSession {
    fn new(
        tuner_index: usize,
        channel_type: ChannelType,
        channel: String,
        command: String,
        filter: String,
        time_limit: u64,
    ) -> Result<TunerSession, Error> {
        let mut commands = vec![command.clone()];
        if filter.len() > 0 {
            commands.push(filter);
        }
        let id = TunerSessionId { tuner_index };
        let mut pipeline = spawn_pipeline(commands, id)?;
        let (_, output) = pipeline.take_endpoints()?;
        let broadcaster = Broadcaster::create(|ctx| {
            Broadcaster::new(id.clone(), output, time_limit, ctx)
        });

        log::info!("{}: Activated with {} {}", id, channel_type, channel);

        Ok(TunerSession {
            id, channel_type, channel, command, pipeline, broadcaster,
            subscribers: HashMap::new(), next_serial_number: 1
        })
    }

    fn is_reuseable(&self, channel_type: ChannelType, channel: &str) -> bool {
        self.channel_type == channel_type && self.channel == channel
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        let serial_number = self.next_serial_number;
        self.next_serial_number += 1;

        let id = TunerSubscriptionId::new(self.id, serial_number);
        log::info!("{}: Subscribed: {}", id, user);
        self.subscribers.insert(serial_number, user);

        TunerSubscription { id, broadcaster: self.broadcaster.clone() }
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        self.subscribers
            .values()
            .all(|user| priority > user.priority)
    }

    fn stop_streaming(
        &mut self,
        id: TunerSubscriptionId
    ) -> Result<usize, Error> {
        if self.id != id.session_id {
            log::warn!("Session ID unmatched, {} was probably deactivated",
                       id.session_id);
            return Err(Error::SessionNotFound);
        }
        match self.subscribers.remove(&id.serial_number) {
            Some(user) => log::info!("{}: Unsubscribed: {}", id, user),
            None => log::warn!("{}: Not subscribed", id),
        }
        self.broadcaster.do_send(UnsubscribeMessage { id });
        Ok(self.subscribers.len())
    }

    fn get_models(
        &self
    ) -> (Option<String>, Option<u32>, Vec<MirakurunTunerUser>) {
        (
            Some(self.command.clone()),
            Some(self.pipeline.pids().as_slice().first().cloned().unwrap()),
            self.subscribers.values().map(|user| user.get_model()).collect(),
        )
    }
}

impl Drop for TunerSession {
    fn drop(&mut self) {
        log::info!("{}: Deactivated", self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matches::assert_matches;
    use crate::command_util::Error as CommandUtilError;

    #[actix_rt::test]
    async fn test_tuner_is_active() {
        let config = create_config("true".to_string());
        let mut tuner = Tuner::new(0, &config);

        assert!(!tuner.is_active());

        let result = tuner.activate(
            ChannelType::GR, String::new(), String::new());
        assert!(result.is_ok());
        assert!(tuner.is_active());

        // Workaround for actix/actix/issues/372.
        //
        // See masnagam/rust-case-studies/tree/master/actix-started-in-drop for
        // details.
        tokio::task::yield_now().await;
    }

    #[actix_rt::test]
    async fn test_tuner_activate() {
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(
                ChannelType::GR, String::new(), String::new());
            assert!(result.is_ok());
            tokio::task::yield_now().await;
        }

        {
            let config = create_config("cmd '".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(
                ChannelType::GR, String::new(), String::new());
            assert_matches!(result, Err(Error::CommandFailed(
                CommandUtilError::UnableToParse(_))));
            tokio::task::yield_now().await;
        }

        {
            let config = create_config("no-such-command".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(
                ChannelType::GR, String::new(), String::new());
            assert_matches!(result, Err(Error::CommandFailed(
                CommandUtilError::UnableToSpawn(..))));
            tokio::task::yield_now().await;
        }
    }

    #[actix_rt::test]
    async fn test_tuner_stop_streaming() {
        let config = create_config("true".to_string());
        let mut tuner = Tuner::new(1, &config);
        let result = tuner.stop_streaming(Default::default());
        assert_matches!(result, Err(Error::SessionNotFound));

        let result = tuner.activate(
            ChannelType::GR, String::new(), String::new());
        assert!(result.is_ok());
        let subscription = tuner.subscribe(TunerUser {
            info: TunerUserInfo::Web { remote: None, agent: None },
            priority: 0.into(),
        });

        let result = tuner.stop_streaming(Default::default());
        assert_matches!(result, Err(Error::SessionNotFound));

        let result = tuner.stop_streaming(subscription.id);
        assert_matches!(result, Ok(()));

        tokio::task::yield_now().await;
    }

    #[actix_rt::test]
    async fn test_tuner_can_grab() {
        let config = create_config("true".to_string());
        let mut tuner = Tuner::new(0, &config);
        assert!(tuner.can_grab(0.into()));

        tuner.activate(
            ChannelType::GR, "1".to_string(), String::new()).unwrap();
        tuner.subscribe(create_user(0.into()));

        assert!(!tuner.can_grab(0.into()));
        assert!(tuner.can_grab(1.into()));
        assert!(tuner.can_grab(2.into()));
        assert!(tuner.can_grab(TunerUserPriority::GRAB));

        tuner.subscribe(create_user(1.into()));

        assert!(!tuner.can_grab(0.into()));
        assert!(!tuner.can_grab(1.into()));
        assert!(tuner.can_grab(2.into()));
        assert!(tuner.can_grab(TunerUserPriority::GRAB));

        tuner.subscribe(create_user(TunerUserPriority::GRAB));

        assert!(!tuner.can_grab(0.into()));
        assert!(!tuner.can_grab(1.into()));
        assert!(!tuner.can_grab(2.into()));
        assert!(tuner.can_grab(TunerUserPriority::GRAB));

        tokio::task::yield_now().await;
    }

    #[actix_rt::test]
    async fn test_tuner_reactivate() {
        let config = create_config("true".to_string());
        let mut tuner = Tuner::new(0, &config);
        tuner.activate(
            ChannelType::GR, "1".to_string(), String::new()).ok();

        tokio::task::yield_now().await;

        tuner.deactivate();
        let result = tuner.activate(
            ChannelType::GR, "2".to_string(), String::new());
        assert!(result.is_ok());

        tokio::task::yield_now().await;
    }

    fn create_config(command: String) -> TunerConfig {
        TunerConfig {
            name: String::new(),
            channel_types: vec![ChannelType::GR],
            command,
            time_limit: 10 * 1000,
            disabled: false,
        }
    }

    fn create_user(priority: TunerUserPriority) -> TunerUser {
        TunerUser {
            info: TunerUserInfo::Job { name: "test".to_string() },
            priority
        }
    }
}
