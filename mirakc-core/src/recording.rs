use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use actlet::prelude::*;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::Jst;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::BufWriter;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

use crate::command_util::spawn_pipeline;
use crate::command_util::CommandPipeline;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::epg;
use crate::epg::EpgProgram;
use crate::epg::QueryClock;
use crate::epg::QueryPrograms;
use crate::epg::QueryService;
use crate::error::Error;
use crate::filter::FilterPipelineBuilder;
use crate::models::ProgramId;
use crate::models::ServiceId;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::onair;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerStreamStopTrigger;
use crate::tuner::TunerSubscriptionId;

const EXIT_RETRY: i32 = 222;

// chrono::Duration has no const function which can be used for defining
// compile-time constants.
const PREP_SECS: i64 = 15;
const MAX_DELAY_HOURS: i64 = 15;

pub struct RecordingManager<T, E, O> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    onair_program_tracker: O,

    // We use two types for managing recording schedules.  `std` provides
    // `BTreeMap` for representing an ordered map, but it cannot provides the
    // following two functionalities:
    //
    // * A priority queue sorted by the start time of each recording schedule
    // * A hash map for a quick access to each recording schedule by a program
    //   ID
    queue: BinaryHeap<QueueItem>,
    schedules: HashMap<ProgramId, RecordingSchedule>,
    recorders: HashMap<ProgramId, Recorder>,
    timer_token: Option<CancellationToken>,

    recording_started_emitters: Vec<Emitter<RecordingStarted>>,
    recording_stopped_emitters: Vec<Emitter<RecordingStopped>>,
    recording_failed_emitters: Vec<Emitter<RecordingFailed>>,
    recording_rescheduled_emitters: Vec<Emitter<RecordingRescheduled>>,
}

