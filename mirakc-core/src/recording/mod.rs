mod content_source;
mod sha256;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod stub;

use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;

use actlet::prelude::*;
use bytes::Bytes;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::Jst;
use chrono_jst::serde::duration_milliseconds_option;
use chrono_jst::serde::ts_milliseconds;
use chrono_jst::serde::ts_milliseconds_option;
use indexmap::IndexMap;
use itertools::Itertools;
use path_dedot::ParseDot;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::BufWriter;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

use crate::command_util::CommandPipeline;
use crate::command_util::CommandPipelineBuilder;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::epg;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::QueryClock;
use crate::epg::QueryPrograms;
use crate::epg::QueryService;
use crate::error::Error;
use crate::file_util;
use crate::filter::FilterPipelineBuilder;
use crate::models::ContentRange;
use crate::models::ProgramId;
use crate::models::ServiceId;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::onair;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerSubscriptionId;

use content_source::ContentSource;
use sha256::CalculateSha256;
use sha256::Sha256Calculator;

const EXIT_RETRY: i32 = 222;

// chrono::Duration has no const function which can be used for defining
// compile-time constants.
const PREP_SECS: i64 = 15;
const MAX_DELAY_HOURS: i64 = 15;
const MAX_HISTORY_RETAIN_HOURS: i64 = 72;

pub struct RecordingManager<T, E, O> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    onair_program_tracker: O,

    sha256_calculator: Option<Address<Sha256Calculator>>,

    // We use two types for managing recording schedules.  `std` provides
    // `BTreeMap` for representing an ordered map, but it cannot provides the
    // following two functionalities:
    //
    // * A priority queue sorted by the start time of each recording schedule
    // * A hash map for a quick access to each recording schedule by a program
    //   ID
    queue: BinaryHeap<QueueItem>,
    schedules: HashMap<ProgramId, RecordingSchedule>,
    history: Vec<RecordingSchedule>,
    recorders: HashMap<ProgramId, Recorder>,
    timer_token: Option<CancellationToken>,

    recording_started: EmitterRegistry<RecordingStarted>,
    recording_stopped: EmitterRegistry<RecordingStopped>,
    recording_failed: EmitterRegistry<RecordingFailed>,
    recording_rescheduled: EmitterRegistry<RecordingRescheduled>,
    record_saved: EmitterRegistry<RecordSaved>,
    record_removed: EmitterRegistry<RecordRemoved>,
    content_removed: EmitterRegistry<ContentRemoved>,
    record_broken: EmitterRegistry<RecordBroken>,
    content_sha256_calculated: EmitterRegistry<ContentSha256Calculated>,
}

