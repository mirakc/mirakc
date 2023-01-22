use super::*;

use std::convert::Infallible;
use std::pin::Pin;

use axum::response::sse::Event;
use axum::response::sse::Sse;
use futures::stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::models::events::*;

pub(super) async fn events<E, R, O>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Error>
where
    E: Clone,
    E: Call<crate::epg::RegisterEmitter>,
    E: Into<Emitter<crate::epg::UnregisterEmitter>>,
    R: Clone,
    R: Call<crate::recording::RegisterEmitter>,
    R: Into<Emitter<crate::recording::UnregisterEmitter>>,
    O: Clone,
    O: Call<crate::onair::RegisterEmitter>,
    O: Into<Emitter<crate::onair::UnregisterEmitter>>,
{
    let (sender, receiver) = mpsc::channel(32);

    let feeder = EventFeeder(sender);

    let id = epg
        .call(crate::epg::RegisterEmitter::ProgramsUpdated(
            feeder.clone().into(),
        ))
        .await?;
    let _epg_programs_updated_cleaner = Cleaner {
        emitter: epg.clone().into(),
        msg: Some(crate::epg::UnregisterEmitter::ProgramsUpdated(id)),
    };

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStarted(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_started_cleaner = Cleaner {
        emitter: recording_manager.clone().into(),
        msg: Some(crate::recording::UnregisterEmitter::RecordingStarted(id)),
    };

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStopped(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_stopped_cleaner = Cleaner {
        emitter: recording_manager.clone().into(),
        msg: Some(crate::recording::UnregisterEmitter::RecordingStopped(id)),
    };

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingFailed(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_failed_cleaner = Cleaner {
        emitter: recording_manager.clone().into(),
        msg: Some(crate::recording::UnregisterEmitter::RecordingFailed(id)),
    };

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingRescheduled(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_rescheduled_cleaner = Cleaner {
        emitter: recording_manager.clone().into(),
        msg: Some(crate::recording::UnregisterEmitter::RecordingRescheduled(
            id,
        )),
    };

    let id = onair_manager
        .call(crate::onair::RegisterEmitter(feeder.clone().into()))
        .await?;
    let _onair_program_changed_cleaner = Cleaner {
        emitter: onair_manager.clone().into(),
        msg: Some(crate::onair::UnregisterEmitter(id)),
    };

    // The Sse instance will be dropped in IntoResponse::into_response().
    // So, we have to create a wrapper for the event stream in order to
    // unregister emitters.
    let sse = Sse::new(EventStreamWrapper {
        inner: ReceiverStream::new(receiver),
        _epg_programs_updated_cleaner,
        _recording_started_cleaner,
        _recording_stopped_cleaner,
        _recording_failed_cleaner,
        _recording_rescheduled_cleaner,
        _onair_program_changed_cleaner,
    });
    Ok(sse.keep_alive(Default::default()))
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
impl_emit!(crate::recording::RecordingFailed, RecordingFailed);
impl_emit!(crate::recording::RecordingRescheduled, RecordingRescheduled);
impl_emit!(crate::onair::OnairProgramChanged, OnairProgramChanged);

struct Cleaner<M: Signal> {
    emitter: Emitter<M>,
    msg: Option<M>,
}

impl<M: Signal> Drop for Cleaner<M> {
    fn drop(&mut self) {
        self.emitter.fire(self.msg.take().unwrap());
    }
}

struct EventStreamWrapper<S> {
    inner: S,
    _epg_programs_updated_cleaner: Cleaner<crate::epg::UnregisterEmitter>,
    _recording_started_cleaner: Cleaner<crate::recording::UnregisterEmitter>,
    _recording_stopped_cleaner: Cleaner<crate::recording::UnregisterEmitter>,
    _recording_failed_cleaner: Cleaner<crate::recording::UnregisterEmitter>,
    _recording_rescheduled_cleaner: Cleaner<crate::recording::UnregisterEmitter>,
    _onair_program_changed_cleaner: Cleaner<crate::onair::UnregisterEmitter>,
}

impl<S> Stream for EventStreamWrapper<S>
where
    S: Stream + Unpin,
{
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
