use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use actlet::prelude::*;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tracing::Instrument;

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
    T: TriggerFactory<StopStreaming>,
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

    async fn feed_eit_sections(&self, ctx: &Context<Self>) -> Result<(), Error> {
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
            self.config.jobs.update_schedules.timeout,
            channels,
            self.tuner_manager.clone(),
            self.epg.clone(),
        )
        .collect_schedules(ctx)
        .await
    }
}

#[async_trait]
impl<T, E> Actor for EitFeeder<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Started");
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopping...");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// feed eit sections

#[derive(Message)]
#[reply(Result<(), Error>)]
pub struct FeedEitSections;

#[async_trait]
impl<T, E> Handler<FeedEitSections> for EitFeeder<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    async fn handle(
        &mut self,
        _msg: FeedEitSections,
        ctx: &mut Context<Self>,
    ) -> <FeedEitSections as Message>::Reply {
        tracing::debug!(msg.name = "FeedEitSections");
        self.feed_eit_sections(ctx).await
    }
}

// collector

pub struct EitCollector<T, E> {
    command: String,
    timeout: Duration,
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
    T: TriggerFactory<StopStreaming>,
    E: Emit<FlushSchedule>,
    E: Emit<PrepareSchedule>,
    E: Emit<UpdateSchedule>,
{
    const LABEL: &'static str = "epg.update-schedules";

    pub fn new(
        command: String,
        timeout: Duration,
        channels: Vec<EpgChannel>,
        tuner_manager: T,
        epg: E,
    ) -> Self {
        EitCollector {
            command,
            timeout,
            channels,
            tuner_manager,
            epg,
        }
    }

    pub async fn collect_schedules<C: Spawn>(self, ctx: &C) -> Result<(), Error> {
        for channel in self.channels.iter() {
            Self::collect_eits_in_channel(
                channel,
                &self.command,
                self.timeout,
                &self.tuner_manager,
                &self.epg,
                ctx,
            )
            .await?;
        }
        Ok(())
    }

    async fn collect_eits_in_channel<C: Spawn>(
        channel: &EpgChannel,
        command: &str,
        timeout: Duration,
        tuner_manager: &T,
        epg: &E,
        ctx: &C,
    ) -> Result<(), Error> {
        tracing::debug!(channel.name, "Collecting EIT sections...");

        let user = TunerUser {
            info: TunerUserInfo::Job(Self::LABEL.to_string()),
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .call(StartStreaming {
                channel: channel.clone(),
                user,
                stream_id: None,
            })
            .await??;

        let msg = StopStreaming { id: stream.id() };
        let stop_trigger = tuner_manager.trigger(msg);

        let template = mustache::compile_str(command)?;
        let data = mustache::MapBuilder::new()
            .insert("sids", &channel.services)?
            .insert("xsids", &channel.excluded_services)?
            .build();
        let cmd = template.render_data_to_string(&data)?;

        let mut pipeline = command_util::spawn_pipeline(vec![cmd], stream.id(), Self::LABEL, ctx)?;

        let (input, output) = pipeline.take_endpoints();

        let (handle, _) = ctx.spawn_task(stream.pipe(input).in_current_span());

        let mut reader = BufReader::new(output);
        let mut json = String::new();
        let mut num_sections = 0;
        let mut service_ids = HashSet::new();

        let timeout = tokio::time::sleep(timeout);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                result = {
                    assert!(json.is_empty());
                    reader.read_line(&mut json)
                }=> {
                    if result? == 0 {
                        break;
                    }
                    let mut section = match serde_json::from_str::<EitSection>(&json) {
                        Ok(section) if section.is_valid() => section,
                        Ok(section) => {
                            tracing::warn!(%channel, section.table_id, "Invalid table_id");
                            json.clear();
                            continue;
                        }
                        Err(err) => {
                            tracing::warn!(%err, %channel, "Ignore broken EIT section");
                            json.clear();
                            continue;
                        }
                    };
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
                    let service_id = section.service_id();
                    if !service_ids.contains(&service_id) {
                        service_ids.insert(service_id);
                        epg.emit(PrepareSchedule { service_id }).await;
                    }
                    epg.emit(UpdateSchedule { section }).await;
                    num_sections += 1;
                    json.clear();
                }
                _ = &mut timeout => {
                    tracing::warn!(err = "Timed out", %channel);
                    break;
                }
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
            channel.name,
            sections.len = num_sections,
            "Collected EIT sections"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_collect_eits_in_channel() {
        let ctx = actlet::stubs::Context::default();

        let tuner_stub = TunerManagerStub::default();

        // TODO: add tests

        // Emulate out of services by using `false`
        let config = Arc::new(
            serde_yaml::from_str::<Config>(
                r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              update-schedules:
                command: false
        "#,
            )
            .unwrap(),
        );
        let mut epg_mock = MockEpg::new();
        epg_mock.0.expect_emit_prepare_schedule().never();
        epg_mock.0.expect_emit_update_schedule().never();
        epg_mock.0.expect_emit_flush_schedule().never();
        let result = EitCollector::collect_eits_in_channel(
            &channel_gr!("channel", "0"),
            &config.jobs.update_schedules.command,
            config.jobs.update_schedules.timeout,
            &tuner_stub,
            &epg_mock,
            &ctx,
        )
        .await;
        assert_matches!(result, Ok(()));

        // Timed out
        let config = Arc::new(
            serde_yaml::from_str::<Config>(
                r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              update-schedules:
                command: sleep 10
                timeout: 10ms
        "#,
            )
            .unwrap(),
        );
        let mut epg_mock = MockEpg::new();
        epg_mock.0.expect_emit_prepare_schedule().never();
        epg_mock.0.expect_emit_update_schedule().never();
        epg_mock.0.expect_emit_flush_schedule().never();
        let result = EitCollector::collect_eits_in_channel(
            &channel_gr!("channel", "0"),
            &config.jobs.update_schedules.command,
            config.jobs.update_schedules.timeout,
            &tuner_stub,
            &epg_mock,
            &ctx,
        )
        .await;
        assert_matches!(result, Ok(()));
    }

    // NOTE: The following code is not compilable:
    //
    // ```
    // mockall::mock! {
    //     Epg {}
    //
    //     #[async_trait]
    //     impl Emit<PrepareSchedule> for Epg {
    //         async fn emit(&self, msg: PrepareSchedule);
    //     }
    //
    //     #[async_trait]
    //     impl Emit<UpdateSchedule> for Epg {
    //         async fn emit(&self, msg: UpdateSchedule);
    //     }
    //
    //     #[async_trait]
    //     impl Emit<FlushSchedule> for Epg {
    //         async fn emit(&self, msg: FlushSchedule);
    //     }
    // }
    // ```
    //
    // See https://github.com/asomers/mockall/issues/271#issuecomment-825710521.

    mockall::mock! {
        pub EpgInner {
            fn emit_prepare_schedule(&self, msg: PrepareSchedule);
            fn emit_update_schedule(&self, msg: UpdateSchedule);
            fn emit_flush_schedule(&self, msg: FlushSchedule);
        }
    }

    struct MockEpg(MockEpgInner);

    impl MockEpg {
        fn new() -> Self {
            Self(MockEpgInner::new())
        }
    }

    #[async_trait]
    impl Emit<PrepareSchedule> for MockEpg {
        async fn emit(&self, msg: PrepareSchedule) {
            self.0.emit_prepare_schedule(msg);
        }
    }

    #[async_trait]
    impl Emit<UpdateSchedule> for MockEpg {
        async fn emit(&self, msg: UpdateSchedule) {
            self.0.emit_update_schedule(msg);
        }
    }

    #[async_trait]
    impl Emit<FlushSchedule> for MockEpg {
        async fn emit(&self, msg: FlushSchedule) {
            self.0.emit_flush_schedule(msg);
        }
    }
}
