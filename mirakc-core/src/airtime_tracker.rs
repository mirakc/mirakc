use actlet::*;
use chrono::DateTime;
use chrono::Duration;
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

use crate::command_util;
use crate::datetime_ext::*;
use crate::epg::*;
use crate::error::Error;
use crate::models::*;
use crate::tuner::*;

pub async fn track_airtime<T, E>(
    command: &str,
    channel: &EpgChannel,
    program: &EpgProgram,
    stream_id: TunerSubscriptionId,
    tuner_manager: T,
    epg: E,
) -> Result<TunerStreamStopTrigger, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
{
    let user = TunerUser {
        info: TunerUserInfo::Tracker { stream_id },
        priority: (-1).into(),
    };

    let stream = tuner_manager
        .call(StartStreaming {
            channel: channel.clone(),
            user,
        })
        .await??;

    let template = mustache::compile_str(command)?;
    let data = mustache::MapBuilder::new()
        .insert("sid", &program.quad.sid())?
        .insert("eid", &program.quad.eid())?
        .build();
    let cmd = template.render_data_to_string(&data)?;

    let mut pipeline = command_util::spawn_pipeline(vec![cmd], stream.id())?;

    let (input, output) = pipeline.take_endpoints().unwrap();

    let stop_trigger = TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

    tokio::spawn(async {
        let _ = stream.pipe(input).await;
    });

    let quad = program.quad;
    tokio::spawn(async move {
        let _ = update_airtime(quad, output, epg).await;
        // Keep the pipeline until the tracker stops.
        drop(pipeline);
    });

    Ok(stop_trigger)
}

async fn update_airtime<R, E>(quad: ProgramQuad, output: R, epg: E) -> Result<(), Error>
where
    R: AsyncRead + Unpin,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
{
    tracing::info!("Tracking airtime of {}...", quad);

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
                tracing::error!("Failed to parse JSON: {}", err);
                continue;
            }
        };
        epg.call(UpdateAirtime {
            quad,
            airtime: airtime.into(),
        })
        .await?;
        json.clear();
    }
    epg.call(RemoveAirtime { quad }).await?;

    tracing::info!("Stopped tracking airtime of {}", quad);

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
