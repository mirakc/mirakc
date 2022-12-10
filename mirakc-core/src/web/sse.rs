use std::convert::Infallible;

use super::*;

use async_trait::async_trait;
use axum::response::sse::Event;
use axum::response::sse::Sse;
use futures::stream::Stream;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::models::MirakurunProgramId;
use crate::models::MirakurunServiceId;

pub(super) async fn events<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Error>
where
    E: Call<crate::epg::RegisterEmitter>,
    R: Call<crate::recording::RegisterEmitter>,
{
    let (sender, receiver) = mpsc::channel(32);

    let feeder = EventFeeder(sender);

    state
        .epg
        .call(crate::epg::RegisterEmitter::ProgramsUpdated(
            feeder.clone().into(),
        ))
        .await?;

    state
        .recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStarted(
            feeder.clone().into(),
        ))
        .await?;

    state
        .recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStopped(
            feeder.clone().into(),
        ))
        .await?;

    Ok(Sse::new(ReceiverStream::new(receiver)).keep_alive(Default::default()))
}

#[derive(Clone)]
struct EventFeeder(mpsc::Sender<Result<Event, Infallible>>);

macro_rules! impl_emit {
    ($msg:path, $event:ty) => {
        #[async_trait]
        impl Emit<$msg> for EventFeeder {
            async fn emit(&self, msg: $msg) {
                let event = Event::default()
                    .event(<$event>::name())
                    .json_data(<$event>::from(msg))
                    .unwrap();
                if let Err(_) = self.0.send(Ok(event)).await {
                    tracing::warn!("Client disconnected");
                }
            }
        }

        impl Into<Emitter<$msg>> for EventFeeder {
            fn into(self) -> Emitter<$msg> {
                Emitter::new(self)
            }
        }
    };
}

impl_emit!(crate::epg::ProgramsUpdated, EpgProgramsUpdated);
impl_emit!(crate::recording::RecordingStarted, RecordingStarted);
impl_emit!(crate::recording::RecordingStopped, RecordingStopped);

// events

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EpgProgramsUpdated {
    service_id: MirakurunServiceId,
}

impl EpgProgramsUpdated {
    const fn name() -> &'static str {
        "epg.programs-updated"
    }
}

impl From<crate::epg::ProgramsUpdated> for EpgProgramsUpdated {
    fn from(msg: crate::epg::ProgramsUpdated) -> Self {
        EpgProgramsUpdated {
            service_id: msg.service_triple.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingStarted {
    program_id: MirakurunProgramId,
}

impl RecordingStarted {
    const fn name() -> &'static str {
        "recording.started"
    }
}

impl From<crate::recording::RecordingStarted> for RecordingStarted {
    fn from(msg: crate::recording::RecordingStarted) -> Self {
        RecordingStarted {
            program_id: msg.program_quad.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingStopped {
    program_id: MirakurunProgramId,
    result: Result<u64, String>,
}

impl RecordingStopped {
    const fn name() -> &'static str {
        "recording.stopped"
    }
}

impl From<crate::recording::RecordingStopped> for RecordingStopped {
    fn from(msg: crate::recording::RecordingStopped) -> Self {
        RecordingStopped {
            program_id: msg.program_quad.into(),
            result: msg.result,
        }
    }
}