impl<T, E, O> RecordingManager<T, E, O> {
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E, onair_program_tracker: O) -> Self {
        RecordingManager {
            config,
            tuner_manager,
            epg,
            onair_program_tracker,
            sha256_calculator: None,
            queue: Default::default(),
            schedules: Default::default(),
            history: Default::default(),
            recorders: Default::default(),
            timer_token: None,
            recording_started: Default::default(),
            recording_stopped: Default::default(),
            recording_failed: Default::default(),
            recording_rescheduled: Default::default(),
            record_saved: Default::default(),
            record_broken: Default::default(),
            record_removed: Default::default(),
            content_removed: Default::default(),
            content_sha256_calculated: Default::default(),
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn load_schedules(&mut self) {
        fn do_load(path: &Path) -> Result<Vec<RecordingSchedule>, Error> {
            let file = std::fs::File::open(path)?;
            Ok(serde_json::from_reader(file)?)
        }

        let path = match make_schedules_path(&self.config) {
            Some(path) if path.exists() => path,
            _ => return,
        };

        match do_load(&path) {
            Ok(schedules) => {
                tracing::info!(?path, "Loaded");
                for schedule in schedules.into_iter() {
                    if schedule.is_done() {
                        self.history.push(schedule);
                    } else {
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
            }
            Err(err) => {
                tracing::warn!(%err, ?path, "Failed to load");
            }
        }
    }

    fn save_schedules(&self) {
        let path = match make_schedules_path(&self.config) {
            Some(path) => path,
            None => return,
        };

        let schedules = self
            .schedules
            .values()
            .chain(self.history.iter())
            .collect_vec();
        if file_util::save_json(&schedules, &path) {
            tracing::info!(schedules.len = schedules.len(), "Saved schedules");
        } else {
            tracing::error!("Failed to save schedules");
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
            let expires_at = schedule.start_at - Duration::try_seconds(PREP_SECS).unwrap();
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
            let (_, token) = ctx.spawn_task(async move {
                tokio::time::sleep(duration).await;
                emitter.emit(ProcessRecording).await;
            });
            self.timer_token = Some(token);
        }
    }

    async fn create_record(&self, record_path: &Path, record: &Record) {
        tracing::info!(?record_path, "Creating record...");

        if record_path.exists() {
            tracing::warn!(?record_path, "Already exists, will be overwritten");
        }

        if file_util::save_json(record, record_path) {
            tracing::info!(?record_path, "Created successfully");
            self.emit_record_saved(record.id.clone(), record.recording_status.clone())
                .await;
        } else {
            tracing::error!(?record_path, "Failed to save");
            self.emit_record_broken(record.id.clone(), "Failed to save record")
                .await;
        }
    }

    // Update record aggressively.
    // If the record has been broken, it will be overwritten if possible.
    async fn update_record(&self, program_id: ProgramId) {
        let recorder = match self.recorders.get(&program_id) {
            Some(recorder) => recorder,
            None => {
                tracing::error!(
                    schedule.program.id = %program_id,
                    "INCONSNSTENT: The recorder stopped before the recording stopped",
                );
                return;
            }
        };

        let record_id = RecordId::from((recorder.started_at, program_id));
        let record_path = match make_record_path(&self.config, &record_id) {
            Some(record_path) => record_path,
            None => {
                // The records feature is disabled.
                return;
            }
        };

        tracing::info!(?record_path, "Updating record...");

        let record = match load_record(&self.config, &record_path).await {
            Ok((mut record, _)) => {
                if !matches!(record.recording_status, RecordingStatus::Recording) {
                    tracing::warn!(
                        ?record_path,
                        "Recording status has been broken, will be overwritten"
                    );
                }
                match self.schedules.get(&program_id) {
                    Some(schedule) => {
                        if matches!(
                            schedule.state,
                            RecordingScheduleState::Scheduled | RecordingScheduleState::Tracking
                        ) {
                            tracing::error!(?record_path, "inconsistent");
                        }
                        record.update_by_schedule(schedule).await;
                    }
                    None => {
                        // The schedule may have been removed before this method is called.
                        // For example, `clear_schedules()` clears schedules before the
                        // recordings stop.
                        tracing::warn!(?record_path, "No recording schedule, maybe canceled");
                        record.recording_status = RecordingStatus::Canceled;
                        record.recording_end_time = None;
                        record.recording_duration = None;
                    }
                }
                record
            }
            Err(err) => match self.schedules.get(&program_id) {
                Some(schedule) => {
                    tracing::error!(
                        ?err,
                        ?record_path,
                        "Broken record, will be overwritten with new record"
                    );
                    let content_path =
                        make_content_path_from_schedule(&self.config, &record_id, schedule);
                    let content_path = make_relative_content_path(&self.config, &content_path);
                    Record::new(
                        record_id,
                        recorder.started_at,
                        schedule,
                        content_path,
                        recorder.content_type.clone(),
                    )
                }
                None => {
                    tracing::error!(?err, ?record_path, "Broken record, skip updating");
                    self.emit_record_broken(record_id, "Broken record").await;
                    return;
                }
            },
        };

        if record.is_done() {
            if let Some(ref addr) = self.sha256_calculator {
                let content_path = make_content_path(&self.config, &record).unwrap();
                addr.emit(CalculateSha256 {
                    record_id: record.id.clone(),
                    content_path,
                })
                .await;
            }
        }

        if file_util::save_json(&record, &record_path) {
            tracing::info!(?record_path, "Updated successfully");
            self.emit_record_saved(record.id, record.recording_status)
                .await;
        } else {
            tracing::error!(?record_path, "Failed to save");
            self.emit_record_broken(record.id, "Faild to save record")
                .await;
        }
    }

    // For testing purposes, we pass an emitter instead of using self.sha256calculator directly in
    // this method.
    async fn check_records(&self, emitter: Emitter<CalculateSha256>) {
        debug_assert!(self.config.recording.is_records_api_enabled());
        let records_dir = self.config.recording.records_dir.as_ref().unwrap();

        for record_path in glob_records(records_dir) {
            match load_record(&self.config, &record_path).await {
                Ok((record, _)) if record.content_sha256.is_some() => {
                    // The content SHA-256 has already been calculated.
                }
                Ok((record, Some(_))) => {
                    // The content file exists, but its SHA-256 has not yet been calculated.
                    // Send a CalculateSha256 message to Sha256Calculator.
                    let content_path = make_content_path(&self.config, &record).unwrap();
                    emitter
                        .emit(CalculateSha256 {
                            record_id: record.id.clone(),
                            content_path,
                        })
                        .await;
                }
                Ok((_record, None)) => {
                    // No content file, nothing to do.
                }
                Err(err) => {
                    // The record has been broken.
                    const MSG: &str = "Failed to load, maybe broken";
                    tracing::error!(?record_path, ?err, MSG);
                    let record_id = RecordId::from_record_path(&record_path);
                    self.emit_record_broken(record_id, MSG).await;
                }
            }
        }
    }
}

// actor

#[async_trait]
impl<T, E, O> Actor for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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

        if !self.config.recording.is_enabled() {
            tracing::info!("Recording is disabled");
            return;
        }

        if self.config.recording.is_records_api_enabled() {
            self.sha256_calculator =
                Some(ctx.spawn_actor(Sha256Calculator::new(ctx.emitter())).await);
        }

        if let Err(err) = self
            .epg
            .call(epg::RegisterEmitter::ServicesUpdated(ctx.emitter()))
            .await
        {
            tracing::error!(?err, "Failed to register emitter for epg::ServicesUpdated");
            return;
        }

        if let Err(err) = self
            .epg
            .call(epg::RegisterEmitter::ProgramsUpdated(ctx.emitter()))
            .await
        {
            tracing::error!(?err, "Failed to register emitter for epg::ProgramsUpdated");
            return;
        }

        if let Err(err) = self
            .onair_program_tracker
            .call(onair::RegisterEmitter(ctx.emitter()))
            .await
        {
            tracing::error!(?err, "Failed to register emitter for OnairProgramUpdated");
            return;
        }

        self.load_schedules();
        self.rebuild_queue();
        self.set_timer(ctx);

        if self.config.recording.is_records_api_enabled() {
            debug_assert!(self.sha256_calculator.is_some());
            let emitter = self.sha256_calculator.as_ref().unwrap().emitter();
            self.check_records(emitter).await;
        }
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopping...");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// query recording schedules

#[derive(Message)]
#[reply(Vec<RecordingSchedule>)]
pub struct QueryRecordingSchedules;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedules> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply(Result<RecordingSchedule, Error>)]
pub struct QueryRecordingSchedule {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply(Result<RecordingSchedule, Error>)]
pub struct AddRecordingSchedule {
    pub schedule: RecordingSchedule,
}

#[async_trait]
impl<T, E, O> Handler<AddRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply(Result<RecordingSchedule, Error>)]
pub struct RemoveRecordingSchedule {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RemoveRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply()]
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
    T: TriggerFactory<StopStreaming>,
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
        let schedules = std::mem::take(&mut self.schedules);
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

        let prep_time = Duration::try_seconds(PREP_SECS).unwrap();
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
                Rescheduling | Finished | Failed => {
                    // Always removed.
                    false
                }
            }
        });
    }
}

// query recording history

#[derive(Message)]
#[reply(Vec<RecordingSchedule>)]
pub struct QueryRecordingHistory;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingHistory> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        _msg: QueryRecordingHistory,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingHistory");
        self.query_history()
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn query_history(&self) -> Vec<RecordingSchedule> {
        self.history.clone()
    }
}

// delete recording history

#[derive(Message)]
#[reply()]
pub struct DeleteRecordingHistory;

