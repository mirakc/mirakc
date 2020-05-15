use std::collections::HashMap;

use actix::prelude::*;
use failure::Error;
use log;
use serde::Deserialize;
use serde_json;
use tokio::io::AsyncReadExt;

#[cfg(test)]
use serde::Serialize;

use crate::command_util;
use crate::epg::*;
use crate::models::*;
use crate::tuner::*;

pub struct ClockSynchronizer {
    command: String,
    channels: Vec<EpgChannel>,
    stream_manager: Recipient<StartStreamingMessage>,
}

// TODO: The following implementation has code clones similar to
//       EitCollector and ServiceScanner.

impl ClockSynchronizer {
    const LABEL: &'static str = "clock-synchronizer";

    pub fn new(
        command: String,
        channels: Vec<EpgChannel>,
        stream_manager: Recipient<StartStreamingMessage>,
    ) -> Self {
        ClockSynchronizer { command, channels, stream_manager }
    }

    pub async fn sync_clocks(
        self
    ) -> Vec<(EpgChannel, Option<HashMap<ServiceTriple, Clock>>)> {
        log::debug!("Synchronizing clocks...");

        let mut results = Vec::new();

        for channel in self.channels.iter() {
            let result = match Self::sync_clocks_in_channel(
                &channel, &self.command, &self.stream_manager).await {
                Ok(clocks) => {
                    let mut map = HashMap::new();
                    for clock in clocks.into_iter() {
                        let triple = (clock.nid, clock.tsid, clock.sid).into();
                        map.insert(triple, clock.clock.clone());
                    }
                    Some(map)
                }
                Err(err) => {
                    log::warn!("Failed to synchronize clocks in {}: {}",
                               channel.name, err);
                    None
                }
            };
            results.push((channel.clone(), result));
        }

        log::debug!("Synchronized {} channels", self.channels.len());

        results
    }

    async fn sync_clocks_in_channel(
        channel: &EpgChannel,
        command: &str,
        stream_manager: &Recipient<StartStreamingMessage>,
    ) -> Result<Vec<SyncClock>, Error> {
        log::debug!("Synchronizing clocks in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job { name: Self::LABEL.to_string() },
            priority: (-1).into(),
        };

        let stream = stream_manager.send(StartStreamingMessage {
            channel_type: channel.channel_type,
            channel: channel.channel.clone(),
            user
        }).await??;

        let template = mustache::compile_str(command)?;
        let data = mustache::MapBuilder::new()
            .insert("sids", &channel.services)?
            .insert("xsids", &channel.excluded_services)?
            .build();
        let cmd = template.render_data_to_string(&data)?;

        let (input, mut output) = command_util::spawn_pipeline(
            vec![cmd], stream.id())?;

        let handle = tokio::spawn(stream.pipe(input));

        let mut buf = Vec::new();
        output.read_to_end(&mut buf).await?;

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(output);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        failure::ensure!(!buf.is_empty(), "No clock, maybe out of service");

        let clocks: Vec<SyncClock> = serde_json::from_slice(&buf)?;
        log::debug!("Synchronized {} clocks in {}", clocks.len(), channel.name);

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
    use crate::error::Error;
    use crate::mpeg_ts_stream::MpegTsStream;

    type Mock = actix::actors::mocker::Mocker<TunerManager>;

    #[actix_rt::test]
    async fn test_sync_clocks_in_channel() {
        let mock = Mock::mock(Box::new(|msg, ctx| {
            if let Some(_) = msg.downcast_ref::<StartStreamingMessage>() {
                let (_, stream) = BroadcasterStream::new_for_test();
                let result: Result<_, Error> = Ok(MpegTsStream::new(
                    Default::default(), stream, ctx.address().recipient()));
                Box::new(Some(result))
            } else if let Some(_) = msg.downcast_ref::<StopStreamingMessage>() {
                Box::new(Some(()))
            } else {
                unimplemented!();
            }
        })).start();

        let channels = vec![EpgChannel {
            name: "channel".to_string(),
            channel_type: ChannelType::GR,
            channel: "0".to_string(),
            services: vec![],
            excluded_services: vec![],
        }];

        let expected = vec![SyncClock {
            nid: 1.into(),
            tsid: 2.into(),
            sid: 3.into(),
            clock: Clock { pcr: 10, time: 11 },
        }];

        let cmd = format!(
            "echo '{}'", serde_json::to_string(&expected).unwrap());
        let sync = ClockSynchronizer::new(
            cmd, channels.clone(), mock.clone().recipient());
        let results = sync.sync_clocks().await;
        assert!(results[0].1.is_some());
        assert_eq!(results[0].1.as_ref().unwrap().len(), 1);

        // Emulate out of services by using `false`
        let cmd = "false".to_string();
        let sync = ClockSynchronizer::new(
            cmd, channels.clone(), mock.clone().recipient());
        let results = sync.sync_clocks().await;
        assert!(results[0].1.is_none());
    }
}
