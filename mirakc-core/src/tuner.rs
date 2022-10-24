use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;

use crate::broadcaster::*;
use crate::command_util::spawn_pipeline;
use crate::command_util::CommandPipeline;
use crate::config::Config;
use crate::config::FilterConfig;
use crate::config::TunerConfig;
use crate::epg::EpgChannel;
use crate::error::Error;
use crate::models::*;
use crate::mpeg_ts_stream::MpegTsStream;

// identifiers

type TunerStream = MpegTsStream<TunerSubscriptionId, BroadcasterStream>;

#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(test, derive(Debug, Default))]
pub struct TunerSessionId {
    tuner_index: usize,
    session_number: u32,
}

impl TunerSessionId {
    pub fn new(tuner_index: usize) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let session_number = COUNTER.fetch_add(1, Ordering::Relaxed);
        TunerSessionId {
            tuner_index,
            session_number,
        }
    }
}

impl fmt::Display for TunerSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tuner#{}.{}", self.tuner_index, self.session_number)
    }
}

#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(test, derive(Debug, Default))]
pub struct TunerSubscriptionId {
    session_id: TunerSessionId,
    serial_number: u32,
}

impl TunerSubscriptionId {
    pub fn new(session_id: TunerSessionId, serial_number: u32) -> Self {
        Self {
            session_id,
            serial_number,
        }
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
    broadcaster: Address<Broadcaster>,
    decoded: bool,
}

impl TunerSubscription {
    fn new(id: TunerSubscriptionId, broadcaster: Address<Broadcaster>) -> Self {
        Self {
            id,
            broadcaster,
            decoded: false,
        }
    }
}

impl TunerManager {
    pub fn new(config: Arc<Config>) -> Self {
        TunerManager {
            config,
            tuners: Vec::new(),
        }
    }

    fn load_tuners(&mut self) {
        tracing::info!("Loading tuners...");
        let tuners: Vec<Tuner> = self
            .config
            .tuners
            .iter()
            .filter(|config| !config.disabled)
            .enumerate()
            .map(|(i, config)| Tuner::new(i, config))
            .collect();
        tracing::info!("Loaded {} tuners", tuners.len());
        self.tuners = tuners;
    }

    async fn activate_tuner<C>(
        &mut self,
        channel: EpgChannel,
        user: TunerUser,
        ctx: &C,
    ) -> Result<TunerSubscription, Error>
    where
        C: Spawn,
    {
        if let TunerUserInfo::Tracker { stream_id } = user.info {
            let tuner = &mut self.tuners[stream_id.session_id.tuner_index];
            if tuner.is_active() {
                return Ok(tuner.subscribe(user));
            }
            return Err(Error::TunerUnavailable);
        }

        let found = self
            .tuners
            .iter_mut()
            .find(|tuner| tuner.is_reuseable(&channel));
        if let Some(tuner) = found {
            tracing::info!(
                "tuner#{}: Reuse tuner already activated for {}",
                tuner.index,
                channel
            );
            return Ok(tuner.subscribe(user));
        }

        // Clone the config in order to avoid compile errors from the borrow checker.
        let config = self.config.clone();

        let found = self
            .tuners
            .iter_mut()
            .find(|tuner| tuner.is_available_for(&channel));
        if let Some(tuner) = found {
            tracing::info!("tuner#{}: Activate for {}", tuner.index, channel);
            let filters =
                Self::make_filter_commands(&tuner, &channel, &config.filters.tuner_filter)?;
            tuner.activate(channel, filters, ctx).await?;
            return Ok(tuner.subscribe(user));
        }

        // No available tuner at this point.
        // Grab a tuner used by lower priority users.
        let found = self
            .tuners
            .iter_mut()
            .filter(|tuner| tuner.is_supported_type(&channel))
            .find(|tuner| tuner.can_grab(user.priority));
        if let Some(tuner) = found {
            tracing::info!(
                "tuner#{}: Grab tuner, reactivate for {}",
                tuner.index,
                channel
            );
            let filters =
                Self::make_filter_commands(&tuner, &channel, &config.filters.tuner_filter)?;
            tuner.deactivate();
            tuner.activate(channel, filters, ctx).await?;
            return Ok(tuner.subscribe(user));
        }

        tracing::warn!("No tuner available for {} {}", channel, user);
        Err(Error::TunerUnavailable)
    }