impl<T, E, O> RecordingManager<T, E, O> {
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E, onair_program_tracker: O) -> Self {
        RecordingManager {
            config,
            tuner_manager,
            epg,
            onair_program_tracker,
            queue: Default::default(),
            schedules: Default::default(),
            recorders: Default::default(),
            timer_token: None,
            recording_started_emitters: Default::default(),
            recording_stopped_emitters: Default::default(),
            recording_failed_emitters: Default::default(),
            recording_rescheduled_emitters: Default::default(),
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn load_schedules(&mut self) {
        fn do_load(path: &Path) -> Result<Vec<RecordingSchedule>, Error> {
            let file = std::fs::File::open(path)?;
            Ok(serde_json::from_reader(file)?)
        }

        let basedir = match self.config.recording.basedir {
            Some(ref basedir) => basedir,
            None => return,
        };

        let path = basedir.join("schedules.json");
        if !path.exists() {
            return;
        }

        match do_load(&path) {
            Ok(schedules) => {
                tracing::info!(?path, "Loaded");
                for schedule in schedules.into_iter() {
                    let program_id = schedule.program.id;
                    // Some of schedules may be expired, but `add_schedule()` doesn't check
                    // that.
                    match self.add_schedule(schedule) {
                        Ok(_) => (),
                        Err(err @ Error::AlreadyExists) => {
                            // This error may happen when user changes schedules.json by hand.
                            // We just output the warning message, ignore the schedule and
                            // don't emit a RecordingFailed message.
                            tracing::warn!(%err, %program_id, "Already added, ignore");
                        }
                        Err(_) => unreachable!(),
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, ?path, "Failed to load");
            }
        }
    }

    fn save_schedules(&self) {
        fn do_save(schedules: Vec<&RecordingSchedule>, path: &Path) -> Result<(), Error> {
            let file = std::fs::File::create(path)?;
            serde_json::to_writer(file, &schedules)?;
            Ok(())
        }

        let basedir = match self.config.recording.basedir {
            Some(ref basedir) => basedir,
            None => return,
        };

        let path = basedir.join("schedules.json");
        let schedules = self.schedules.values().collect_vec();

        match do_save(schedules, &path) {
            Ok(_) => tracing::info!(?path, "Saved"),
            Err(err) => tracing::error!(%err, ?path, "Failed to save"),
        }
    }

    fn rebuild_queue(&mut self) {
        self.queue.clear();
        let schedules = self
            .schedules
            .values()
            .filter(|schedule| schedule.is_ready_for_recording());
        for schedule in schedules {
            self.queue.push(QueueItem {
                program_id: schedule.program.id,
                start_at: schedule.program.start_at.unwrap(),
                priority: schedule.options.priority,
            });
        }
    }

    fn set_timer<C>(&mut self, ctx: &C)
    where
        C: Spawn + EmitterFactory<ProcessRecording>,
    {
        if let Some(token) = self.timer_token.take() {
            token.cancel();
        }
        if let Some(schedule) = self.queue.peek() {
            let expires_at = schedule.start_at - Duration::seconds(PREP_SECS);
            let duration = match (expires_at - Jst::now()).to_std() {
                Ok(duration) => {
                    tracing::debug!(%expires_at, "Set timer");
                    duration
                }
                Err(_) => {
                    tracing::warn!(
                        %expires_at,
                        "Preparation time too short, \
                         the beginning of some programs may not be recorded",
                    );
                    std::time::Duration::ZERO
                }
            };
            let emitter = ctx.emitter();
            let token = ctx.spawn_task(async move {
                tokio::time::sleep(duration).await;
                emitter.emit(ProcessRecording).await;
            });
            self.timer_token = Some(token);
        }
    }
}

// actor

#[async_trait]
impl<T, E, O> Actor for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");
        self.epg
            .call(epg::RegisterEmitter::ServicesUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for epg::ServicesUpdated");
        self.epg
            .call(epg::RegisterEmitter::ProgramsUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for epg::ProgramsUpdated");
        self.onair_program_tracker
            .call(onair::RegisterEmitter(ctx.address().clone().into()))
            .await
            .expect("Failed to register emitter for OnairProgramUpdated");
        self.load_schedules();
        self.rebuild_queue();
        self.set_timer(ctx);
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// query recording schedules

#[derive(Message)]
#[reply("Vec<RecordingSchedule>")]
pub struct QueryRecordingSchedules;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedules> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryRecordingSchedules,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingSchedules");
        self.query_schedules()
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn query_schedules(&self) -> Vec<RecordingSchedule> {
        self.schedules
            .values()
            .sorted_by(|s1, s2| {
                let e1 = QueueItem {
                    program_id: s1.program.id,
                    start_at: s1.program.start_at.unwrap(),
                    priority: s1.options.priority,
                };
                let e2 = QueueItem {
                    program_id: s2.program.id,
                    start_at: s2.program.start_at.unwrap(),
                    priority: s2.options.priority,
                };
                e1.cmp(&e2).reverse()
            })
            .cloned()
            .collect_vec()
    }
}

// query recording schedule

#[derive(Message)]
#[reply("Result<RecordingSchedule, Error>")]
pub struct QueryRecordingSchedule {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingSchedule,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingSchedule", %msg.program_id);
        self.query_schedule(msg.program_id)
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn query_schedule(&self, program_id: ProgramId) -> Result<RecordingSchedule, Error> {
        self.schedules
            .get(&program_id)
            .cloned()
            .ok_or(Error::ScheduleNotFound)
    }
}

// add recording schedule

#[derive(Message)]
#[reply("Result<RecordingSchedule, Error>")]
pub struct AddRecordingSchedule {
    pub schedule: RecordingSchedule,
}

#[async_trait]
impl<T, E, O> Handler<AddRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::QueryOnairProgram>,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: AddRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <AddRecordingSchedule as Message>::Reply {
        tracing::debug!(
            msg.name = "AddRecordingSchedule",
            %msg.schedule.program.id,
            ?msg.schedule.options.content_path,
            %msg.schedule.options.priority,
            ?msg.schedule.options.pre_filters,
            ?msg.schedule.options.post_filters,
        );
        let program_id = msg.schedule.program.id;
        self.add_schedule(msg.schedule)?;
        // TODO: `schedule` should be updated before adding it.
        self.update_schedule_with_onair_program(program_id.into())
            .await;
        self.save_schedules();
        self.rebuild_queue();
        self.set_timer(ctx);
        self.query_schedule(program_id)
    }
}

impl<T, E, O> RecordingManager<T, E, O>
where
    O: Call<onair::QueryOnairProgram>,
{
    async fn update_schedule_with_onair_program(&mut self, service_id: ServiceId) {
        let msg = onair::QueryOnairProgram { service_id };
        match self.onair_program_tracker.call(msg).await {
            Ok(Ok(onair_program)) => {
                if let Some(program) = onair_program.current {
                    // The start time in EIT[p/f] may be undefined.
                    if program.start_at.is_some() {
                        self.update_schedule_by_onair_program(program).await;
                    }
                }
                if let Some(program) = onair_program.next {
                    // The start time in EIT[p/f] may be undefined.
                    if program.start_at.is_some() {
                        self.update_schedule_by_onair_program(program).await;
                    }
                }
            }
            Ok(Err(_)) => {
                // No tracker, nothing to do.
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    %service_id,
                    "Failed to get on-air program",
                );
            }
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn add_schedule(&mut self, schedule: RecordingSchedule) -> Result<(), Error> {
        let program_id = schedule.program.id;

        if self.schedules.contains_key(&program_id) {
            let err = Error::AlreadyExists;
            tracing::warn!(%err, %schedule.program.id);
            return Err(err);
        }

        // We don't check the start and end times of the TV program here and
        // simply add a schedule for it.  Additional checks will be performed
        // in later stages.
        self.schedules.insert(program_id, schedule);
        tracing::info!(
            schedule.program.id = %program_id,
            "Added",
        );

        Ok(())
    }
}

// remove recording schedule

#[derive(Message)]
#[reply("Result<RecordingSchedule, Error>")]
pub struct RemoveRecordingSchedule {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RemoveRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedule", %msg.program_id);
        let program_id = msg.program_id;
        match self.schedules.remove(&program_id) {
            Some(schedule) => {
                if schedule.is_recording() {
                    let _ = self.stop_recorder(program_id);
                }
                self.save_schedules();
                self.rebuild_queue();
                self.set_timer(ctx);
                Ok(schedule)
            }
            None => {
                tracing::warn!(
                    schedule.program.id = %program_id,
                    "No such schedule added",
                );
                Err(Error::ScheduleNotFound)
            }
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn stop_recorder(&mut self, program_id: ProgramId) -> Result<(), Error> {
        match self.recorders.get_mut(&program_id) {
            Some(recorder) => {
                match recorder.stop_trigger.take() {
                    Some(_) => {
                        tracing::info!(
                            schedule.program.id = %program_id,
                            recorder.pipeline.id = %recorder.pipeline.id(),
                            "Stop recorder pipeline",
                        );
                    }
                    None => {
                        tracing::warn!(
                            schedule.program.id = %program_id,
                            recorder.pipeline.id = %recorder.pipeline.id(),
                            "Recorder pipeline already stopped",
                        );
                    }
                }
                Ok(())
            }
            None => {
                tracing::error!(
                    schedule.program.id = %program_id,
                    "No such recorder found, maybe already stopped",
                );
                Err(Error::RecorderNotFound)
            }
        }
    }
}

// remove recording schedules

#[derive(Message)]
#[reply("()")]
pub struct RemoveRecordingSchedules {
    pub target: RemovalTarget,
}

#[derive(Debug)]
pub enum RemovalTarget {
    All,
    Tag(String),
}

#[async_trait]
impl<T, E, O> Handler<RemoveRecordingSchedules> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedules,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedules", ?msg.target);
        self.remove_schedules(msg.target, Jst::now());
        self.save_schedules();
        self.rebuild_queue();
        self.set_timer(ctx);
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn remove_schedules(&mut self, target: RemovalTarget, now: DateTime<Jst>) {
        match target {
            RemovalTarget::All => self.clear_schedules(),
            RemovalTarget::Tag(tag) => self.remove_schedules_by_tag(&tag, now),
        }
    }

    fn clear_schedules(&mut self) {
        tracing::info!("Remove all schedules");
        let schedules = std::mem::replace(&mut self.schedules, Default::default());
        for schedule in schedules.into_values() {
            if schedule.is_recording() {
                let _ = self.stop_recorder(schedule.program.id);
            }
        }
    }

    fn remove_schedules_by_tag(&mut self, tag: &str, now: DateTime<Jst>) {
        use RecordingScheduleState::*;

        // No notification message will be emitted.
        // Users know what they are doing.

        let prep_time = Duration::seconds(PREP_SECS);
        tracing::info!(tag, "Remove tagged schedules");
        self.schedules.retain(|_, schedule| {
            if !schedule.tags.contains(tag) {
                return true;
            }
            // Schedules in "Tracking" and "Recording" are retained.
            match schedule.state {
                Scheduled => {
                    let start_time = schedule.program.start_at.unwrap();
                    // Always retain schedules which will start soon
                    // (or have already started).
                    start_time - now <= prep_time
                }
                Tracking | Recording => {
                    // Always retained.
                    true
                }
                Rescheduling => {
                    // Always removed.
                    false
                }
            }
        });
    }
}

// query recording recorders

#[derive(Message)]
#[reply("Vec<RecorderModel>")]
pub struct QueryRecordingRecorders;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingRecorders> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryRecordingRecorders,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecorders as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecorders");
        self.recorders
            .iter()
            .map(|(&program_id, recorder)| recorder.get_model(program_id))
            .collect()
    }
}

// query recording recorder

#[derive(Message)]
#[reply("Result<RecorderModel, Error>")]
pub struct QueryRecordingRecorder {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingRecorder> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingRecorder,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecorder as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecorder", %msg.program_id);
        self.recorders
            .get(&msg.program_id)
            .map(|recorder| recorder.get_model(msg.program_id))
            .ok_or(Error::RecorderNotFound)
    }
}

