use actix::prelude::*;
use chrono::{DateTime, Duration};
use log;
use serde::Deserialize;
use serde_json;
use tokio::prelude::*;
use tokio::io::BufReader;

use crate::command_util;
use crate::datetime_ext::*;
use crate::epg::*;
use crate::error::Error;
use crate::models::*;
use crate::mpeg_ts_stream::*;
use crate::tuner::*;

#[cfg(not(test))]
type TunerManagerActor = TunerManager;
#[cfg(test)]
type TunerManagerActor = actix::actors::mocker::Mocker<TunerManager>;

#[cfg(not(test))]
type EpgActor = Epg;
#[cfg(test)]
type EpgActor = actix::actors::mocker::Mocker<Epg>;

pub async fn track_airtime(
    command: &str,
    channel: &EpgChannel,
    program: &EpgProgram,
    stream_id: MpegTsStreamId,
    tuner_manager: Addr<TunerManagerActor>,
    epg: Addr<EpgActor>,
) -> Result<Option<MpegTsStreamStopTrigger>, Error> {
    let user = TunerUser {
        info: TunerUserInfo::Tracker { stream_id },
        priority: (-1).into(),
    };

    let mut stream = tuner_manager.send(StartStreamingMessage {
        channel_type: channel.channel_type,
        channel: channel.channel.clone(),
        user
    }).await??;

    let template = mustache::compile_str(command)?;
    let data = mustache::MapBuilder::new()
        .insert("sid", &program.quad.sid())?
        .insert("eid", &program.quad.eid())?
        .build();
    let cmd = template.render_data_to_string(&data)?;

    let mut pipeline = command_util::spawn_pipeline(
        vec![cmd], stream.id())?;

    let (input, output) = pipeline.take_endpoints().unwrap();

    let stop_trigger = stream.take_stop_trigger();

    actix::spawn(async {
        let _ = stream.pipe(input).await;
    });

    let quad = program.quad;
    actix::spawn(async move {
        let _ = update_airtime(quad, output, epg).await;
        // Keep the pipeline until the tracker stops.
        drop(pipeline);
    });

    Ok(stop_trigger)
}

async fn update_airtime<R: AsyncRead + Unpin>(
    quad: EventQuad,
    output: R,
    epg: Addr<EpgActor>,
) -> Result<(), Error> {
    log::info!("Tracking airtime of {}...", quad);

    let mut reader = BufReader::new(output);
    let mut json = String::new();
    while reader.read_line(&mut json).await? > 0 {
        let airtime = match serde_json::from_str::<TsAirtime>(&json) {
            Ok(airtime) => {
                debug_assert_eq!(airtime.nid, quad.nid());
                debug_assert_eq!(airtime.tsid, quad.tsid());
                debug_assert_eq!(airtime.sid, quad.sid());
                debug_assert_eq!(airtime.eid, quad.eid());
                airtime
            }
            Err(err) => {
                log::error!("Failed to parse JSON: {}", err);
                continue
            }
        };
        epg.send(UpdateAirtimeMessage {
            quad,
            airtime: airtime.into(),
        }).await?;
        json.clear();
    }
    epg.send(RemoveAirtimeMessage { quad }).await?;

    log::info!("Stopped tracking airtime of {}", quad);

    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsAirtime {
    nid: NetworkId,
    tsid: TransportStreamId,
    sid: ServiceId,
    eid: EventId,
    #[serde(with = "serde_jst")]
    start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    duration: Duration,
}

impl Into<Airtime> for TsAirtime {
    fn into(self) -> Airtime {
        Airtime {
            start_time: self.start_time,
            duration: self.duration,
        }
    }
}