    fn deactivate_tuner(&mut self, id: TunerSubscriptionId) {
        tracing::info!("tuner#{}: Deactivate", id.session_id.tuner_index);
        self.tuners[id.session_id.tuner_index].deactivate();
    }

    async fn stop_streaming(&mut self, id: TunerSubscriptionId) -> Result<(), Error> {
        tracing::info!("{}: Stop streaming", id);
        self.tuners[id.session_id.tuner_index]
            .stop_streaming(id)
            .await
    }

    fn make_filter_commands(
        tuner: &Tuner,
        channel: &EpgChannel,
        filter: &FilterConfig,
    ) -> Result<Vec<String>, Error> {
        let filter = Self::make_filter_command(tuner, channel, &filter.command)?;
        if filter.trim().is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![filter])
        }
    }

    fn make_filter_command(
        tuner: &Tuner,
        channel: &EpgChannel,
        filter: &str,
    ) -> Result<String, Error> {
        let template = mustache::compile_str(filter)?;
        let data = mustache::MapBuilder::new()
            .insert("tuner_index", &tuner.index)?
            .insert_str("tuner_name", &tuner.name)
            .insert_str("channel_name", &channel.name)
            .insert("channel_type", &channel.channel_type)?
            .insert_str("channel", &channel.channel)
            .build();
        Ok(template.render_data_to_string(&data)?)
    }
}

#[async_trait]
impl Actor for TunerManager {
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        // It's guaranteed that no response is sent before tuners are loaded.
        tracing::debug!("Started");
        self.load_tuners();
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        for tuner in self.tuners.iter_mut() {
            tuner.deactivate();
        }
        tracing::debug!("Stopped");
    }
}

// query tuners

#[derive(Message)]
#[reply("Vec<MirakurunTuner>")]
pub struct QueryTuners;

impl fmt::Display for QueryTuners {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryTuners")
    }
}

#[async_trait]
impl Handler<QueryTuners> for TunerManager {
    async fn handle(
        &mut self,
        msg: QueryTuners,
        _ctx: &mut Context<Self>,
    ) -> <QueryTuners as Message>::Reply {
        tracing::debug!("{}", msg);
        self.tuners
            .iter()
            .map(|tuner| tuner.get_mirakurun_model())
            .collect()
    }
}

// start streaming

#[derive(Message)]
#[reply("Result<TunerStream, Error>")]
pub struct StartStreaming {
    pub channel: EpgChannel,
    pub user: TunerUser,
}

impl fmt::Display for StartStreaming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StartStreaming {} to {}", self.channel, self.user)
    }
}

#[async_trait]
impl Handler<StartStreaming> for TunerManager {
    async fn handle(
        &mut self,
        msg: StartStreaming,
        ctx: &mut Context<Self>,
    ) -> <StartStreaming as Message>::Reply {
        tracing::debug!("{}", msg);

        let subscription = self.activate_tuner(msg.channel, msg.user, ctx).await?;

        let result = subscription
            .broadcaster
            .call(Subscribe {
                id: subscription.id,
            })
            .await;
        match result {
            Ok(stream) => {
                tracing::info!("{}: Started streaming", subscription.id);
                let stream = MpegTsStream::new(subscription.id, stream);
                let stream = if subscription.decoded {
                    stream.decoded()
                } else {
                    stream
                };
                Ok(stream)
            }
            Err(err) => {
                tracing::error!("{}: Broadcaster may have stopped", subscription.id);
                self.deactivate_tuner(subscription.id);
                Err(Error::from(err))
            }
        }
    }
}

// stop streaming

#[derive(Message)]
pub struct StopStreaming {
    pub id: TunerSubscriptionId,
}

impl fmt::Display for StopStreaming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StopStreaming {}", self.id)
    }
}

#[async_trait]
impl Handler<StopStreaming> for TunerManager {
    async fn handle(&mut self, msg: StopStreaming, _ctx: &mut Context<Self>) {
        tracing::debug!("{}", msg);
        match self.stop_streaming(msg.id).await {
            Ok(_) => (),
            Err(err) => tracing::error!("{}", err),
        }
    }
}