#[async_trait]
impl<T, E, O> Handler<DeleteRecordingHistory> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        _msg: DeleteRecordingHistory,
        _ctx: &mut Context<Self>,
    ) -> <DeleteRecordingHistory as Message>::Reply {
        tracing::debug!(msg.name = "DeleteRecordingHistory");
        self.delete_history();
        self.save_schedules();
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn delete_history(&mut self) {
        self.history.clear();
    }
}

// query recording recorders

#[derive(Message)]
#[reply(Vec<RecorderModel>)]
pub struct QueryRecordingRecorders;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingRecorders> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply(Result<RecorderModel, Error>)]
pub struct QueryRecordingRecorder {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingRecorder> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
#[reply(Result<(), Error>)]
pub struct StartRecording {
    pub schedule: RecordingSchedule,
}

#[async_trait]
impl<T, E, O> Handler<StartRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
            .await;
        self.save_schedules();
        Ok(())
    }
}

// stop recording

#[derive(Message)]
#[reply(Result<(), Error>)]
pub struct StopRecording {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<StopRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        msg: StopRecording,
        _ctx: &mut Context<Self>,
    ) -> <StopRecording as Message>::Reply {
        tracing::debug!(
            msg.name = "StopRecording",
            %msg.program_id,
        );
        // This function doesn't wait for the recorder to stop completely.
        //
        // Dropping the stop trigger will stop the tuner pipeline and this
        // will cause EOF in the recording pipeline.  A `RecordingStopped`
        // signal will be emitted, and then it will be handled in the handler
        // as usual.
        self.recorders
            .get_mut(&msg.program_id)
            .ok_or(Error::RecorderNotFound)?
            .stop_trigger
            .take();
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
    T: TriggerFactory<StopStreaming>,
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

        let mut changed = false;

        if self.maintain_schedules(now).await {
            self.rebuild_queue();
            changed = true;
        }

        let program_ids = self.dequeue_next_schedules(now);
        if !program_ids.is_empty() {
            changed = true;
        }

        for program_id in program_ids.into_iter() {
            self.start_recording(program_id, ctx.address().clone(), ctx)
                .await;
        }

        if self.maintain_history(now) {
            changed = true;
        }

        self.set_timer(ctx);
        if changed {
            self.save_schedules();
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn maintain_schedules(&mut self, now: DateTime<Jst>) -> bool {
        use RecordingScheduleState::*;

        let max_delay = Duration::try_hours(MAX_DELAY_HOURS).unwrap();

        let len = self.schedules.len();
        let mut expired = vec![];

        self.schedules.retain(|_, schedule| {
            if schedule.is_recording() {
                return true;
            }
            if schedule.program.start_at.unwrap() + max_delay > now {
                return true;
            }
            match schedule.state {
                Scheduled | Tracking | Rescheduling => {
                    tracing::error!(
                        %schedule.program.id,
                        "Schedule expired",
                    );
                    expired.push(schedule.program.id);
                }
                _ => {
                    tracing::error!(
                        %schedule.program.id,
                        schedule.state = ?schedule.state,
                        "INCONSNSTENT: Invalid schedule.state",
                    );
                    panic!();
                }
            }
            false
        });

        for &program_id in expired.iter() {
            self.emit_recording_failed(program_id, RecordingFailedReason::ScheduleExpired)
                .await;
        }

        self.schedules.len() != len
    }

    fn dequeue_next_schedules(&mut self, now: DateTime<Jst>) -> Vec<ProgramId> {
        let mut program_ids = vec![];
        let prep_secs = Duration::try_seconds(PREP_SECS).unwrap();
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

    fn maintain_history(&mut self, now: DateTime<Jst>) -> bool {
        let max_delay = Duration::try_hours(MAX_HISTORY_RETAIN_HOURS).unwrap();

        let len = self.history.len();

        self.history.retain(|schedule| {
            use RecordingScheduleState::*;
            debug_assert!(matches!(schedule.state, Finished | Failed));
            schedule.program.start_at.unwrap() + max_delay > now
        });

        // No SSE event is sent.

        self.history.len() != len
    }
}

impl<T, E, O> RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
    ) {
        match self.do_start_recording(program_id, addr, ctx).await {
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
                let reason = RecordingFailedReason::StartRecordingFailed {
                    message: format!("{err}"),
                };
                if let Some(schedule) = self.schedules.get_mut(&program_id) {
                    schedule.state = RecordingScheduleState::Failed;
                    schedule.failed_reason = Some(reason.clone());
                }
                self.emit_recording_failed(program_id, reason).await;
            }
        }
    }

    async fn do_start_recording<C: Spawn>(
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

        let clock = self.epg.call(QueryClock { service_id }).await??;

        let stream = self
            .tuner_manager
            .call(StartStreaming {
                channel: schedule.service.channel.clone(),
                user: TunerUser {
                    info: TunerUserInfo::Recorder(program_id),
                    priority: schedule.options.priority.into(),
                },
                stream_id: None,
            })
            .await??;

        // stop_trigger must be created here in order to stop streaming when
        // an error occurs.
        let msg = StopStreaming { id: stream.id() };
        let stop_trigger = self.tuner_manager.trigger(msg);

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
            .insert_str("channel_name", &schedule.service.channel.name)
            .insert("channel_type", &schedule.service.channel.channel_type)?
            .insert_str("channel", &schedule.service.channel.channel)
            .insert("sid", &program_id.sid().value())?
            .insert("eid", &program_id.eid().value())?
            .insert("clock_pid", &clock.pid)?
            .insert("clock_pcr", &clock.pcr)?
            .insert("clock_time", &clock.time)?
            .insert("video_tags", &video_tags)?
            .insert("audio_tags", &audio_tags)?;
        let data = builder.build();

        let mut builder = FilterPipelineBuilder::new(data, false);
        builder.add_pre_filters(&self.config.pre_filters, &schedule.options.pre_filters)?;
        if !stream.is_decoded() {
            builder.add_decode_filter(&self.config.filters.decode_filter)?;
        }
        builder.add_program_filter(&self.config.filters.program_filter)?;
        builder.add_post_filters(&self.config.post_filters, &schedule.options.post_filters)?;
        let (filters, content_type, _) = builder.build();

        let now = Jst::now();
        let record_id = RecordId::from((now, program_id));

        let content_path = make_content_path_from_schedule(&self.config, &record_id, schedule);
        // We assumed that schedule.content_path has already been normalized.
        if let Some(dir) = content_path.parent() {
            // Create missing directories if they don't exist.
            tokio::fs::create_dir_all(dir).await?;
        }

        let log_filter = schedule.options.log_filter.as_ref().or(self
            .config
            .recording
            .log_filter
            .as_ref());

        let mut builder = CommandPipelineBuilder::new(filters, stream.id(), "recording");
        match log_filter {
            Some(log_filter) => {
                builder.set_log_filter(log_filter);
                match log_filter.as_str() {
                    "" | "off" => {
                        builder.set_logging(false); // Disable logging
                    }
                    _ => {
                        builder.set_logging(true); // Enable logging
                        let log_path = make_log_path_from_content_path(&content_path);
                        builder.set_log_file(log_path);
                    }
                }
            }
            None => {
                // logging to STDOUT (compatible with 3.x and older).
            }
        }
        let mut pipeline = builder.build(ctx)?;
        let (input, mut output) = pipeline.take_endpoints();

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
                tokio::io::copy(&mut output, &mut writer).await
            }
        };
        // Outer future emits messages to observers.
        let fut = {
            async move {
                addr.emit(RecordingStarted { program_id }).await;
                let _stopped = addr.trigger(RecordingStopped { program_id });
                let result = inner_fut.await;
                if let Err(err) = result {
                    addr.emit(RecordingFailed {
                        program_id,
                        reason: RecordingFailedReason::IoError {
                            message: format!("{err}"),
                            os_error: err.raw_os_error(),
                        },
                    })
                    .await;
                }
            }
        };

        let recorder = Recorder {
            started_at: now,
            pipeline,
            stop_trigger: Some(stop_trigger),
            content_type: content_type.clone(),
        };
        self.recorders.insert(program_id, recorder);
        self.schedules.get_mut(&program_id).unwrap().state = RecordingScheduleState::Recording;

        // Spawn the following task after the recorder is inserted so that
        // actors receiving RecordingStarted messages can access the recorder.
        ctx.spawn_task(fut);

        let record_path = make_record_path(&self.config, &record_id);
        if let Some(record_path) = record_path {
            let schedule = self.schedules.get(&program_id).unwrap();
            let content_path = make_relative_content_path(&self.config, &content_path);
            let record = Record::new(record_id, now, schedule, content_path, content_type);
            self.create_record(&record_path, &record).await;
        }

        Ok(())
    }
}

