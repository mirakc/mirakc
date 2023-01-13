use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use actlet::prelude::*;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;

use crate::command_util;
use crate::config::Config;
use crate::epg::*;
use crate::error::Error;
use crate::models::*;
use crate::tuner::*;

pub struct EitFeeder<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
}

impl<T, E> EitFeeder<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        EitFeeder {
            config,
            tuner_manager,
            epg,
        }
    }

    async fn feed_eit_sections(&self) -> Result<(), Error> {
        let services = self.epg.call(QueryServices).await?;

        let mut map: HashMap<String, EpgChannel> = HashMap::new();
        for sv in services.values() {
            let chid = format!("{}/{}", sv.channel.channel_type, sv.channel.channel);
            map.entry(chid)
                .and_modify(|ch| ch.services.push(sv.sid()))
                .or_insert(EpgChannel {
                    name: sv.channel.name.clone(),
                    channel_type: sv.channel.channel_type,
                    channel: sv.channel.channel.clone(),
                    extra_args: sv.channel.extra_args.clone(),
                    services: vec![sv.sid()],
                    excluded_services: vec![],
                });
        }
        let channels: Vec<EpgChannel> = map.values().cloned().collect();

        EitCollector::new(
            self.config.jobs.update_schedules.command.clone(),
            channels,
            self.tuner_manager.clone(),
            self.epg.clone(),
        )
        .collect_schedules()
        .await
    }
}

#[async_trait]
impl<T, E> Actor for EitFeeder<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Started");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// feed eit sections

#[derive(Message)]
#[reply("Result<(), Error>")]
pub struct FeedEitSections;

#[async_trait]
impl<T, E> Handler<FeedEitSections> for EitFeeder<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    async fn handle(
        &mut self,
        _msg: FeedEitSections,
        _ctx: &mut Context<Self>,
    ) -> <FeedEitSections as Message>::Reply {
        tracing::debug!(msg.name = "FeedEitSections");
        self.feed_eit_sections().await
    }
}

// collector

pub struct EitCollector<T, E> {
    command: String,
    channels: Vec<EpgChannel>,
    tuner_manager: T,
    epg: E,
}

// TODO: The following implementation has code clones similar to
//       ClockSynchronizer and ServiceScanner.

impl<T, E> EitCollector<T, E>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    const LABEL: &'static str = "eit-collector";

    pub fn new(command: String, channels: Vec<EpgChannel>, tuner_manager: T, epg: E) -> Self {
        EitCollector {
            command,
            channels,
            tuner_manager,
            epg,
        }
    }

    pub async fn collect_schedules(self) -> Result<(), Error> {
        tracing::info!("Collecting EIT sections...");
        let mut num_sections = 0;
        for channel in self.channels.iter() {
            num_sections += Self::collect_eits_in_channel(
                &channel,
                &self.command,
                &self.tuner_manager,
                &self.epg,
            )
            .await?;
        }
        tracing::info!("Collected {} EIT sections", num_sections);
        Ok(())
    }

    async fn collect_eits_in_channel(
        channel: &EpgChannel,
        command: &str,
        tuner_manager: &T,
        epg: &E,
    ) -> Result<usize, Error> {
        tracing::debug!("Collecting EIT sections in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job {
                name: Self::LABEL.to_string(),
            },
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .call(StartStreaming {
                channel: channel.clone(),
                user,
                stream_id: None,
            })
            .await??;

        let stop_trigger = TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

        let template = mustache::compile_str(command)?;
        let data = mustache::MapBuilder::new()
            .insert("sids", &channel.services)?
            .insert("xsids", &channel.excluded_services)?
            .build();
        let cmd = template.render_data_to_string(&data)?;

        let mut pipeline = command_util::spawn_pipeline(vec![cmd], stream.id())?;

        let (input, output) = pipeline.take_endpoints().unwrap();

        let handle = tokio::spawn(stream.pipe(input));

        let mut reader = BufReader::new(output);
        let mut json = String::new();
        let mut num_sections = 0;
        let mut service_ids = HashSet::new();
        while reader.read_line(&mut json).await? > 0 {
            let section = match serde_json::from_str::<EitSection>(&json) {
                Ok(mut section) => {
                    // We assume that events in EIT[schedule] always have
                    // non-null values of the `start_time` and `duration`
                    // properties.
                    section.events.retain(|event| {
                        if event.start_time.is_none() {
                            tracing::warn!(%channel, %event.event_id,
                                           "Ignore event which has no start_time");
                            return false;
                        }
                        if event.duration.is_none() {
                            tracing::warn!(%channel, %event.event_id,
                                           "Ignore event which has no duration");
                            return false;
                        }
                        true
                    });
                    section
                }
                Err(err) => {
                    tracing::warn!(%err, %channel, "Ignore broken EIT section");
                    continue;
                }
            };
            if section.is_valid() {
                let service_id = section.service_id();
                if !service_ids.contains(&service_id) {
                    service_ids.insert(service_id);
                    epg.emit(PrepareSchedule { service_id }).await;
                }
                epg.emit(UpdateSchedule { section }).await;
                json.clear();
                num_sections += 1;
            } else {
                tracing::warn!(%channel, section.table_id, "Invalid table_id");
            }
        }

        drop(stop_trigger);

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(pipeline);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        for service_id in service_ids.into_iter() {
            epg.emit(FlushSchedule { service_id }).await;
        }

        tracing::debug!(
            "Collected {} EIT sections in {}",
            num_sections,
            channel.name
        );

        Ok(num_sections)
    }
}