// start recording

#[derive(Message)]
#[reply("Result<(), Error>")]
pub struct StartRecording {
    pub schedule: RecordingSchedule,
}

#[async_trait]
impl<T, E, O> Handler<StartRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: StartRecording,
        ctx: &mut Context<Self>,
    ) -> <StartRecording as Message>::Reply {
        tracing::debug!(
            msg.name = "StartRecording",
            %msg.schedule.program.id,
            ?msg.schedule.options.content_path,
            %msg.schedule.options.priority,
            ?msg.schedule.options.pre_filters,
            ?msg.schedule.options.post_filters,
        );
        let program_id = msg.schedule.program.id;
        self.add_schedule(msg.schedule)?;
        self.start_recording(program_id, ctx.address().clone(), ctx)
            .await?;
        self.save_schedules();
        Ok(())
    }
}

// process recording

#[derive(Message)]
struct ProcessRecording;

#[async_trait]
impl<T, E, O> Handler<ProcessRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, _msg: ProcessRecording, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ProcessRecording");
        let now = Jst::now();

        let mut changed = self.remove_expired_schedules(now).await;
        if changed {
            self.rebuild_queue();
        }

        let program_ids = self.dequeue_next_schedules(now);

        for program_id in program_ids.into_iter() {
            match self
                .start_recording(program_id, ctx.address().clone(), ctx)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        schedule.program.id = %program_id,
                        "Start recording",
                    );
                }
                Err(err) => {
                    tracing::error!(
                        %err,
                        schedule.program.id = %program_id,
                        "Failed to start recording",
                    );
                    self.emit_recording_failed(
                        program_id,
                        RecordingFailedReason::StartRecordingFailed {
                            message: format!("{}", err),
                        },
                    )
                    .await;
                    self.schedules.remove(&program_id);
                }
            }
            changed = true;
        }

        self.set_timer(ctx);
        if changed {
            self.save_schedules();
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn remove_expired_schedules(&mut self, now: DateTime<Jst>) -> bool {
        let max_delay = Duration::hours(MAX_DELAY_HOURS);

        let mut removed = vec![];

        self.schedules.retain(|_, schedule| {
            if schedule.is_recording() {
                return true;
            }
            if schedule.program.start_at.unwrap() + max_delay >= now {
                return true;
            }
            tracing::error!(
                %schedule.program.id,
                "Schedule expired",
            );
            removed.push(schedule.program.id);
            false
        });

        for &program_id in removed.iter() {
            self.emit_recording_failed(program_id, RecordingFailedReason::ScheduleExpired)
                .await;
        }

        !removed.is_empty()
    }

    fn dequeue_next_schedules(&mut self, now: DateTime<Jst>) -> Vec<ProgramId> {
        let mut program_ids = vec![];
        let prep_secs = Duration::seconds(PREP_SECS);
        while let Some(schedule) = self.queue.peek() {
            if schedule.start_at - now <= prep_secs {
                let schedule = self.queue.pop().unwrap();
                debug_assert!(self.schedules.contains_key(&schedule.program_id));
                program_ids.push(schedule.program_id);
            } else {
                break;
            }
        }
        program_ids
    }
}

impl<T, E, O> RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn start_recording<C: Spawn>(
        &mut self,
        program_id: ProgramId,
        addr: Address<Self>,
        ctx: &C,
    ) -> Result<(), Error> {
        if self.recorders.contains_key(&program_id) {
            return Err(Error::AlreadyExists);
        }

        // TODO: copied from web::program_stream_g().

        let service_id = program_id.into();
        let schedule = self.schedules.get(&program_id).unwrap();

        let service = self
            .epg
            .call(QueryService::ByServiceId(service_id))
            .await??;

        let clock = self.epg.call(QueryClock { service_id }).await??;

        let stream = self
            .tuner_manager
            .call(StartStreaming {
                channel: service.channel.clone(),
                user: TunerUser {
                    info: TunerUserInfo::Recorder {
                        name: format!("program#{}", program_id),
                    },
                    priority: schedule.options.priority.into(),
                },
                stream_id: None,
            })
            .await??;

        // stop_trigger must be created here in order to stop streaming when
        // an error occurs.
        let stop_trigger =
            TunerStreamStopTrigger::new(stream.id(), self.tuner_manager.clone().into());

        let video_tags: Vec<u8> = schedule
            .program
            .video
            .iter()
            .map(|video| video.component_tag)
            .collect();

        let audio_tags: Vec<u8> = schedule
            .program
            .audios
            .values()
            .map(|audio| audio.component_tag)
            .collect();

        let mut builder = mustache::MapBuilder::new();
        builder = builder
            .insert_str("channel_name", &service.channel.name)
            .insert("channel_type", &service.channel.channel_type)?
            .insert_str("channel", &service.channel.channel)
            .insert("sid", &program_id.sid().value())?
            .insert("eid", &program_id.eid().value())?
            .insert("clock_pid", &clock.pid)?
            .insert("clock_pcr", &clock.pcr)?
            .insert("clock_time", &clock.time)?
            .insert("video_tags", &video_tags)?
            .insert("audio_tags", &audio_tags)?;
        let data = builder.build();

        let mut builder = FilterPipelineBuilder::new(data);
        builder.add_pre_filters(&self.config.pre_filters, &schedule.options.pre_filters)?;
        if !stream.is_decoded() {
            builder.add_decode_filter(&self.config.filters.decode_filter)?;
        }
        builder.add_program_filter(&self.config.filters.program_filter)?;
        builder.add_post_filters(&self.config.post_filters, &schedule.options.post_filters)?;
        let (filters, _) = builder.build();

        let basedir = self.config.recording.basedir.as_ref().unwrap();

        let content_path = if schedule.options.content_path.is_absolute() {
            schedule.options.content_path.clone()
        } else {
            basedir.join(&schedule.options.content_path)
        };
        // We assumed that schedule.content_path has already been normalized.
        if let Some(dir) = content_path.parent() {
            // Create missing directories if they don't exist.
            tokio::fs::create_dir_all(dir).await?;
        }

        let mut pipeline = spawn_pipeline(filters, stream.id())?;
        let (input, mut output) = pipeline.take_endpoints()?;

        let fut = async move {
            let _ = stream.pipe(input).await;
        };
        ctx.spawn_task(fut);

        // Inner future in order to capture the result in an outer future.
        let inner_fut = {
            let content_path = content_path.clone();
            async move {
                let record = tokio::fs::File::create(&content_path).await?;
                let mut writer = BufWriter::new(record);
                // TODO: use Stdio
                Ok::<_, std::io::Error>(tokio::io::copy(&mut output, &mut writer).await?)
            }
        };
        // Outer future emits messages to observers.
        let fut = {
            async move {
                addr.emit(RecordingStarted { program_id }).await;
                let result = inner_fut.await;
                addr.emit(RecordingStopped { program_id }).await;
                if let Err(err) = result {
                    addr.emit(RecordingFailed {
                        program_id,
                        reason: RecordingFailedReason::IoError {
                            message: format!("{}", err),
                            os_error: err.raw_os_error(),
                        },
                    })
                    .await;
                }
            }
        };

        let recorder = Recorder {
            started_at: Jst::now(),
            pipeline,
            stop_trigger: Some(stop_trigger),
        };
        self.recorders.insert(program_id, recorder);
        self.schedules.get_mut(&program_id).unwrap().state = RecordingScheduleState::Recording;

        // Spawn the following task after the recorder is inserted so that
        // actors receiving RecordingStarted messages can access the recorder.
        ctx.spawn_task(fut);

        Ok(())
    }
}