// query records

#[derive(Message)]
#[reply(Result<Vec<(Record, Option<u64>)>, Error>)]
pub struct QueryRecords;

#[async_trait]
impl<T, E, O> Handler<QueryRecords> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        _msg: QueryRecords,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecords as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecords");
        self.query_records().await
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn query_records(&self) -> Result<Vec<(Record, Option<u64>)>, Error> {
        let records_dir = match self.config.recording.records_dir.as_ref() {
            Some(records_dir) => records_dir,
            None => return Err(Error::WrongConfig("config.recording.records-dir")),
        };

        let mut records = vec![];
        for record_path in glob_records(records_dir) {
            match load_record(&self.config, &record_path).await {
                Ok(tuple) => records.push(tuple),
                Err(err) => tracing::warn!(?record_path, ?err, "Failed to load, skip"),
            }
        }

        Ok(records)
    }
}

// query record

#[derive(Message)]
#[reply(Result<(Record, Option<u64>), Error>)]
pub struct QueryRecord {
    pub id: RecordId,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecord> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        msg: QueryRecord,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecord as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecord", %msg.id);
        self.query_record(&msg.id).await
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn query_record(&self, id: &RecordId) -> Result<(Record, Option<u64>), Error> {
        let record_path = match make_record_path(&self.config, id) {
            Some(record_path) => record_path,
            None => return Err(Error::WrongConfig("config.recording.records-dir")),
        };

        load_record(&self.config, &record_path).await
    }
}

// remove record

#[derive(Message)]
#[reply(Result<(bool, bool), Error>)]
pub struct RemoveRecord {
    pub id: RecordId,
    pub purge: bool,
}

#[async_trait]
impl<T, E, O> Handler<RemoveRecord> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        msg: RemoveRecord,
        _ctx: &mut Context<Self>,
    ) -> <RemoveRecord as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecord", %msg.id, msg.purge);
        self.remove_record(&msg.id, msg.purge).await
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn remove_record(&self, id: &RecordId, purge: bool) -> Result<(bool, bool), Error> {
        let record_path = match make_record_path(&self.config, id) {
            Some(record_path) => record_path,
            None => return Err(Error::WrongConfig("config.recording.records-dir")),
        };

        let (record, _) = load_record(&self.config, &record_path).await?;

        if matches!(record.recording_status, RecordingStatus::Recording) {
            tracing::warn!(
                ?record_path,
                "Cannot remove the record while it's recording"
            );
            return Err(Error::InvalidRequest("Now recording"));
        }

        let mut record_removed = false;
        let mut content_removed = false;

        if purge {
            let content_path = make_content_path(&self.config, &record).unwrap();
            if content_path.exists() {
                match tokio::fs::remove_file(&content_path).await {
                    Ok(_) => {
                        content_removed = true;
                        self.emit_content_removed(id.clone()).await;
                    }
                    Err(err) => tracing::error!(?err, ?content_path, "Failed to remove"),
                }
            } else {
                tracing::warn!(?record_path, ?content_path, "No content file");
                content_removed = true;
            }

            let log_path = make_log_path_from_content_path(&content_path);
            if log_path.exists() {
                if let Err(err) = tokio::fs::remove_file(&log_path).await {
                    tracing::error!(?err, ?log_path, "Failed to remove");
                }
            }
        }

        debug_assert!(record_path.exists());
        match tokio::fs::remove_file(&record_path).await {
            Ok(_) => {
                record_removed = true;
                self.emit_record_removed(id.clone()).await;
            }
            Err(err) => tracing::error!(?err, ?record_path),
        }

        Ok((record_removed, content_removed))
    }
}

// open content

type BoxedStream = Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>;
type ContentStream = MpegTsStream<RecordId, BoxedStream>;
type StopTrigger = Trigger<actlet::Stop>;

#[derive(Message)]
#[reply(Result<(ContentStream, Option<StopTrigger>), Error>)]
pub struct OpenContent {
    pub id: RecordId,
    pub range: Option<ContentRange>,
    pub time_limit: u64,
}