// tuner

struct Tuner {
    index: usize,
    name: String,
    channel_types: Vec<ChannelType>,
    command: String,
    time_limit: u64,
    decoded: bool,
    activity: TunerActivity,
}

impl Tuner {
    fn new(index: usize, config: &TunerConfig) -> Self {
        Tuner {
            index,
            name: config.name.clone(),
            channel_types: config.channel_types.clone(),
            command: config.command.clone(),
            time_limit: config.time_limit,
            decoded: config.decoded,
            activity: TunerActivity::Inactive,
        }
    }

    fn is_active(&self) -> bool {
        self.activity.is_active()
    }

    fn is_available(&self) -> bool {
        self.activity.is_inactive()
    }

    fn is_supported_type(&self, channel: &EpgChannel) -> bool {
        self.channel_types.contains(&channel.channel_type)
    }

    fn is_available_for(&self, channel: &EpgChannel) -> bool {
        self.is_available() && self.is_supported_type(channel)
    }

    fn is_reuseable(&self, channel: &EpgChannel) -> bool {
        self.activity.is_reuseable(channel)
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        priority.is_grab() || self.activity.can_grab(priority)
    }

    async fn activate<C>(
        &mut self,
        channel: EpgChannel,
        filters: Vec<String>,
        ctx: &C,
    ) -> Result<(), Error>
    where
        C: Spawn,
    {
        let command = self.make_command(&channel)?;
        self.activity
            .activate(self.index, channel, command, filters, self.time_limit, ctx)
            .await
    }

    fn deactivate(&mut self) {
        self.activity.deactivate();
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        let mut subscription = self.activity.subscribe(user);
        subscription.decoded = self.decoded;
        subscription
    }

    async fn stop_streaming(&mut self, id: TunerSubscriptionId) -> Result<(), Error> {
        let num_users = self.activity.stop_streaming(id).await?;
        if num_users == 0 {
            self.deactivate();
        }
        Ok(())
    }