// register emitter

#[derive(Message)]
#[reply("()")]
pub enum RegisterEmitter {
    RecordingStarted(Emitter<RecordingStarted>),
    RecordingStopped(Emitter<RecordingStopped>),
    RecordingFailed(Emitter<RecordingFailed>),
    RecordingRescheduled(Emitter<RecordingRescheduled>),
}

#[async_trait]
impl<T, E, O> Handler<RegisterEmitter> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        _ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        match msg {
            RegisterEmitter::RecordingStarted(emitter) => {
                self.recording_started_emitters.push(emitter);
            }
            RegisterEmitter::RecordingStopped(emitter) => {
                self.recording_stopped_emitters.push(emitter);
            }
            RegisterEmitter::RecordingFailed(emitter) => {
                self.recording_failed_emitters.push(emitter);
            }
            RegisterEmitter::RecordingRescheduled(emitter) => {
                self.recording_rescheduled_emitters.push(emitter);
            }
        }
    }
}

// recording started

#[derive(Message)]
pub struct RecordingStarted {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStarted> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingStarted, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_id);
        self.handle_recording_started(msg.program_id).await;
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn handle_recording_started(&self, program_id: ProgramId) {
        tracing::info!(
            schedule.program.id = %program_id,
            "Recording started",
        );
        for emitter in self.recording_started_emitters.iter() {
            emitter.emit(RecordingStarted { program_id }).await;
        }
    }
}

// recording stopped

#[derive(Message)]
pub struct RecordingStopped {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStopped> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingStopped, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_id);
        let changed = self.handle_recording_stopped(msg.program_id).await;
        if changed {
            self.save_schedules();
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn handle_recording_stopped(&mut self, program_id: ProgramId) -> bool {
        // The schedule may have been removed before the recording stops.
        // For example, `clear_schedules()` clears schedules before the
        // recordings stop.
        let maybe_schedule = self.schedules.remove(&program_id);
        let changed = maybe_schedule.is_some();

        // Unlike the schedule, the recorder should be removed after the
        // recording stopped.
        let recorder = self.recorders.remove(&program_id);
        match recorder {
            Some(mut recorder) => {
                // Manually drop the stop trigger so that we get the exit code
                // from the program-filter without killing its process.
                if let Some(stop_trigger) = recorder.stop_trigger.take() {
                    drop(stop_trigger);
                }

                let results = recorder.pipeline.wait().await;
                if check_retry(&results) {
                    tracing::error!(
                        schedule.program.id = %program_id,
                        "Recording stopped before the TV program starts",
                    );
                    if let Some(mut schedule) = maybe_schedule {
                        tracing::warn!(
                            %schedule.program.id,
                            "Need rescheduling",
                        );
                        schedule.state = RecordingScheduleState::Rescheduling;
                        self.schedules.insert(program_id, schedule);
                        self.emit_recording_failed(
                            program_id,
                            RecordingFailedReason::NeedRescheduling,
                        )
                        .await;
                    }
                } else if let Some(exit_code) = get_first_error(&results) {
                    tracing::error!(
                        %exit_code,
                        schedule.program.id = %program_id,
                        "The recording pipeline terminated abnormally",
                    );
                    self.emit_recording_failed(
                        program_id,
                        RecordingFailedReason::PipelineError { exit_code },
                    )
                    .await;
                }
            }
            None => {
                tracing::debug!(
                    schedule.program.id = %program_id,
                    "INCONSNSTENT: The recorder stopped before the recording stopped",
                );
            }
        }

        tracing::info!(
            schedule.program.id = %program_id,
            "Recording stopped",
        );

        // Receivers cannot access any data about the schedule.
        // It has already been removed.
        //
        // TODO: Save recording logs to a file.
        for emitter in self.recording_stopped_emitters.iter() {
            emitter.emit(RecordingStopped { program_id }).await;
        }

        changed
    }
}

// recording failed