impl OpenContent {
    // See the `tail` command used in `ContentSource::new()` for the reason why the time limit is
    // 1500ms.
    const DEFAULT_TIME_LIMIT: u64 = 1500;

    pub fn new(id: RecordId, range: Option<ContentRange>) -> Self {
        Self {
            id,
            range,
            time_limit: Self::DEFAULT_TIME_LIMIT,
        }
    }
}

#[async_trait]
impl<T, E, O> Handler<OpenContent> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        msg: OpenContent,
        ctx: &mut Context<Self>,
    ) -> <OpenContent as Message>::Reply {
        tracing::debug!(msg.name = "OpenContent", %msg.id);
        self.open_content(&msg.id, msg.range.as_ref(), msg.time_limit, ctx)
            .await
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn open_content(
        &self,
        id: &RecordId,
        range: Option<&ContentRange>,
        time_limit: u64,
        ctx: &Context<Self>,
    ) -> Result<(ContentStream, Option<StopTrigger>), Error> {
        let record_path = match make_record_path(&self.config, id) {
            Some(record_path) => record_path,
            None => return Err(Error::WrongConfig("config.recording.records-dir")),
        };

        let (record, content_length) = load_record(&self.config, &record_path).await?;
        let _content_length = match content_length {
            Some(content_length) if content_length > 0 => content_length,
            _ => return Err(Error::NoContent),
        };

        let mut content_source = ContentSource::new(&self.config, &record, range, ctx).await?;
        let stream = content_source.create_stream(time_limit);

        let addr = ctx.spawn_actor(content_source).await;
        let stop_trigger = Some(addr.trigger(actlet::Stop));

        Ok((stream, stop_trigger))
    }
}

// register emitter

#[derive(Message)]
#[reply(usize)]
pub enum RegisterEmitter {
    RecordingStarted(Emitter<RecordingStarted>),
    RecordingStopped(Emitter<RecordingStopped>),
    RecordingFailed(Emitter<RecordingFailed>),
    RecordingRescheduled(Emitter<RecordingRescheduled>),
    RecordSaved(Emitter<RecordSaved>),
    RecordRemoved(Emitter<RecordRemoved>),
    ContentRemoved(Emitter<ContentRemoved>),
    RecordBroken(Emitter<RecordBroken>),
    ContentSha256Calculated(Emitter<ContentSha256Calculated>),
}

#[async_trait]
impl<T, E, O> Handler<RegisterEmitter> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        debug_assert!(self.config.recording.is_enabled());
        match msg {
            RegisterEmitter::RecordingStarted(emitter) => {
                let id = self.recording_started.register(emitter.clone());
                tracing::debug!(msg.name = "RegisterEmitter::RecordingStarted", id);
                if id != 0 {
                    // Sending many messages in the message handler may cause a dead lock
                    // when the number of messages to be sent is larger than the capacity
                    // of the emitter's channel.  See the issue #705 for example.
                    ctx.set_post_process(RegisterEmitterPostProcess(emitter));
                }
                id
            }
            RegisterEmitter::RecordingStopped(emitter) => {
                let id = self.recording_stopped.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordingStopped", id);
                id
            }
            RegisterEmitter::RecordingFailed(emitter) => {
                let id = self.recording_failed.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordingFailed", id);
                id
            }
            RegisterEmitter::RecordingRescheduled(emitter) => {
                let id = self.recording_rescheduled.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordingRescheduled", id);
                id
            }
            RegisterEmitter::RecordSaved(emitter) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                // Create a task to send messages.
                //
                // Sending many messages in the message handler may cause a dead lock
                // when the number of messages to be sent is larger than the capacity
                // of the emitter's channel.  See the issue #705 for example.
                let task =
                    Self::emit_record_saved_for_each_record(self.config.clone(), emitter.clone());
                ctx.spawn_task(task);
                let id = self.record_saved.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordSaved", id);
                id
            }
            RegisterEmitter::RecordBroken(emitter) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                let id = self.record_broken.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordBroken", id);
                id
            }
            RegisterEmitter::RecordRemoved(emitter) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                let id = self.record_removed.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::RecordRemoved", id);
                id
            }
            RegisterEmitter::ContentRemoved(emitter) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                let id = self.content_removed.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::ContentRemoved", id);
                id
            }
            RegisterEmitter::ContentSha256Calculated(emitter) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                let id = self.content_sha256_calculated.register(emitter);
                tracing::debug!(msg.name = "RegisterEmitter::ContentSha256Calculated", id);
                id
            }
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_record_saved_for_each_record(config: Arc<Config>, emitter: Emitter<RecordSaved>) {
        let records_dir = config.recording.records_dir.as_ref().unwrap();
        for record_path in glob_records(records_dir) {
            match load_record(&config, &record_path).await {
                Ok((record, _)) => {
                    let msg = RecordSaved {
                        record_id: record.id,
                        recording_status: record.recording_status,
                    };
                    emitter.emit(msg).await;
                }
                Err(err) => tracing::error!(?record_path, ?err, "Failed to load, skip"),
            }
        }
    }
}

#[derive(Message)]
pub struct RegisterEmitterPostProcess(Emitter<RecordingStarted>);

#[async_trait]
impl<T, E, O> Handler<RegisterEmitterPostProcess> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RegisterEmitterPostProcess, _ctx: &mut Context<Self>) {
        let emitter = msg.0;
        for &program_id in self.recorders.keys() {
            let msg = RecordingStarted { program_id };
            emitter.emit(msg).await;
        }
    }
}

// unregister emitter

#[derive(Message)]
pub enum UnregisterEmitter {
    RecordingStarted(usize),
    RecordingStopped(usize),
    RecordingFailed(usize),
    RecordingRescheduled(usize),
    RecordSaved(usize),
    RecordRemoved(usize),
    ContentRemoved(usize),
    RecordBroken(usize),
    ContentSha256Calculated(usize),
}

