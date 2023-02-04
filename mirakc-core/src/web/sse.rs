use super::*;

use std::convert::Infallible;
use std::pin::Pin;

use axum::response::sse::Event;
use axum::response::sse::Sse;
use futures::stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::events::*;

pub(super) async fn events<T, E, R, S, O>(
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Error>
where
    T: Clone,
    T: Call<crate::tuner::RegisterEmitter>,
    T: TriggerFactory<crate::tuner::UnregisterEmitter>,
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

    let id = tuner_manager
        .call(crate::tuner::RegisterEmitter(feeder.clone().into()))
        .await?;
    let _tuner_event_unregister_trigger =
        tuner_manager.trigger(crate::tuner::UnregisterEmitter(id));

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
        _tuner_event_unregister_trigger,
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

macro_rules! impl_emit {
    ($msg:path) => {
        #[async_trait]
        impl Emit<$msg> for EventFeeder {
            async fn emit(&self, msg: $msg) {
                if let Err(_) = self.0.send(Ok(msg.into())).await {
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

// tuner events

impl_emit! {crate::tuner::Event}

impl Into<Event> for crate::tuner::Event {
    fn into(self) -> Event {
        match self {
            Self::StatusChanged(tuner_index) => Event::default()
                .event("tuner.status-changed")
                .json_data(TunerStatusChanged { tuner_index })
                .unwrap(),
        }
    }
}

// epg events

impl_emit! {crate::epg::ProgramsUpdated}

impl Into<Event> for crate::epg::ProgramsUpdated {
    fn into(self) -> Event {
        Event::default()
            .event("epg.programs-updated")
            .json_data(EpgProgramsUpdated {
                service_id: self.service_id.into(),
            })
            .unwrap()
    }
}

// recording events

impl_emit! {crate::recording::RecordingStarted}

impl Into<Event> for crate::recording::RecordingStarted {
    fn into(self) -> Event {
        Event::default()
            .event("recording.started")
            .json_data(RecordingStarted {
                program_id: self.program_id.into(),
            })
            .unwrap()
    }
}

impl_emit! {crate::recording::RecordingStopped}

impl Into<Event> for crate::recording::RecordingStopped {
    fn into(self) -> Event {
        Event::default()
            .event("recording.stopped")
            .json_data(RecordingStopped {
                program_id: self.program_id.into(),
            })
            .unwrap()
    }
}

impl_emit! {crate::recording::RecordingFailed}

impl Into<Event> for crate::recording::RecordingFailed {
    fn into(self) -> Event {
        Event::default()
            .event("recording.failed")
            .json_data(RecordingFailed {
                program_id: self.program_id.into(),
                reason: self.reason,
            })
            .unwrap()
    }
}

impl_emit! {crate::recording::RecordingRescheduled}

impl Into<Event> for crate::recording::RecordingRescheduled {
    fn into(self) -> Event {
        Event::default()
            .event("recording.rescheduled")
            .json_data(RecordingRescheduled {
                program_id: self.program_id.into(),
            })
            .unwrap()
    }
}

// timeshift events

impl_emit! {crate::timeshift::TimeshiftEvent}

impl Into<Event> for crate::timeshift::TimeshiftEvent {
    fn into(self) -> Event {
        match self {
            Self::Timeline {
                recorder,
                start_time,
                end_time,
                duration,
            } => Event::default()
                .event("timeshift.timeline")
                .json_data(TimeshiftTimeline {
                    recorder,
                    start_time,
                    end_time,
                    duration,
                })
                .unwrap(),
            Self::Started { recorder } => Event::default()
                .event("timeshift.started")
                .json_data(TimeshiftStarted { recorder })
                .unwrap(),
            Self::Stopped { recorder } => Event::default()
                .event("timeshift.stopped")
                .json_data(TimeshiftStopped { recorder })
                .unwrap(),
            Self::RecordStarted {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-started")
                .json_data(TimeshiftRecordStarted {
                    recorder,
                    record_id,
                })
                .unwrap(),
            Self::RecordUpdated {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-updated")
                .json_data(TimeshiftRecordUpdated {
                    recorder,
                    record_id,
                })
                .unwrap(),
            Self::RecordEnded {
                recorder,
                record_id,
            } => Event::default()
                .event("timeshift.record-ended")
                .json_data(TimeshiftRecordEnded {
                    recorder,
                    record_id,
                })
                .unwrap(),
        }
    }
}

// on-air events

impl_emit! {crate::onair::OnairProgramChanged}

impl Into<Event> for crate::onair::OnairProgramChanged {
    fn into(self) -> Event {
        Event::default()
            .event("onair.program-changed")
            .json_data(OnairProgramChanged {
                service_id: self.service_id.into(),
            })
            .unwrap()
    }
}

// A wrapper type to send `UnregisterEmitter` messages when it's destroyed.

struct EventStreamWrapper<S> {
    inner: S,
    _tuner_event_unregister_trigger: Trigger<crate::tuner::UnregisterEmitter>,
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
