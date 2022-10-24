use std::collections::HashMap;
use std::sync::Arc;

use actlet::*;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

#[cfg(test)]
use serde::Serialize;

use crate::command_util;
use crate::config::ChannelConfig;
use crate::config::Config;
use crate::epg::*;
use crate::models::*;
use crate::tuner::*;

pub struct ClockSynchronizer<T> {
    config: Arc<Config>,
    tuner_manager: T,
}

// TODO: The following implementation has code clones similar to
//       EitCollector and ServiceScanner.

impl<T> ClockSynchronizer<T>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    const LABEL: &'static str = "clock-synchronizer";

    pub fn new(config: Arc<Config>, tuner_manager: T) -> Self {
        ClockSynchronizer {
            config,
            tuner_manager,
        }
    }

    pub async fn sync_clocks(self) -> Vec<(EpgChannel, Option<HashMap<ServiceTriple, Clock>>)> {
        tracing::debug!("Synchronizing clocks...");

        let command = &self.config.jobs.sync_clocks.command;
        let mut results = Vec::new();

        for channel in self.config.channels.iter() {
            let result =
                match Self::sync_clocks_in_channel(channel, command, &self.tuner_manager).await {
                    Ok(clocks) => {
                        let mut map = HashMap::new();
                        for clock in clocks.into_iter() {
                            let triple = (clock.nid, clock.tsid, clock.sid).into();
                            map.insert(triple, clock.clock.clone());
                        }
                        Some(map)
                    }
                    Err(err) => {
                        tracing::warn!("Failed to synchronize clocks in {}: {}", channel.name, err);
                        None
                    }
                };
            results.push((channel.clone().into(), result));
        }

        tracing::debug!("Synchronized {} channels", self.config.channels.len());

        results
    }

    async fn sync_clocks_in_channel(
        channel: &ChannelConfig,
        command: &str,
        tuner_manager: &T,
    ) -> anyhow::Result<Vec<SyncClock>> {
        tracing::debug!("Synchronizing clocks in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job {
                name: Self::LABEL.to_string(),
            },
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .call(StartStreaming {
                channel: channel.clone().into(),
                user,
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

        let (input, mut output) = pipeline.take_endpoints().unwrap();

        let handle = tokio::spawn(stream.pipe(input));

        let mut buf = Vec::new();
        output.read_to_end(&mut buf).await?;

        drop(stop_trigger);

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(pipeline);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        anyhow::ensure!(!buf.is_empty(), "No clock, maybe out of service");

        let clocks: Vec<SyncClock> = serde_json::from_slice(&buf)?;
        tracing::debug!("Synchronized {} clocks in {}", clocks.len(), channel.name);

        Ok(clocks)
    }
}

#[derive(Clone, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(rename_all = "camelCase")]
struct SyncClock {
    nid: NetworkId,
    tsid: TransportStreamId,
    sid: ServiceId,
    clock: Clock,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcaster::BroadcasterStream;
    use crate::mpeg_ts_stream::MpegTsStream;
    use assert_matches::assert_matches;

    #[tokio::test]
    async fn test_sync_clocks_in_channel() {
        let stub = TunerManagerStub;

        let expected = vec![SyncClock {
            nid: 1.into(),
            tsid: 2.into(),
            sid: 3.into(),
            clock: Clock {
                pid: 1,
                pcr: 2,
                time: 3,
            },
        }];

        let config_yml = format!(
            r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              sync-clocks:
                command: echo '{}'
        "#,
            serde_json::to_string(&expected).unwrap()
        );

        let config = Arc::new(serde_yaml::from_str::<Config>(&config_yml).unwrap());

        let sync = ClockSynchronizer::new(config, stub.clone());
        let results = sync.sync_clocks().await;
        assert_eq!(results.len(), 1);
        assert_matches!(&results[0], (_, Some(v)) => {
            let triple = (1, 2, 3).into();
            assert_eq!(v.len(), 1);
            assert!(v.contains_key(&triple));
            assert_matches!(v[&triple], Clock { pid, pcr, time } => {
                assert_eq!(pid, 1);
                assert_eq!(pcr, 2);
                assert_eq!(time, 3);
            });
        });

        // Emulate out of services by using `false`
        let config = Arc::new(
            serde_yaml::from_str::<Config>(
                r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              scan-services:
                command: false
        "#,
            )
            .unwrap(),
        );
        let sync = ClockSynchronizer::new(config, stub.clone());
        let results = sync.sync_clocks().await;
        assert_eq!(results.len(), 1);
        assert_matches!(&results[0], (_, None));
    }

    #[derive(Clone)]
    struct TunerManagerStub;

    #[async_trait]
    impl Call<StartStreaming> for TunerManagerStub {
        async fn call(
            &self,
            _msg: StartStreaming,
        ) -> Result<<StartStreaming as Message>::Reply, actlet::Error> {
            let (_, stream) = BroadcasterStream::new_for_test();
            Ok(Ok(MpegTsStream::new(
                TunerSubscriptionId::default(),
                stream,
            )))
        }
    }

    #[async_trait]
    impl Emit<StopStreaming> for TunerManagerStub {
        async fn emit(&self, _msg: StopStreaming) {}
        fn fire(&self, _msg: StopStreaming) {}
    }

    impl Into<Emitter<StopStreaming>> for TunerManagerStub {
        fn into(self) -> Emitter<StopStreaming> {
            Emitter::new(self)
        }
    }
}