#[async_trait]
impl<T, E, O> Handler<UnregisterEmitter> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: UnregisterEmitter, _ctx: &mut Context<Self>) {
        debug_assert!(self.config.recording.is_enabled());
        match msg {
            UnregisterEmitter::RecordingStarted(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::RecordingStarted", id);
                self.recording_started.unregister(id);
            }
            UnregisterEmitter::RecordingStopped(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::RecordingStopped", id);
                self.recording_stopped.unregister(id);
            }
            UnregisterEmitter::RecordingFailed(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::RecordingFailed", id);
                self.recording_failed.unregister(id);
            }
            UnregisterEmitter::RecordingRescheduled(id) => {
                tracing::debug!(msg.name = "UnregisterEmitter::RecordingRescheduled", id);
                self.recording_rescheduled.unregister(id);
            }
            UnregisterEmitter::RecordSaved(id) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                tracing::debug!(msg.name = "UnregisterEmitter::RecordSaved", id);
                self.record_saved.unregister(id);
            }
            UnregisterEmitter::RecordBroken(id) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                tracing::debug!(msg.name = "UnregisterEmitter::RecordBroken", id);
                self.record_broken.unregister(id);
            }
            UnregisterEmitter::RecordRemoved(id) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                tracing::debug!(msg.name = "UnregisterEmitter::RecordRemoved", id);
                self.record_removed.unregister(id);
            }
            UnregisterEmitter::ContentRemoved(id) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                tracing::debug!(msg.name = "UnregisterEmitter::ContentRemoved", id);
                self.content_removed.unregister(id);
            }
            UnregisterEmitter::ContentSha256Calculated(id) => {
                debug_assert!(self.config.recording.is_records_api_enabled());
                tracing::debug!(msg.name = "UnregisterEmitter::ContentSha256Calculated", id);
                self.content_sha256_calculated.unregister(id);
            }
        }
    }
}

// recording started

#[derive(Clone, Message)]
pub struct RecordingStarted {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStarted> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        let msg = RecordingStarted { program_id };
        self.recording_started.emit(msg).await;
    }
}

// recording stopped

#[derive(Clone, Message)]
pub struct RecordingStopped {
    pub program_id: ProgramId,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStopped> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        // The schedule is NOT removed for a while so that an external script
        // listening recording events can access the details of the schedule.
        //
        // It will be removed in the `ProcessRecording` message handler.

        // NOTE: `self.schedules.get_mut(&program_id)` may return `None`.
        // The schedule may have been removed before the recording stops.
        // For example, `clear_schedules()` clears schedules before the recordings stop.

        let mut changed = false;
        let mut done = false;

        let recorder = match self.recorders.get_mut(&program_id) {
            Some(recorder) => recorder,
            None => {
                tracing::error!(
                    schedule.program.id = %program_id,
                    "INCONSNSTENT: The recorder stopped before the recording stopped",
                );
                let msg = RecordingStopped { program_id };
                self.recording_stopped.emit(msg).await;
                return changed;
            }
        };

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
            if let Some(schedule) = self.schedules.get_mut(&program_id) {
                tracing::warn!(
                    %schedule.program.id,
                    "Need rescheduling",
                );
                schedule.state = RecordingScheduleState::Rescheduling;
                changed = true;
                self.emit_recording_failed(program_id, RecordingFailedReason::NeedRescheduling)
                    .await;
            }
        } else if let Some(exit_code) = get_first_error(&results) {
            tracing::error!(
                %exit_code,
                schedule.program.id = %program_id,
                "The recording pipeline terminated abnormally",
            );
            let reason = RecordingFailedReason::PipelineError { exit_code };
            if let Some(schedule) = self.schedules.get_mut(&program_id) {
                schedule.state = RecordingScheduleState::Failed;
                schedule.failed_reason = Some(reason.clone());
                changed = true;
                done = true;
            }
            self.emit_recording_failed(program_id, reason).await;
        } else {
            tracing::info!(
                schedule.program.id = %program_id,
                "The recording finished successfully",
            );
            if let Some(schedule) = self.schedules.get_mut(&program_id) {
                schedule.state = RecordingScheduleState::Finished;
                changed = true;
                done = true;
            }
        }

        // Don't move the schedule from `schedules` to `history` at this point.
        // The schedule may be used in `update_record()`.

        self.update_record(program_id).await;

        // Unlike the schedule, the recorder should be removed after the
        // recording stopped.
        self.recorders.remove(&program_id);

        // TODO: Save recording logs to a file.
        let msg = RecordingStopped { program_id };
        self.recording_stopped.emit(msg).await;

        if done {
            if let Some(schedule) = self.schedules.remove(&program_id) {
                self.history.push(schedule);
            }
        }

        changed
    }
}

// recording failed

#[derive(Clone, Message)]
pub struct RecordingFailed {
    pub program_id: ProgramId,
    pub reason: RecordingFailedReason,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
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
    T: TriggerFactory<StopStreaming>,
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
        let msg = RecordingFailed {
            program_id,
            reason: reason.clone(),
        };
        self.recording_failed.emit(msg).await;
    }
}

// recording rescheduled

#[derive(Clone, Message)]
pub struct RecordingRescheduled {
    pub program_id: ProgramId,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_rescheduled(&self, program_id: ProgramId) {
        let msg = RecordingRescheduled { program_id };
        self.recording_rescheduled.emit(msg).await;
    }
}

// record saved

#[derive(Clone, Message)]
pub struct RecordSaved {
    pub record_id: RecordId,
    pub recording_status: RecordingStatus,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_record_saved(&self, record_id: RecordId, recording_status: RecordingStatus) {
        let msg = RecordSaved {
            record_id,
            recording_status,
        };
        self.record_saved.emit(msg).await;
    }
}

// record broken

#[derive(Clone, Message)]
pub struct RecordBroken {
    pub record_id: RecordId,
    pub reason: &'static str,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_record_broken(&self, record_id: RecordId, reason: &'static str) {
        let msg = RecordBroken { record_id, reason };
        self.record_broken.emit(msg).await;
    }
}

// record removed

#[derive(Clone, Message)]
pub struct RecordRemoved {
    pub record_id: RecordId,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_record_removed(&self, record_id: RecordId) {
        let msg = RecordRemoved { record_id };
        self.record_removed.emit(msg).await;
    }
}

// content removed

#[derive(Clone, Message)]
pub struct ContentRemoved {
    pub record_id: RecordId,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_content_removed(&self, record_id: RecordId) {
        let msg = ContentRemoved { record_id };
        self.content_removed.emit(msg).await;
    }
}

// content sha256 calculated