    fn get_mirakurun_model(&self) -> MirakurunTuner {
        let (command, pid, users) = self.activity.get_mirakurun_models();

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

    fn make_command(&self, channel: &EpgChannel) -> Result<String, Error> {
        let template = mustache::compile_str(&self.command)?;
        let data = mustache::MapBuilder::new()
            .insert("channel_type", &channel.channel_type)?
            .insert_str("channel", &channel.channel)
            .insert_str("extra_args", &channel.extra_args)
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
    async fn activate<C>(
        &mut self,
        tuner_index: usize,
        channel: EpgChannel,
        command: String,
        filters: Vec<String>,
        time_limit: u64,
        ctx: &C,
    ) -> Result<(), Error>
    where
        C: Spawn,
    {
        match self {
            Self::Inactive => {
                let session =
                    TunerSession::new(tuner_index, channel, command, filters, time_limit, ctx)
                        .await?;
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

    fn is_reuseable(&self, channel: &EpgChannel) -> bool {
        match self {
            Self::Inactive => false,
            Self::Active(session) => session.is_reuseable(channel),
        }
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        match self {
            Self::Inactive => panic!("Must be activated before subscribing"),
            Self::Active(session) => session.subscribe(user),
        }
    }

    async fn stop_streaming(&mut self, id: TunerSubscriptionId) -> Result<usize, Error> {
        match self {
            Self::Inactive => Err(Error::SessionNotFound),
            Self::Active(session) => session.stop_streaming(id).await,
        }
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        match self {
            Self::Inactive => true,
            Self::Active(session) => session.can_grab(priority),
        }
    }

    fn get_mirakurun_models(&self) -> (Option<String>, Option<u32>, Vec<MirakurunTunerUser>) {
        match self {
            Self::Inactive => (None, None, Vec::new()),
            Self::Active(session) => session.get_mirakurun_models(),
        }
    }
}

// session

struct TunerSession {
    id: TunerSessionId,
    channel: EpgChannel,
    command: String,
    // Used for closing the tuner in order to take over the right to use it.
    pipeline: CommandPipeline<TunerSessionId>,
    broadcaster: Address<Broadcaster>,
    subscribers: HashMap<u32, TunerUser>,
    next_serial_number: u32,
}

impl TunerSession {
    async fn new<C>(
        tuner_index: usize,
        channel: EpgChannel,
        command: String,
        mut filters: Vec<String>,
        time_limit: u64,
        ctx: &C,
    ) -> Result<TunerSession, Error>
    where
        C: Spawn,
    {
        let mut commands = vec![command.clone()];
        commands.append(&mut filters);
        let id = TunerSessionId::new(tuner_index);
        let mut pipeline = spawn_pipeline(commands, id)?;
        let (_, output) = pipeline.take_endpoints()?;
        let broadcaster = ctx
            .spawn_actor(Broadcaster::new(id.clone(), time_limit))
            .await;
        broadcaster.emit(BindStream(output)).await;

        tracing::info!("{}: Activated with {}", id, channel);

        Ok(TunerSession {
            id,
            channel,
            command,
            pipeline,
            broadcaster,
            subscribers: HashMap::new(),
            next_serial_number: 1,
        })
    }

    fn is_reuseable(&self, channel: &EpgChannel) -> bool {
        self.channel.channel_type == channel.channel_type && self.channel.channel == channel.channel
    }

    fn subscribe(&mut self, user: TunerUser) -> TunerSubscription {
        let serial_number = self.next_serial_number;
        self.next_serial_number += 1;

        let id = TunerSubscriptionId::new(self.id, serial_number);
        tracing::info!("{}: Subscribed: {}", id, user);
        self.subscribers.insert(serial_number, user);

        TunerSubscription::new(id, self.broadcaster.clone())
    }

    fn can_grab(&self, priority: TunerUserPriority) -> bool {
        self.subscribers
            .values()
            .all(|user| priority > user.priority)
    }

    async fn stop_streaming(&mut self, id: TunerSubscriptionId) -> Result<usize, Error> {
        if self.id != id.session_id {
            tracing::warn!(
                "Session ID unmatched, {} was probably deactivated",
                id.session_id
            );
            return Err(Error::SessionNotFound);
        }
        match self.subscribers.remove(&id.serial_number) {
            Some(user) => tracing::info!("{}: Unsubscribed: {}", id, user),
            None => tracing::warn!("{}: Not subscribed", id),
        }
        self.broadcaster.emit(Unsubscribe { id }).await;
        Ok(self.subscribers.len())
    }

    fn get_mirakurun_models(&self) -> (Option<String>, Option<u32>, Vec<MirakurunTunerUser>) {
        (
            Some(self.command.clone()),
            self.pipeline.pids().iter().cloned().next().flatten(),
            self.subscribers
                .values()
                .map(|user| user.get_mirakurun_model())
                .collect(),
        )
    }
}

impl Drop for TunerSession {
    fn drop(&mut self) {
        tracing::info!("{}: Deactivated", self.id);
    }
}

pub struct TunerStreamStopTrigger {
    id: TunerSubscriptionId,
    emitter: Option<Emitter<StopStreaming>>,
}

impl TunerStreamStopTrigger {
    pub fn new(id: TunerSubscriptionId, emitter: Emitter<StopStreaming>) -> Self {
        Self {
            id,
            emitter: Some(emitter),
        }
    }
}

impl Drop for TunerStreamStopTrigger {
    fn drop(&mut self) {
        tracing::debug!("{}: Closing...", self.id);
        if let Some(emitter) = self.emitter.take() {
            emitter.fire(StopStreaming { id: self.id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_util::Error as CommandUtilError;
    use assert_matches::assert_matches;

    #[tokio::test]
    async fn test_start_streaming() {
        let system = System::new();

        {
            let config: Arc<Config> = Arc::new(
                serde_yaml::from_str(
                    r#"
                tuners:
                  - name: bs
                    types: [BS]
                    command: >-
                      true
                  - name: gr
                    types: [GR]
                    command: >-
                      true
                "#,
                )
                .unwrap(),
            );

            let manager = system.spawn_actor(TunerManager::new(config)).await;

            let result = manager
                .call(StartStreaming {
                    channel: create_channel("0"),
                    user: create_user(0.into()),
                })
                .await;
            let stream1 = assert_matches!(result, Ok(Ok(stream)) => {
                assert_eq!(stream.id().session_id.tuner_index, 1);
                stream
            });

            // Reuse the tuner
            let result = manager
                .call(StartStreaming {
                    channel: create_channel("0"),
                    user: create_user(1.into()),
                })
                .await;
            assert_matches!(result, Ok(Ok(stream)) => {
                assert_eq!(stream.id().session_id, stream1.id().session_id);
                assert_ne!(stream.id(), stream1.id());
            });

            // Lower and same priority user cannot grab the tuner
            let result = manager
                .call(StartStreaming {
                    channel: create_channel("1"),
                    user: create_user(1.into()),
                })
                .await;
            assert_matches!(result, Ok(Err(Error::TunerUnavailable)));

            // Higher priority user can grab the tuner
            let result = manager
                .call(StartStreaming {
                    channel: create_channel("1"),
                    user: create_user(2.into()),
                })
                .await;
            assert_matches!(result, Ok(Ok(stream)) => {
                assert_eq!(stream.id().session_id.tuner_index, 1);
                assert_ne!(stream.id().session_id, stream1.id().session_id);
            });
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_tuner_is_active() {
        let system = System::new();
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(0, &config);

            assert!(!tuner.is_active());

            let result = tuner.activate(create_channel("1"), vec![], &system).await;
            assert!(result.is_ok());
            assert!(tuner.is_active());
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_tuner_activate() {
        let system = System::new();
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(create_channel("1"), vec![], &system).await;
            assert!(result.is_ok());
            tokio::task::yield_now().await;
        }

        {
            let config = create_config("cmd '".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(create_channel("1"), vec![], &system).await;
            assert_matches!(
                result,
                Err(Error::CommandFailed(CommandUtilError::UnableToParse(_)))
            );
            tokio::task::yield_now().await;
        }

        {
            let config = create_config("no-such-command".to_string());
            let mut tuner = Tuner::new(0, &config);
            let result = tuner.activate(create_channel("1"), vec![], &system).await;
            assert_matches!(
                result,
                Err(Error::CommandFailed(CommandUtilError::UnableToSpawn(..)))
            );
            tokio::task::yield_now().await;
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_tuner_stop_streaming() {
        let system = System::new();
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(1, &config);
            let result = tuner.stop_streaming(Default::default()).await;
            assert_matches!(result, Err(Error::SessionNotFound));

            let result = tuner.activate(create_channel("1"), vec![], &system).await;
            assert!(result.is_ok());
            let subscription = tuner.subscribe(TunerUser {
                info: TunerUserInfo::Web {
                    id: "".to_string(),
                    agent: None,
                },
                priority: 0.into(),
            });

            let result = tuner.stop_streaming(Default::default()).await;
            assert_matches!(result, Err(Error::SessionNotFound));

            let result = tuner.stop_streaming(subscription.id).await;
            assert_matches!(result, Ok(()));

            tokio::task::yield_now().await;
        }
        system.stop();
    }

    #[tokio::test]
    async fn test_tuner_can_grab() {
        let system = System::new();
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            assert!(tuner.can_grab(0.into()));

            tuner
                .activate(create_channel("1"), vec![], &system)
                .await
                .unwrap();
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
        system.stop();
    }

    #[tokio::test]
    async fn test_tuner_reactivate() {
        let system = System::new();
        {
            let config = create_config("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            tuner
                .activate(create_channel("1"), vec![], &system)
                .await
                .ok();

            tokio::task::yield_now().await;

            tuner.deactivate();
            let result = tuner.activate(create_channel("2"), vec![], &system).await;
            assert!(result.is_ok());

            tokio::task::yield_now().await;
        }
        system.stop();
    }

    fn create_config(command: String) -> TunerConfig {
        TunerConfig {
            name: String::new(),
            channel_types: vec![ChannelType::GR],
            command,
            time_limit: 10 * 1000,
            disabled: false,
            decoded: false,
        }
    }

    fn create_channel(channel: &str) -> EpgChannel {
        EpgChannel {
            name: "".to_string(),
            channel_type: ChannelType::GR,
            channel: channel.to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
        }
    }

    fn create_user(priority: TunerUserPriority) -> TunerUser {
        TunerUser {
            info: TunerUserInfo::Job {
                name: "test".to_string(),
            },
            priority,
        }
    }
}
