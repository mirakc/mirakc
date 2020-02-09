use std::collections::HashMap;

use log;
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::io::AsyncReadExt;

use crate::command_util;
use crate::epg::*;
use crate::error::Error;
use crate::models::*;
use crate::mpeg_ts_stream::*;
use crate::tuner;

pub struct ClockSynchronizer {
    command: String,
    channels: Vec<EpgChannel>,
}

// TODO: The following implementation has code clones similar to
//       EitCollector and ServiceScanner.

impl ClockSynchronizer {
    const LABEL: &'static str = "clock-synchronizer";

    pub fn new(
        command: String,
        channels: Vec<EpgChannel>
    ) -> Self {
        ClockSynchronizer { command, channels }
    }

    pub async fn sync_clocks(
        self
    ) -> Result<HashMap<ServiceTriple, Clock>, Error> {
        log::debug!("Synchronizing clocks...");

        let mut clocks = Vec::new();
        for channel in self.channels.iter() {
            clocks.append(&mut Self::sync_clocks_in_channel(
                &channel, &self.command).await?);
        }

        let mut map = HashMap::new();
        for clock in clocks.iter() {
            let triple =
                ServiceTriple::from((clock.nid, clock.tsid, clock.sid));
            map.insert(triple, clock.clock.clone());
        }

        log::debug!("Synchronized {} clocks", map.len());

        Ok(map)
    }

    async fn sync_clocks_in_channel(
        channel: &EpgChannel,
        command: &str,
    ) -> Result<Vec<SyncClock>, Error> {
        log::debug!("Synchronizing clocks in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job { name: Self::LABEL.to_string() },
            priority: (-1).into(),
        };

        let stream = tuner::start_streaming(
            channel.channel_type, channel.channel.clone(), user).await?;

        let stream = MpegTsStreamTerminator::new(stream.id(), stream);

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

        let clocks: Vec<SyncClock> = serde_json::from_slice(&buf)?;
        log::debug!("Synchronized {} clocks in {}", clocks.len(), channel.name);

        Ok(clocks)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncClock {
    nid: NetworkId,
    tsid: TransportStreamId,
    sid: ServiceId,
    clock: Clock,
}