#[derive(Message)]
pub struct RecordingFailed {
    pub program_id: ProgramId,
    pub reason: RecordingFailedReason,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RecordingFailedReason {
    #[serde(rename_all = "camelCase")]
    StartRecordingFailed {
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    IoError {
        message: String,
        os_error: Option<i32>,
    },
    #[serde(rename_all = "camelCase")]
    PipelineError {
        exit_code: i32,
    },
    NeedRescheduling,
    ScheduleExpired,
    RemovedFromEpg,
}

#[async_trait]
impl<T, E, O> Handler<RecordingFailed> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingFailed, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingFailed", %msg.program_id, ?msg.reason);
        self.emit_recording_failed(msg.program_id, msg.reason).await;
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_failed(&self, program_id: ProgramId, reason: RecordingFailedReason) {
        for emitter in self.recording_failed_emitters.iter() {
            let msg = RecordingFailed {
                program_id,
                reason: reason.clone(),
            };
            emitter.emit(msg).await;
        }
    }
}

// recording rescheduled

#[derive(Clone, Message)]
pub struct RecordingRescheduled {
    pub program_id: ProgramId,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_rescheduled(&self, program_id: ProgramId) {
        for emitter in self.recording_rescheduled_emitters.iter() {
            emitter.emit(RecordingRescheduled { program_id }).await;
        }
    }
}

// services updated

#[async_trait]
impl<T, E, O> Handler<epg::ServicesUpdated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ServicesUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        let changed = self.update_schedules_by_epg_services(&msg.services).await;
        if changed {
            self.save_schedules();
            self.rebuild_queue();
            self.set_timer(ctx);
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn update_schedules_by_epg_services(
        &mut self,
        services: &IndexMap<ServiceId, epg::EpgService>,
    ) -> bool {
        let mut removed = vec![];
        self.schedules.retain(|&program_id, schedule| {
            if services.contains_key(&ServiceId::from(program_id)) {
                return true;
            }
            tracing::warn!(%schedule.program.id, "Removed from EPG");
            removed.push(program_id);
            false
        });

        for &program_id in removed.iter() {
            if self.recorders.contains_key(&program_id) {
                let _ = self.stop_recorder(program_id);
            }
            self.emit_recording_failed(program_id, RecordingFailedReason::RemovedFromEpg)
                .await;
        }

        !removed.is_empty()
    }
}

// programs updated

#[async_trait]
impl<T, E, O> Handler<epg::ProgramsUpdated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ProgramsUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ProgramsUpdated", %msg.service_id);
        let now = Jst::now();
        let changed = self
            .update_schedules_by_epg_programs(now, msg.service_id)
            .await;
        if changed {
            self.save_schedules();
            self.rebuild_queue();
            self.set_timer(ctx);
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O>
where
    E: Call<QueryPrograms>,
{
    async fn update_schedules_by_epg_programs(
        &mut self,
        now: DateTime<Jst>,
        service_id: ServiceId,
    ) -> bool {
        let programs = match self.epg.call(QueryPrograms { service_id }).await {
            Ok(programs) => programs,
            Err(err) => {
                tracing::error!(%err, %service_id, "Failed to update schedules");
                return false;
            }
        };

        let mut changed = false;
        let mut removed = vec![];
        let mut rescheduled = vec![];
        self.schedules.retain(|&program_id, schedule| {
            if ServiceId::from(program_id) != service_id {
                return true;
            }
            if !schedule.can_be_updated_by_epg() {
                return true;
            }
            let program = match programs.get(&program_id.eid()) {
                Some(program) => program,
                None => {
                    tracing::warn!(%schedule.program.id, "Removed from EPG");
                    removed.push(program_id);
                    changed = true;
                    return false;
                }
            };
            if let Some(end_time) = program.end_at() {
                // REMARK: `programs` contains TV programs already ended.
                if end_time > now {
                    if program.start_at != schedule.program.start_at {
                        schedule.state = RecordingScheduleState::Scheduled;
                        rescheduled.push(program_id);
                    }
                    schedule.program = Arc::new(program.clone());
                    changed = true;
                }
            }
            true
        });

        for &program_id in removed.iter() {
            let reason = RecordingFailedReason::RemovedFromEpg;
            self.emit_recording_failed(program_id, reason).await;
        }

        for &program_id in rescheduled.iter() {
            self.emit_recording_rescheduled(program_id).await;
        }

        changed
    }
}

// on-air program changed

#[async_trait]
impl<T, E, O> Handler<onair::OnairProgramChanged> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: onair::OnairProgramChanged, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "OnairProgramChanged");

        let mut changed = false;

        // The start time in EIT[p/f] may be undefined.
        if let Some(program) = msg.current {
            if program.start_at.is_some() {
                changed = self.update_schedule_by_onair_program(program).await;
            }
        }
        if let Some(program) = msg.next {
            if program.start_at.is_some() {
                changed = self.update_schedule_by_onair_program(program).await;
            }
        }

        if changed {
            self.save_schedules();
            self.rebuild_queue();
            self.set_timer(ctx);
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn update_schedule_by_onair_program(&mut self, program: Arc<EpgProgram>) -> bool {
        use RecordingScheduleState::*;

        let mut rescheduled = false;

        if let Some(mut schedule) = self.schedules.get_mut(&program.id) {
            match schedule.state {
                Scheduled | Rescheduling => {
                    rescheduled = schedule.program.start_at != program.start_at;
                    schedule.program = program.clone();
                    schedule.state = Tracking;
                    tracing::info!(
                        %schedule.program.id,
                        "Start tracking changes of the schedule",
                    );
                }
                Tracking => {
                    rescheduled = schedule.program.start_at != program.start_at;
                    schedule.program = program.clone();
                }
                Recording => {
                    schedule.program = program.clone();
                }
            }
        }

        if rescheduled {
            self.emit_recording_rescheduled(program.id).await;
        }

        rescheduled
    }
}

// models

#[derive(Debug, Eq)]
struct QueueItem {
    program_id: ProgramId,
    start_at: DateTime<Jst>,
    priority: i32,
}

impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.start_at
            .cmp(&other.start_at)
            .reverse()
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl PartialEq for QueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.program_id == other.program_id
    }
}

impl PartialOrd for QueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecordingSchedule {
    pub state: RecordingScheduleState,
    pub program: Arc<EpgProgram>,
    pub options: RecordingOptions,
    pub tags: HashSet<String>,
}

impl RecordingSchedule {
    fn can_be_updated_by_epg(&self) -> bool {
        use RecordingScheduleState::*;
        match self.state {
            Scheduled | Rescheduling => true,
            _ => false,
        }
    }

    fn is_ready_for_recording(&self) -> bool {
        use RecordingScheduleState::*;
        match self.state {
            Scheduled | Tracking => true,
            _ => false,
        }
    }

