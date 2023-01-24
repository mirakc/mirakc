use super::*;

use std::convert::Infallible;
use std::pin::Pin;

use axum::response::sse::Event;
use axum::response::sse::Sse;
use futures::stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::models::events::*;

pub(super) async fn events<E, R, S, O>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Error>
where
    E: Clone,
    E: Call<crate::epg::RegisterEmitter>,
    E: TriggerFactory<crate::epg::UnregisterEmitter>,
    R: Clone,
    R: Call<crate::recording::RegisterEmitter>,
    R: TriggerFactory<crate::recording::UnregisterEmitter>,
    S: Clone,
    S: Call<crate::timeshift::RegisterEmitter>,
    S: TriggerFactory<crate::timeshift::UnregisterEmitter>,
    O: Clone,
    O: Call<crate::onair::RegisterEmitter>,
    O: TriggerFactory<crate::onair::UnregisterEmitter>,
{
    let (sender, receiver) = mpsc::channel(32);

    let feeder = EventFeeder(sender);

    let id = epg
        .call(crate::epg::RegisterEmitter::ProgramsUpdated(
            feeder.clone().into(),
        ))
        .await?;
    let _epg_programs_updated_unregister_trigger =
        epg.trigger(crate::epg::UnregisterEmitter::ProgramsUpdated(id));

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStarted(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_started_unregister_trigger =
        recording_manager.trigger(crate::recording::UnregisterEmitter::RecordingStarted(id));

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingStopped(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_stopped_unregister_trigger =
        recording_manager.trigger(crate::recording::UnregisterEmitter::RecordingStopped(id));

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingFailed(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_failed_unregister_trigger =
        recording_manager.trigger(crate::recording::UnregisterEmitter::RecordingFailed(id));

    let id = recording_manager
        .call(crate::recording::RegisterEmitter::RecordingRescheduled(
            feeder.clone().into(),
        ))
        .await?;
    let _recording_rescheduled_unregister_trigger = recording_manager.trigger(
        crate::recording::UnregisterEmitter::RecordingRescheduled(id),
    );

    let id = timeshift_manager
        .call(crate::timeshift::RegisterEmitter(feeder.clone().into()))
        .await?;
    let _timeshift_event_unregister_trigger =
        timeshift_manager.trigger(crate::timeshift::UnregisterEmitter(id));

    let id = onair_manager
        .call(crate::onair::RegisterEmitter(feeder.clone().into()))
        .await?;
    let _onair_program_changed_unregister_trigger =
        onair_manager.trigger(crate::onair::UnregisterEmitter(id));

    // The Sse instance will be dropped in IntoResponse::into_response().
    // So, we have to create a wrapper for the event stream in order to
    // unregister emitters.
    let sse = Sse::new(EventStreamWrapper {
        inner: ReceiverStream::new(receiver),
        _epg_programs_updated_unregister_trigger,
        _recording_started_unregister_trigger,
        _recording_stopped_unregister_trigger,
        _recording_failed_unregister_trigger,
        _recording_rescheduled_unregister_trigger,
        _timeshift_event_unregister_trigger,
        _onair_program_changed_unregister_trigger,
    });
    Ok(sse.keep_alive(Default::default()))
}

#[derive(Clone)]
struct EventFeeder(mpsc::Sender<Result<Event, Infallible>>);

macro_rules! impl_into_emitter {
    ($msg:path) => {
        impl Into<Emitter<$msg>> for EventFeeder {
            fn into(self) -> Emitter<$msg> {
                Emitter::new(self)
            }
        }
    };
}

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

        impl_into_emitter! {$msg}
    };
}

impl_emit! {crate::epg::ProgramsUpdated, EpgProgramsUpdated}
impl_emit! {crate::recording::RecordingStarted, RecordingStarted}
impl_emit! {crate::recording::RecordingStopped, RecordingStopped}
impl_emit! {crate::recording::RecordingFailed, RecordingFailed}
impl_emit! {crate::recording::RecordingRescheduled, RecordingRescheduled}
impl_emit! {crate::onair::OnairProgramChanged, OnairProgramChanged}

#[async_trait]
impl Emit<crate::timeshift::TimeshiftEvent> for EventFeeder {
    async fn emit(&self, msg: crate::timeshift::TimeshiftEvent) {
        use crate::timeshift::TimeshiftEvent::*;
        let event = match msg {
            Started { recorder } => Event::default()
                .event("timeshift.started")
                .json_data(TimeshiftStarted { recorder })
                .unwrap(),
            Stopped { recorder } => Event::default()
                .event("timeshift.stopped")
                .json_data(TimeshiftStopped { recorder })
                .unwrap(),
            RecordStarted {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-started")
                .json_data(TimeshiftRecordStarted {
                    recorder,
                    record_id,
                })
                .unwrap(),
            RecordUpdated {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-updated")
                .json_data(TimeshiftRecordUpdated {
                    recorder,
                    record_id,
                })
                .unwrap(),
            RecordEnded {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-ended")
                .json_data(TimeshiftRecordEnded {
                    recorder,
                    record_id,
                })
                .unwrap(),
        };
        if let Err(_) = self.0.send(Ok(event)).await {
            tracing::warn!("Client disconnected");
        }
    }
}

impl_into_emitter! {crate::timeshift::TimeshiftEvent}

struct EventStreamWrapper<S> {
    inner: S,
    _epg_programs_updated_unregister_trigger: Trigger<crate::epg::UnregisterEmitter>,
    _recording_started_unregister_trigger: Trigger<crate::recording::UnregisterEmitter>,
    _recording_stopped_unregister_trigger: Trigger<crate::recording::UnregisterEmitter>,
    _recording_failed_unregister_trigger: Trigger<crate::recording::UnregisterEmitter>,
    _recording_rescheduled_unregister_trigger: Trigger<crate::recording::UnregisterEmitter>,
    _timeshift_event_unregister_trigger: Trigger<crate::timeshift::UnregisterEmitter>,
    _onair_program_changed_unregister_trigger: Trigger<crate::onair::UnregisterEmitter>,
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