#[derive(Clone, Message)]
pub struct ContentSha256Calculated {
    pub record_id: RecordId,
    pub content_sha256: String,
}

#[async_trait]
impl<T, E, O> Handler<ContentSha256Calculated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: ContentSha256Calculated, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ContentSha256Calculated", %msg.record_id, %msg.content_sha256);
        debug_assert!(self.config.recording.is_records_api_enabled());
        let record_path = make_record_path(&self.config, &msg.record_id).unwrap();
        match load_record(&self.config, &record_path).await {
            Ok((mut record, _)) => {
                record.content_sha256 = Some(msg.content_sha256.clone());
                if file_util::save_json(&record, &record_path) {
                    tracing::info!(?record_path, "Updated successfully");
                    self.content_sha256_calculated.emit(msg).await;
                    self.emit_record_saved(record.id.clone(), record.recording_status.clone())
                        .await;
                }
            }
            Err(err) => {
                tracing::error!(?err, ?record_path, "Broken record, skip updating");
                self.emit_record_broken(msg.record_id, "Broken record")
                    .await;
            }
        }
    }
}

// services updated

#[async_trait]
impl<T, E, O> Handler<epg::ServicesUpdated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
        let mut changed = false;
        let mut removed = vec![];
        self.schedules.retain(|&program_id, schedule| {
            if !schedule.can_be_updated_by_epg() {
                return true;
            }
            let service_id = ServiceId::from(program_id);
            match services.get(&service_id) {
                Some(service) => {
                    if schedule.service != *service {
                        schedule.service = service.clone();
                        changed = true;
                    }
                    true
                }
                None => {
                    tracing::warn!(%schedule.program.id, "Removed from EPG");
                    removed.push(program_id);
                    changed = true;
                    false
                }
            }
        });

        for &program_id in removed.iter() {
            if self.recorders.contains_key(&program_id) {
                let _ = self.stop_recorder(program_id);
            }
            self.emit_recording_failed(program_id, RecordingFailedReason::RemovedFromEpg)
                .await;
        }

        changed
    }
}

// programs updated

#[async_trait]
impl<T, E, O> Handler<epg::ProgramsUpdated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
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
                    schedule.program = program.clone();
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
    T: TriggerFactory<StopStreaming>,
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

        if let Some(schedule) = self.schedules.get_mut(&program.id) {
            match schedule.state {
                Scheduled | Rescheduling => {
                    rescheduled = schedule.program.start_at != program.start_at;
                    schedule.program = (*program).clone();
                    schedule.state = Tracking;
                    tracing::info!(
                        %schedule.program.id,
                        "Start tracking changes of the schedule",
                    );
                }
                Tracking => {
                    rescheduled = schedule.program.start_at != program.start_at;
                    schedule.program = (*program).clone();
                }
                Recording => {
                    schedule.program = (*program).clone();
                    self.update_record(program.id).await;
                }
                Finished | Failed => {
                    // Nothing to do.
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
    pub program: EpgProgram,
    pub service: EpgService,
    pub options: RecordingOptions,
    #[serde(default)]
    pub tags: HashSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_reason: Option<RecordingFailedReason>,
}

impl RecordingSchedule {
    pub fn new(
        program: EpgProgram,
        service: EpgService,
        options: RecordingOptions,
        tags: HashSet<String>,
    ) -> Self {
        RecordingSchedule {
            state: RecordingScheduleState::Scheduled,
            program,
            service,
            options,
            tags,
            failed_reason: None,
        }
    }

    fn can_be_updated_by_epg(&self) -> bool {
        use RecordingScheduleState::*;
        matches!(self.state, Scheduled | Rescheduling)
    }

    fn is_ready_for_recording(&self) -> bool {
        use RecordingScheduleState::*;
        matches!(self.state, Scheduled | Tracking)
    }

    fn is_recording(&self) -> bool {
        use RecordingScheduleState::*;
        matches!(self.state, Recording)
    }

    fn is_done(&self) -> bool {
        use RecordingScheduleState::*;
        matches!(self.state, Finished | Failed)
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
    // The recording finished successfully.
    Finished,
    // The recording failed for some reason.
    Failed,
}

/// Recording options.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(ToSchema)]
#[schema(title = "RecordingOptions")]
pub struct RecordingOptions {
    /// The path of the content file relative to `config.recording.basedir`.
    ///
    /// The path must be a valid Unicode string.
    ///
    /// ### If `config.recording.records-dir` is NOT specified
    ///
    /// This is a required option.  A response with the status code 401 will be replied if this
    /// option is not specified.
    ///
    /// ### If `config.recording.records-dir` is specified
    ///
    /// An auto-generated filename will be used for the content file if this option is not
    /// specified.
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_path: Option<PathBuf>,

    /// A priority of tuner usage.
    #[serde(default)]
    pub priority: i32,

    /// A list of pre-filters to use.
    #[serde(default)]
    pub pre_filters: Vec<String>,

    /// A list of post-filters to use.
    #[serde(default)]
    pub post_filters: Vec<String>,

    /// Log filter of the recording schedule.
    ///
    /// If this option is not specified, the value of `config.recording.log-filter` is used when
    /// the recording starts.  The log filter will be set to the environment variable
    /// `MIRAKC_ARIB_LOG` passed to the recording command pipeline.
    ///
    /// If the log filter is neither empty nor `off`, logs coming from the recording command
    /// pipeline will be stored into a log file.  The log file will be placed in the same folder as
    /// the content file and its name is "<content-file>.log".  The environment variable
    /// `MIRAKC_ARIB_LOG_NO_TIMESTAMP` will be disabled and each log will have a timestamp.
    ///
    /// If the log filter is empty or `off`, no log will come from the recording command pipeline.
    /// And the log file won't be created.
    ///
    /// For backward compatibility with 3.x and older versions, the logs will be output to STDOUT
    /// if neither this option nor `recording.log-filter` is specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_filter: Option<String>,
}

struct Recorder {
    started_at: DateTime<Jst>,
    pipeline: CommandPipeline<TunerSubscriptionId>,
    stop_trigger: Option<Trigger<StopStreaming>>,
    content_type: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RecordId(String);

impl RecordId {
    fn new(timestamp: i64, program_id: u64) -> Self {
        Self(format!("{timestamp:016X}{program_id:016X}"))
    }

    pub fn value(&self) -> &str {
        &self.0
    }

    fn from_record_path(record_path: &Path) -> Self {
        debug_assert!(record_path.to_string_lossy().ends_with(".record.json"));
        let id_str = record_path.file_prefix().unwrap();
        Self::from(id_str.to_str().unwrap().to_owned())
    }
}

impl From<(DateTime<Jst>, ProgramId)> for RecordId {
    fn from(value: (DateTime<Jst>, ProgramId)) -> Self {
        Self::new(value.0.timestamp_millis(), value.1.value())
    }
}

impl From<String> for RecordId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "record#{}", self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    pub id: RecordId,
    pub program: EpgProgram,
    pub service: EpgService,
    pub options: RecordingOptions,
    #[serde(default)]
    pub tags: HashSet<String>,
    pub recording_status: RecordingStatus,
    #[serde(with = "ts_milliseconds")]
    pub recording_start_time: DateTime<Jst>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "ts_milliseconds_option")]
    pub recording_end_time: Option<DateTime<Jst>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "duration_milliseconds_option")]
    pub recording_duration: Option<Duration>,
    pub content_path: PathBuf,
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
}