    fn is_recording(&self) -> bool {
        use RecordingScheduleState::*;
        match self.state {
            Recording => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
#[derive(ToSchema)]
#[schema(title = "RecordingScheduleState")]
pub enum RecordingScheduleState {
    Scheduled,
    // This state is used only when on-air program manager is available for the
    // service of the target TV program.  In this case, the state of the
    // schedule always transits to `recording` via `tracking`.
    Tracking,
    Recording,
    // When the recording fails, the schedule transits to this state.  If on-air
    // program tracker is available, the schedule may be rescheduled when an EIT
    // section ([schedule] or [p/f]) for the target TV program is emitted.
    // Otherwise, it will be expired eventually.
    Rescheduling,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ToSchema)]
#[schema(title = "RecordingOptions")]
pub struct RecordingOptions {
    #[schema(value_type = String)]
    pub content_path: PathBuf,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub pre_filters: Vec<String>,
    #[serde(default)]
    pub post_filters: Vec<String>,
}

struct Recorder {
    started_at: DateTime<Jst>,
    pipeline: CommandPipeline<TunerSubscriptionId>,
    stop_trigger: Option<TunerStreamStopTrigger>,
}

impl Recorder {
    fn get_model(&self, program_id: ProgramId) -> RecorderModel {
        RecorderModel {
            program_id,
            started_at: self.started_at,
            pipeline: self.pipeline.get_model(),
        }
    }
}

pub struct RecorderModel {
    pub program_id: ProgramId,
    pub started_at: DateTime<Jst>,
    pub pipeline: Vec<CommandPipelineProcessModel>,
}

// helpers

fn check_retry(results: &[std::io::Result<ExitStatus>]) -> bool {
    results.iter().any(|result| match result {
        Ok(status) => {
            if let Some(EXIT_RETRY) = status.code() {
                true
            } else {
                false
            }
        }
        _ => false,
    })
}

fn get_first_error(results: &[std::io::Result<ExitStatus>]) -> Option<i32> {
    results
        .iter()
        .flat_map(|result| match result {
            Ok(status) => status.code(),
            _ => None,
        })
        .find(|&code| code != 0)
}

// <coverage:exclude>
#[cfg(test)]
#[macro_use]
mod test_macros {
    macro_rules! options {
        ($content_path:expr, $priority:expr) => {
            RecordingOptions {
                content_path: $content_path.into(),
                priority: $priority.into(),
                pre_filters: vec![],
                post_filters: vec![],
            }
        };
    }

    macro_rules! schedule {
        ($state:expr, $program:expr, $options:expr) => {
            RecordingSchedule {
                state: $state,
                program: Arc::new($program),
                options: $options,
                tags: Default::default(),
            }
        };
        ($state:expr, $program:expr, $options:expr, $tags:expr) => {
            RecordingSchedule {
                state: $state,
                program: Arc::new($program),
                options: $options,
                tags: $tags,
            }
        };
    }

    macro_rules! recorder {
        ($started_at:expr, $pipeline:expr) => {
            Recorder {
                started_at: $started_at,
                pipeline: $pipeline,
                stop_trigger: None,
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epg::stub::EpgStub;
    use crate::epg::EpgChannel;
    use crate::epg::EpgService;
    use crate::models::ChannelType;
    use crate::onair::stub::OnairProgramManagerStub;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use indexmap::indexmap;
    use maplit::hashset;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager = RecordingManager::new(
            config.clone(),
            TunerManagerStub,
            EpgStub,
            OnairProgramManagerStub,
        );

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now, "1h"),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Tracking,
            program!((0, 0, 1, 2), now, "1h"),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 3), now - Duration::hours(1), "2h"),
            options!("3.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let num_schedules = manager.schedules.len();

        manager.save_schedules();
        assert!(temp_dir.path().join("schedules.json").is_file());

        let mut manager = RecordingManager::new(
            config.clone(),
            TunerManagerStub,
            EpgStub,
            OnairProgramManagerStub,
        );
        manager.load_schedules();
        assert_eq!(manager.schedules.len(), num_schedules);
        assert!(manager.schedules.contains_key(&(0, 0, 1, 1).into()));
        assert!(manager.schedules.contains_key(&(0, 0, 1, 2).into()));
        assert!(manager.schedules.contains_key(&(0, 0, 1, 3).into()));
    }

    #[test]
    fn test_rebuild_queue() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now + Duration::hours(1)),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 2), now),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Tracking,
            program!((0, 0, 1, 3), now),
            options!("3.m2ts", 1)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 4), now - Duration::minutes(30)),
            options!("4.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        manager.rebuild_queue();
        assert_matches!(manager.queue.pop(), Some(item) => {
            assert_eq!(item.program_id, (0, 0, 1, 3).into());
        });
        assert_matches!(manager.queue.pop(), Some(item) => {
            assert_eq!(item.program_id, (0, 0, 1, 2).into());
        });
        assert_matches!(manager.queue.pop(), Some(item) => {
            assert_eq!(item.program_id, (0, 0, 1, 1).into());
        });
        assert_matches!(manager.queue.pop(), None);
    }

    #[test]
    fn test_query_schedules() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now + Duration::hours(1)),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 2), now),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Tracking,
            program!((0, 0, 1, 3), now),
            options!("3.m2ts", 1)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 4), now - Duration::minutes(30)),
            options!("4.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        assert_eq!(manager.schedules.len(), 4);

        let schedules = manager.query_schedules();
        assert_eq!(schedules.len(), 4);
        assert_matches!(schedules.get(0), Some(schedule) => {
            assert_eq!(schedule.program.id, (0, 0, 1, 4).into());
        });
        assert_matches!(schedules.get(1), Some(schedule) => {
            assert_eq!(schedule.program.id, (0, 0, 1, 3).into());
        });
        assert_matches!(schedules.get(2), Some(schedule) => {
            assert_eq!(schedule.program.id, (0, 0, 1, 2).into());
        });
        assert_matches!(schedules.get(3), Some(schedule) => {
            assert_eq!(schedule.program.id, (0, 0, 1, 1).into());
        });
    }

    #[test]
    fn test_query_schedule() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        assert_matches!(manager.query_schedule((0, 0, 1, 1).into()), Ok(schedule) => {
            assert_eq!(schedule.program.id, (0, 0, 1, 1).into());
        });
        assert_matches!(manager.query_schedule((0, 0, 1, 2).into()), Err(err) => {
            assert_matches!(err, Error::ScheduleNotFound);
        });
    }

    #[test]
    fn test_add_schedule() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now, "1h"),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        // Schedule already exists.
        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now, "1h"),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::AlreadyExists));
        assert_eq!(manager.schedules.len(), 1);

        // Adding a schedule for an ended program is allowed.
        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 2), now - Duration::hours(1), "3h"),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);

        // Adding a schedule for a program already started is allowed.
        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 3), now - Duration::hours(1), "30m"),
            options!("3.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 3);
    }

    #[test]
    fn test_remove_schedules() {
        let now = Jst::now();

        let config = config_for_test("/tmp");

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now + Duration::seconds(PREP_SECS + 1), "1h"),
            options!("1.m2ts", 0),
            hashset!["tag1".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 2), now + Duration::seconds(PREP_SECS + 1), "1h"),
            options!("2.m2ts", 0),
            hashset!["tag2".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        // Schedules which will start soon are always retained.
        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 3), now + Duration::seconds(PREP_SECS - 1), "1h"),
            options!("3.m2ts", 0),
            hashset!["tag1".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        // Schedules in "Tracking" are always retained.
        let schedule = schedule!(
            RecordingScheduleState::Tracking,
            program!((0, 0, 1, 4), now, "1h"),
            options!("4.m2ts", 0),
            hashset!["tag2".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        // Schedules in "Recording" are always retained.
        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 5), now, "1h"),
            options!("5.m2ts", 0),
            hashset!["tag2".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        // Schedules in "Rescheduling" are always removed.
        let schedule = schedule!(
            RecordingScheduleState::Rescheduling,
            program!((0, 0, 1, 6), now, "1h"),
            options!("6.m2ts", 0),
            hashset!["tag2".to_string()]
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 6);

        manager.remove_schedules(RemovalTarget::Tag("tag2".to_string()), now);
        assert_eq!(manager.schedules.len(), 4);
        assert!(manager.schedules.contains_key(&(0, 0, 1, 1).into()));
        assert!(manager.schedules.contains_key(&(0, 0, 1, 3).into()));
        assert!(manager.schedules.contains_key(&(0, 0, 1, 4).into()));
        assert!(manager.schedules.contains_key(&(0, 0, 1, 5).into()));

        manager.remove_schedules(RemovalTarget::All, now);
        assert!(manager.schedules.is_empty());
    }

    #[tokio::test]
    async fn test_handle_recording_stopped() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let mut stopped = MockRecordingStoppedValidator::new();
        stopped.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
        });
        manager
            .recording_stopped_emitters
            .push(Emitter::new(stopped));

        let mut failed = MockRecordingFailedValidator::new();
        failed.expect_emit().never();
        manager.recording_failed_emitters.push(Emitter::new(failed));

        let start_time = now - Duration::minutes(30);

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 1), start_time, "1h"),
            options!("1.m2ts", 0),
            hashset!["tag1".to_string()]
        );
        manager.schedules.insert((0, 0, 1, 1).into(), schedule);

        let recorder = recorder!(start_time, pipeline!["true"]);
        manager.recorders.insert((0, 0, 1, 1).into(), recorder);

        let changed = manager.handle_recording_stopped((0, 0, 1, 1).into()).await;
        assert!(changed);
        assert!(manager.recorders.get(&(0, 0, 1, 1).into()).is_none());
        assert_matches!(manager.schedules.get(&(0, 0, 1, 1).into()), None);
    }

    #[tokio::test]
    async fn test_handle_recording_stopped_retry() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let mut stopped = MockRecordingStoppedValidator::new();
        stopped.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
        });
        manager
            .recording_stopped_emitters
            .push(Emitter::new(stopped));

        let mut failed = MockRecordingFailedValidator::new();
        failed.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
            assert_matches!(msg.reason, RecordingFailedReason::NeedRescheduling);
        });
        manager.recording_failed_emitters.push(Emitter::new(failed));

        let start_time = now - Duration::minutes(30);

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 1), start_time, "1h"),
            options!("1.m2ts", 0),
            hashset!["tag1".to_string()]
        );
        manager.schedules.insert((0, 0, 1, 1).into(), schedule);

        let recorder = recorder!(
            start_time,
            pipeline![format!("sh -c 'exit {}'", EXIT_RETRY)]
        );
        manager.recorders.insert((0, 0, 1, 1).into(), recorder);

        let changed = manager.handle_recording_stopped((0, 0, 1, 1).into()).await;
        assert!(changed);
        assert!(manager.recorders.get(&(0, 0, 1, 1).into()).is_none());
        assert_matches!(manager.schedules.get(&(0, 0, 1, 1).into()), Some(schedule) => {
            assert_matches!(schedule.state, RecordingScheduleState::Rescheduling);
        });
    }

    #[tokio::test]
    async fn test_handle_recording_stopped_pipeline_error() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);

        let mut stopped = MockRecordingStoppedValidator::new();
        stopped.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
        });
        manager
            .recording_stopped_emitters
            .push(Emitter::new(stopped));

        let mut failed = MockRecordingFailedValidator::new();
        failed.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
            assert_matches!(
                msg.reason,
                RecordingFailedReason::PipelineError { exit_code: 1 }
            );
        });
        manager.recording_failed_emitters.push(Emitter::new(failed));

        let start_time = now - Duration::minutes(30);

        let schedule = schedule!(
            RecordingScheduleState::Recording,
            program!((0, 0, 1, 1), start_time, "1h"),
            options!("1.m2ts", 0),
            hashset!["tag1".to_string()]
        );
        manager.schedules.insert((0, 0, 1, 1).into(), schedule);

        let recorder = recorder!(start_time, pipeline!["false"]);
        manager.recorders.insert((0, 0, 1, 1).into(), recorder);

        let changed = manager.handle_recording_stopped((0, 0, 1, 1).into()).await;
        assert!(changed);
        assert!(manager.recorders.get(&(0, 0, 1, 1).into()).is_none());
        assert_matches!(manager.schedules.get(&(0, 0, 1, 1).into()), None);
    }

    #[tokio::test]
    async fn test_update_schedules_by_epg_services() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairProgramManagerStub);
        let mut mock = MockRecordingFailedValidator::new();
        mock.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 2, 1).into());
            assert_matches!(msg.reason, RecordingFailedReason::RemovedFromEpg);
        });
        manager.recording_failed_emitters.push(Emitter::new(mock));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 1), now),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 2, 1), now),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let services = indexmap! {
            (0, 0, 1).into() => service!((0, 0, 1), "test", gr!("gr", "1")),
        };
        let changed = manager.update_schedules_by_epg_services(&services).await;
        assert!(changed);
        assert_eq!(manager.schedules.len(), 1);
        assert!(manager.schedules.contains_key(&(0, 0, 1, 1).into()));
        assert!(!manager.schedules.contains_key(&(0, 0, 2, 1).into()));

        let changed = manager.update_schedules_by_epg_services(&services).await;
        assert!(!changed);
    }

    #[tokio::test]
    async fn test_update_schedules_by_epg_programs() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut epg = MockEpg::new();
        {
            let now = now;
            epg.expect_call().returning(move |_| {
                Ok(Arc::new(indexmap! {
                    1.into() => program!((0, 0, 1, 1), now, "1h"),
                }))
            });
        }

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, epg, OnairProgramManagerStub);
        let mut failed_mock = MockRecordingFailedValidator::new();
        failed_mock.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 2).into());
            assert_matches!(msg.reason, RecordingFailedReason::RemovedFromEpg);
        });
        manager
            .recording_failed_emitters
            .push(Emitter::new(failed_mock));

        let mut rescheduled_mock = MockRecordingRescheduledValidator::new();
        rescheduled_mock.expect_emit().never();
        manager
            .recording_rescheduled_emitters
            .push(Emitter::new(rescheduled_mock));

        let schedule = schedule!(
            RecordingScheduleState::Rescheduling,
            program!((0, 0, 1, 1), now),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let schedule = schedule!(
            RecordingScheduleState::Scheduled,
            program!((0, 0, 1, 2), now),
            options!("2.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let changed = manager
            .update_schedules_by_epg_programs(now, (0, 0, 1).into())
            .await;
        assert!(changed);
        assert_eq!(manager.schedules.len(), 1);
        assert!(manager.schedules.contains_key(&(0, 0, 1, 1).into()));
        assert!(!manager.schedules.contains_key(&(0, 0, 1, 2).into()));

        let changed = manager
            .update_schedules_by_epg_programs(now, (0, 0, 0).into())
            .await;
        assert!(!changed);
    }

    #[tokio::test]
    async fn test_update_schedules_by_epg_programs_rescheduled() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut epg = MockEpg::new();
        {
            let now = now;
            epg.expect_call().returning(move |_| {
                Ok(Arc::new(indexmap! {
                    1.into() => program!((0, 0, 1, 1), now, "1h"),
                }))
            });
        }

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, epg, OnairProgramManagerStub);

        let mut mock = MockRecordingRescheduledValidator::new();
        mock.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 0, 1, 1).into());
        });
        manager
            .recording_rescheduled_emitters
            .push(Emitter::new(mock));

        let schedule = schedule!(
            RecordingScheduleState::Rescheduling,
            program!((0, 0, 1, 1), now - Duration::minutes(30), "1h"),
            options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));

        let changed = manager
            .update_schedules_by_epg_programs(now, (0, 0, 1).into())
            .await;
        assert!(changed);
        assert_eq!(manager.schedules.len(), 1);
        assert_matches!(manager.schedules.get(&(0, 0, 1, 1).into()), Some(schedule) => {
            assert_matches!(schedule.state, RecordingScheduleState::Scheduled);
        });
    }

    #[tokio::test]
    async fn test_check_retry() {
        // exit(0)
        let mut pipeline: CommandPipeline<u8> = pipeline!["true"];
        let results = pipeline.wait().await;
        assert!(!check_retry(&results));

        // exit(1)
        let mut pipeline: CommandPipeline<u8> = pipeline!["false"];
        let results = pipeline.wait().await;
        assert!(!check_retry(&results));

        // no such command
        let mut pipeline: CommandPipeline<u8> = pipeline!["sh -c 'command_not_fond'"];
        let results = pipeline.wait().await;
        assert!(!check_retry(&results));

        // retry
        let mut pipeline: CommandPipeline<u8> = pipeline![format!("sh -c 'exit {}'", EXIT_RETRY)];
        let results = pipeline.wait().await;
        assert!(check_retry(&results));
    }

    #[tokio::test]
    async fn test_recorder_get_first_error() {
        // exit(0)
        let mut pipeline: CommandPipeline<u8> = pipeline!["true"];
        let results = pipeline.wait().await;
        assert_matches!(get_first_error(&results), None);

        // exit(1)
        let mut pipeline: CommandPipeline<u8> = pipeline!["false"];
        let results = pipeline.wait().await;
        assert_matches!(get_first_error(&results), Some(1));
    }

    fn config_for_test<P: AsRef<Path>>(dir: P) -> Arc<Config> {
        let mut config = Config::default();
        config.recording.basedir = Some(dir.as_ref().to_owned());
        Arc::new(config)
    }

    mockall::mock! {
        Epg {}

        #[async_trait]
        impl Call<QueryPrograms> for Epg {
            async fn call(&self, msg: QueryPrograms) -> actlet::Result<<QueryPrograms as Message>::Reply>;
        }
    }

    mockall::mock! {
        RecordingStoppedValidator {}

        #[async_trait]
        impl Emit<RecordingStopped> for RecordingStoppedValidator {
            async fn emit(&self, msg: RecordingStopped);
        }
    }

    mockall::mock! {
        RecordingFailedValidator {}

        #[async_trait]
        impl Emit<RecordingFailed> for RecordingFailedValidator {
            async fn emit(&self, msg: RecordingFailed);
        }
    }

    mockall::mock! {
        RecordingRescheduledValidator {}

        #[async_trait]
        impl Emit<RecordingRescheduled> for RecordingRescheduledValidator {
            async fn emit(&self, msg: RecordingRescheduled);
        }
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    #[derive(Clone)]
    pub(crate) struct RecordingManagerStub;

    #[async_trait]
    impl Call<AddRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: AddRecordingSchedule,
        ) -> actlet::Result<<AddRecordingSchedule as Message>::Reply> {
            match msg.schedule.program.id.eid().value() {
                // 0 is reserved for Error::ProgramNotFound
                1 => Ok(Err(Error::AlreadyExists)),
                2 => Ok(Err(Error::ProgramEnded)),
                _ => Ok(Ok(msg.schedule)),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: QueryRecordingSchedule,
        ) -> actlet::Result<<QueryRecordingSchedule as Message>::Reply> {
            let mut program = EpgProgram::new(msg.program_id);
            program.start_at = Some(Jst::now());
            program.duration = Some(Duration::minutes(1));
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::ProgramNotFound)),
                _ => Ok(Ok(schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(msg.program_id, Jst::now(), "1m"),
                    options!("test.m2ts", 1)
                ))),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingSchedules> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: QueryRecordingSchedules,
        ) -> actlet::Result<<QueryRecordingSchedules as Message>::Reply> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl Call<RegisterEmitter> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: RegisterEmitter,
        ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
            Ok(())
        }
    }

    #[async_trait]
    impl Call<RemoveRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: RemoveRecordingSchedule,
        ) -> actlet::Result<<RemoveRecordingSchedule as Message>::Reply> {
            let mut program = EpgProgram::new(msg.program_id);
            program.start_at = Some(Jst::now());
            program.duration = Some(Duration::minutes(1));
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::ScheduleNotFound)),
                _ => Ok(Ok(schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(msg.program_id, Jst::now(), "1m"),
                    options!("test.m2ts", 1)
                ))),
            }
        }
    }

    #[async_trait]
    impl Call<RemoveRecordingSchedules> for RecordingManagerStub {
        async fn call(
            &self,
            msg: RemoveRecordingSchedules,
        ) -> actlet::Result<<RemoveRecordingSchedules as Message>::Reply> {
            match msg.target {
                RemovalTarget::All => Ok(()),
                RemovalTarget::Tag(tag) => {
                    assert_eq!(tag, "tag");
                    Ok(())
                }
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingRecorder> for RecordingManagerStub {
        async fn call(
            &self,
            msg: QueryRecordingRecorder,
        ) -> actlet::Result<<QueryRecordingRecorder as Message>::Reply> {
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(RecorderModel {
                    program_id: msg.program_id,
                    started_at: Jst::now(),
                    pipeline: vec![],
                })),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingRecorders> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: QueryRecordingRecorders,
        ) -> actlet::Result<<QueryRecordingRecorders as Message>::Reply> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl Call<StartRecording> for RecordingManagerStub {
        async fn call(
            &self,
            msg: StartRecording,
        ) -> actlet::Result<<StartRecording as Message>::Reply> {
            match msg.schedule.program.id.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(())),
            }
        }
    }
}
// </coverage:exclude>