impl Record {
    fn new(
        id: RecordId,
        start_time: DateTime<Jst>,
        schedule: &RecordingSchedule,
        content_path: PathBuf,
        content_type: String,
    ) -> Self {
        debug_assert!(content_path.is_relative());
        debug_assert!(!content_type.is_empty());
        Self {
            id,
            program: schedule.program.clone(),
            service: schedule.service.clone(),
            options: schedule.options.clone(),
            tags: schedule.tags.clone(),
            recording_status: RecordingStatus::Recording,
            recording_start_time: start_time,
            recording_end_time: None,
            recording_duration: None,
            content_path,
            content_type,
            content_sha256: None,
        }
    }

    fn is_done(&self) -> bool {
        matches!(
            self.recording_status,
            RecordingStatus::Finished | RecordingStatus::Failed { .. }
        )
    }

    async fn update_by_schedule(&mut self, schedule: &RecordingSchedule) {
        let now = Jst::now();

        self.program = schedule.program.clone();
        self.options = schedule.options.clone();
        self.tags = schedule.tags.clone();

        self.recording_end_time = Some(now);
        self.recording_duration = Some(now - self.recording_start_time);

        match schedule.state {
            RecordingScheduleState::Scheduled | RecordingScheduleState::Tracking => (),
            RecordingScheduleState::Recording => {
                self.recording_status = RecordingStatus::Recording;
                self.recording_end_time = None;
                self.recording_duration = None;
            }
            RecordingScheduleState::Rescheduling => {
                self.recording_status = RecordingStatus::Failed {
                    reason: RecordingFailedReason::NeedRescheduling,
                };
            }
            RecordingScheduleState::Finished => {
                self.recording_status = RecordingStatus::Finished;
            }
            RecordingScheduleState::Failed => {
                self.recording_status = RecordingStatus::Failed {
                    reason: schedule.failed_reason.clone().unwrap(),
                };
            }
        }
    }
}

/// A recording status.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum RecordingStatus {
    Recording,
    Finished,
    Canceled,
    Failed { reason: RecordingFailedReason },
}

// helpers

fn check_retry(results: &[std::io::Result<ExitStatus>]) -> bool {
    results.iter().any(|result| match result {
        Ok(status) => matches!(status.code(), Some(EXIT_RETRY)),
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

pub fn make_schedules_path(config: &Config) -> Option<PathBuf> {
    config
        .recording
        .basedir
        .as_ref()
        .map(|basedir| basedir.join("schedules.v1.json"))
}

fn make_record_path(config: &Config, record_id: &RecordId) -> Option<PathBuf> {
    config
        .recording
        .records_dir
        .as_ref()
        .map(|records_dir| records_dir.join(format!("{}.record.json", record_id.value())))
}

fn make_content_filename(record_id: &RecordId) -> String {
    format!("{}.content", record_id.value())
}

fn make_content_path(config: &Config, record: &Record) -> Option<PathBuf> {
    config
        .recording
        .basedir
        .as_ref()
        .map(|basedir| basedir.join(&record.content_path))
}

fn make_relative_content_path(config: &Config, content_path: &Path) -> PathBuf {
    let basedir = config.recording.basedir.as_ref().unwrap();
    content_path.strip_prefix(basedir).unwrap().to_owned()
}

fn make_content_path_from_schedule(
    config: &Config,
    record_id: &RecordId,
    schedule: &RecordingSchedule,
) -> PathBuf {
    let basedir = config.recording.basedir.as_ref().unwrap();

    match schedule.options.content_path.as_ref() {
        Some(content_path) => {
            debug_assert!(content_path.is_relative());
            basedir.join(content_path).parse_dot().unwrap().into_owned()
        }
        None => {
            assert!(config.recording.is_records_api_enabled());
            basedir.join(make_content_filename(record_id))
        }
    }
}

// content_path + ".log"
fn make_log_path_from_content_path(content_path: &Path) -> PathBuf {
    // TODO(refactor): use PathBuf::add_extension() when it's stabilized.
    let mut path = content_path.to_owned();
    path.as_mut_os_string().push(".log");
    path
}

async fn load_record(config: &Config, record_path: &Path) -> Result<(Record, Option<u64>), Error> {
    let data = tokio::fs::read(record_path).await?;
    let record: Record = serde_json::from_slice(&data)?;
    let content_path = make_content_path(config, &record).unwrap();
    let size = content_path.is_file().then(|| {
        content_path
            .metadata()
            .ok()
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    });
    Ok((record, size))
}

fn glob_records(records_dir: &Path) -> impl Iterator<Item = PathBuf> {
    let record_pattern = format!("{}/*.record.json", records_dir.to_str().unwrap());
    glob::glob(&record_pattern)
        .unwrap()
        .filter_map(|record_path| match record_path {
            Ok(record_path) if record_path.is_file() => {
                Some(record_path)
            }
            Ok(record_path) => {
                tracing::warn!(?record_path, "Not a regular file, skip");
                None
            }
            Err(err) => {
                tracing::warn!(record_path = ?err.path(), err = ?err.error(), "Not accessible, skip");
                None
            }
        })
}
