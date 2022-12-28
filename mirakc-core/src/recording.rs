use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use chrono::DateTime;
use chrono::Duration;
use chrono_jst::Jst;
use chrono_jst::serde::ts_milliseconds;
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::BufWriter;
use tokio_util::sync::CancellationToken;

use crate::command_util::spawn_pipeline;
use crate::command_util::CommandPipeline;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::epg;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::QueryClock;
use crate::epg::QueryProgram;
use crate::epg::QueryService;
use crate::epg::ServicesUpdated;
use crate::error::Error;
use crate::filter::FilterPipelineBuilder;
use crate::models::ProgramQuad;
use crate::models::ServiceTriple;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::onair_tracker::OnairProgramChanged;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerStreamStopTrigger;
use crate::tuner::TunerSubscriptionId;

const EXIT_RETRY: i32 = 222;
const PREP_SECS: i64 = 15;

pub struct RecordingManager<T, E, O> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    onair_tracker: O,
    schedules: BinaryHeap<Arc<Schedule>>,
    schedule_map: HashMap<ProgramQuad, Arc<Schedule>>,
    recorders: HashMap<ProgramQuad, Recorder>,
    retries: HashMap<ProgramQuad, Arc<Schedule>>,
    timer_token: Option<CancellationToken>,
    recording_started_emitters: Vec<Emitter<RecordingStarted>>,
    recording_stopped_emitters: Vec<Emitter<RecordingStopped>>,
    recording_failed_emitters: Vec<Emitter<RecordingFailed>>,
    recording_retried_emitters: Vec<Emitter<RecordingRetried>>,
    recording_rescheduled_emitters: Vec<Emitter<RecordingRescheduled>>,
}

impl<T, E, O> RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E, onair_tracker: O) -> Self {
        RecordingManager {
            config,
            tuner_manager,
            epg,
            onair_tracker,
            schedules: Default::default(),
            schedule_map: Default::default(),
            recorders: Default::default(),
            retries: Default::default(),
            timer_token: None,
            recording_started_emitters: Default::default(),
            recording_stopped_emitters: Default::default(),
            recording_failed_emitters: Default::default(),
            recording_retried_emitters: Default::default(),
            recording_rescheduled_emitters: Default::default(),
        }
    }

    fn update_services(&mut self, services: Arc<IndexMap<ServiceTriple, EpgService>>) {
        let mut schedules = BinaryHeap::new();
        for schedule in self.schedules.drain() {
            let triple = ServiceTriple::from(schedule.program_quad);
            if services.contains_key(&triple) {
                schedules.push(schedule);
            } else {
                tracing::warn!(%schedule.program_quad, "Invalidated");
                self.schedule_map.remove(&schedule.program_quad);
            }
        }
        self.schedules = schedules;
        assert_eq!(self.schedules.len(), self.schedule_map.len());
    }

    async fn check_schedules(&mut self, ctx: &Context<Self>) {
        let prep_secs = Duration::seconds(PREP_SECS);
        let now = Jst::now();
        while let Some(schedule) = self.schedules.peek() {
            if schedule.end_at - now <= prep_secs {
                tracing::warn!(%schedule.program_quad, "Program will end soon, remove schedule");
                self.schedule_map.remove(&schedule.program_quad);
                self.schedules.pop();
            } else if schedule.start_at - now <= prep_secs {
                self.schedule_map.remove(&schedule.program_quad);
                let schedule = self.schedules.pop().unwrap();
                match self.start_recording(schedule.clone(), ctx).await {
                    Ok(_) => {
                        tracing::info!(%schedule.program_quad, "Start recording");
                    }
                    Err(err) => {
                        tracing::error!(%err, %schedule.program_quad, "Failed to start recording");
                    }
                }
            } else {
                break;
            }
        }

        assert_eq!(self.schedules.len(), self.schedule_map.len());
    }

    async fn start_recording(
        &mut self,
        schedule: Arc<Schedule>,
        ctx: &Context<Self>,
    ) -> Result<RecorderModel, Error> {
        let program_quad = schedule.program_quad;
        let service_triple = schedule.program_quad.into();

        if self.recorders.contains_key(&program_quad) {
            return Err(Error::AlreadyExists);
        }

        // TODO: copied from web::program_stream_g().

        let program = self
            .epg
            .call(QueryProgram::ByProgramQuad(program_quad))
            .await??;

        let service = self
            .epg
            .call(QueryService::ByServiceTriple(service_triple))
            .await??;

        let clock = self
            .epg
            .call(QueryClock {
                triple: service_triple,
            })
            .await??;

        let stream = self
            .tuner_manager
            .call(StartStreaming {
                channel: service.channel.clone(),
                user: TunerUser {
                    info: TunerUserInfo::Recorder {
                        name: format!("program#{}", program_quad),
                    },
                    priority: schedule.priority.into(),
                },
            })
            .await??;

        // stop_trigger must be created here in order to stop streaming when
        // an error occurs.
        let stop_trigger =
            TunerStreamStopTrigger::new(stream.id(), self.tuner_manager.clone().into());

        let video_tags: Vec<u8> = program
            .video
            .iter()
            .map(|video| video.component_tag)
            .collect();

        let audio_tags: Vec<u8> = program
            .audios
            .values()
            .map(|audio| audio.component_tag)
            .collect();

        let mut builder = mustache::MapBuilder::new();
        builder = builder
            .insert_str("channel_name", &service.channel.name)
            .insert("channel_type", &service.channel.channel_type)?
            .insert_str("channel", &service.channel.channel)
            .insert("sid", &program.quad.sid().value())?
            .insert("eid", &program.quad.eid().value())?
            .insert("clock_pid", &clock.pid)?
            .insert("clock_pcr", &clock.pcr)?
            .insert("clock_time", &clock.time)?
            .insert("video_tags", &video_tags)?
            .insert("audio_tags", &audio_tags)?;
        let data = builder.build();

        let mut builder = FilterPipelineBuilder::new(data);
        builder.add_pre_filters(&self.config.pre_filters, &schedule.pre_filters)?;
        if !stream.is_decoded() {
            builder.add_decode_filter(&self.config.filters.decode_filter)?;
        }
        builder.add_program_filter(&self.config.filters.program_filter)?;
        builder.add_post_filters(&self.config.post_filters, &schedule.post_filters)?;
        let (filters, _) = builder.build();

        let contents_dir = self.config.recording.contents_dir.as_ref().unwrap();

        let content_path = if schedule.content_path.is_absolute() {
            schedule.content_path.clone()
        } else {
            contents_dir.join(&schedule.content_path)
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
            let addr = ctx.address().clone();
            async move {
                addr.emit(RecordingStarted { program_quad }).await;
                let result = inner_fut.await;
                addr.emit(RecordingStopped { program_quad }).await;
                if let Err(err) = result {
                    addr.emit(RecordingFailed {
                        program_quad,
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
            schedule,
            started_at: Jst::now(),
            pipeline,
            stop_trigger: Some(stop_trigger),
        };
        let model = recorder.get_model();
        self.recorders.insert(program_quad, recorder);

        // Spawn the following task after the recorder is inserted so that
        // actors receiving RecordingStarted messages can access the recorder.
        ctx.spawn_task(fut);

        Ok(model)
    }

    fn stop_recording(&mut self, program_quad: ProgramQuad) -> Result<(), Error> {
        match self.recorders.get_mut(&program_quad) {
            Some(recorder) => {
                match recorder.stop_trigger.take() {
                    Some(_) => tracing::info!(%program_quad, "Stop recording"),
                    None => tracing::warn!(%program_quad, "Already stopped"),
                }
                Ok(())
            }
            None => {
                tracing::error!(%program_quad, "No such recorder, maybe already stopped");
                Err(Error::RecorderNotFound)
            }
        }
    }

    fn set_timer(&mut self, ctx: &Context<Self>) {
        if let Some(token) = self.timer_token.take() {
            token.cancel();
        }
        if let Some(schedule) = self.schedules.peek() {
            let expires_at = schedule.start_at - Duration::seconds(PREP_SECS);
            let duration = match (expires_at - Jst::now()).to_std() {
                Ok(duration) => {
                    tracing::debug!(%expires_at, "Set timer");
                    duration
                }
                Err(_) => {
                    tracing::warn!(%expires_at, "Preparation time too small, the beginning of some programs may not be recorded");
                    std::time::Duration::ZERO
                }
            };
            let addr = ctx.address().clone();
            let token = ctx.spawn_task(async move {
                tokio::time::sleep(duration).await;
                addr.emit(TimerExpired).await;
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
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");
        self.load_schedules();
        self.epg
            .call(epg::RegisterEmitter::ServicesUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for ServicesUpdated");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn load_schedules(&mut self) {
        fn do_load(path: &Path) -> Result<Vec<Schedule>, Error> {
            let file = std::fs::File::open(path)?;
            Ok(serde_json::from_reader(file)?)
        }

        let records_dir = match self.config.recording.records_dir {
            Some(ref records_dir) => records_dir,
            None => return,
        };

        let path = records_dir.join("schedules.json");
        if !path.exists() {
            return;
        }

        match do_load(&path) {
            Ok(schedules) => {
                tracing::info!(?path, "Loaded");
                for schedule in schedules.into_iter() {
                    let program_quad = schedule.program_quad;
                    if let Err(err) = self.add_schedule(schedule) {
                        tracing::error!(%err, %program_quad, "Recording failed");
                        // TODO
                        // ----
                        // Should emit RecordingFailed messages when the
                        // schedule has been expired.  However, no observer is
                        // registered at this point.  Because this function is
                        // called from Actor::started()...
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, ?path, "Failed to load");
            }
        }
    }

    fn save_schedules(&self) {
        fn do_save(schedules: Vec<&Arc<Schedule>>, path: &Path) -> Result<(), Error> {
            let file = std::fs::File::create(path)?;
            serde_json::to_writer(file, &schedules)?;
            Ok(())
        }

        let records_dir = match self.config.recording.records_dir {
            Some(ref records_dir) => records_dir,
            None => return,
        };

        let path = records_dir.join("schedules.json");
        let schedules = self.schedule_map.values().collect_vec();

        match do_save(schedules, &path) {
            Ok(_) => tracing::info!(?path, "Saved"),
            Err(err) => tracing::error!(%err, ?path, "Failed to save"),
        }
    }
}

// query recording schedules

#[derive(Message)]
#[reply("Vec<Arc<Schedule>>")]
pub struct QueryRecordingSchedules;

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedules> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
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
    fn query_schedules(&self) -> Vec<Arc<Schedule>> {
        // TODO: somewhat inefficient...
        self.schedules
            .clone()
            .into_sorted_vec()
            .into_iter()
            .rev()
            .collect_vec()
    }
}

// query recording schedule

#[derive(Message)]
#[reply("Result<Arc<Schedule>, Error>")]
pub struct QueryRecordingSchedule {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingSchedule,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingSchedule", %msg.program_quad);
        self.get_schedule(msg.program_quad)
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn get_schedule(&self, program_quad: ProgramQuad) -> Result<Arc<Schedule>, Error> {
        self.schedule_map
            .get(&program_quad)
            .cloned()
            .ok_or(Error::ScheduleNotFound)
    }
}

// add recording schedule

#[derive(Message)]
#[reply("Result<Arc<Schedule>, Error>")]
pub struct AddRecordingSchedule {
    pub schedule: Schedule,
}

#[async_trait]
impl<T, E, O> Handler<AddRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: AddRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <AddRecordingSchedule as Message>::Reply {
        tracing::debug!(
            msg.name = "AddRecordingSchedule",
            %msg.schedule.program_quad,
            ?msg.schedule.content_path,
            %msg.schedule.priority,
            ?msg.schedule.pre_filters,
            ?msg.schedule.post_filters,
            %msg.schedule.start_at,
        );
        let program_quad = msg.schedule.program_quad;
        self.add_schedule(msg.schedule)?;
        self.save_schedules();
        self.set_timer(ctx);
        self.get_schedule(program_quad)
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn add_schedule(&mut self, schedule: Schedule) -> Result<(), Error> {
        let deadline = Jst::now() + Duration::seconds(PREP_SECS);
        let program_quad = schedule.program_quad;
        if self.schedule_map.contains_key(&program_quad) {
            let err = Error::AlreadyExists;
            tracing::warn!(%schedule.program_quad, %err);
            Err(err)
        } else if schedule.end_at <= deadline {
            let err = Error::ProgramEnded;
            tracing::error!(%schedule.program_quad, %err);
            Err(err)
        } else {
            tracing::info!(%schedule.program_quad, "Added");
            let schedule = Arc::new(schedule);
            self.schedules.push(schedule.clone());
            self.schedule_map.insert(program_quad, schedule);
            assert_eq!(self.schedules.len(), self.schedule_map.len());
            Ok(())
        }
    }
}

// remove recording schedule

#[derive(Message)]
#[reply("Result<Arc<Schedule>, Error>")]
pub struct RemoveRecordingSchedule {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<RemoveRecordingSchedule> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedule", %msg.program_quad);
        match self.schedule_map.remove(&msg.program_quad) {
            Some(schedule) => {
                self.sync_schedules();
                self.save_schedules();
                self.set_timer(ctx);
                Ok(schedule)
            }
            None => {
                tracing::warn!(%msg.program_quad, "No such schedule");
                Err(Error::ScheduleNotFound)
            }
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn sync_schedules(&mut self) {
        self.schedules.clear();
        for schedule in self.schedule_map.values() {
            self.schedules.push(schedule.clone());
        }
        assert_eq!(self.schedules.len(), self.schedule_map.len());
    }
}

// remove recording schedule

#[derive(Message)]
#[reply("()")]
pub struct RemoveRecordingSchedules {
    pub target: RemoveTarget,
}

#[derive(Debug)]
pub enum RemoveTarget {
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
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedules,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedules", ?msg.target);
        self.remove_schedules(msg.target);
        self.save_schedules();
        self.set_timer(ctx);
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn remove_schedules(&mut self, target: RemoveTarget) {
        match target {
            RemoveTarget::All => self.clear_schedules(),
            RemoveTarget::Tag(tag) => self.remove_schedules_by_tag(&tag),
        }
    }

    fn clear_schedules(&mut self) {
        tracing::info!("Remove all schedules");
        self.schedule_map.clear();
        self.schedules.clear();
    }

    fn remove_schedules_by_tag(&mut self, tag: &str) {
        let start_soon = Jst::now() + Duration::seconds(PREP_SECS);
        tracing::info!("Remove schedules tagged with {}", tag);
        self.schedule_map.retain(|_, v| {
            // Always retain schedules which will start soon
            // (or have already been expired).
            if v.start_at <= start_soon {
                return true;
            }
            return !v.tags.contains(tag);
        });
        self.sync_schedules();
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
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        _msg: QueryRecordingRecorders,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecorders as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecorders");
        self.recorders
            .values()
            .map(|recorder| recorder.get_model())
            .collect()
    }
}

// query recording recorder

#[derive(Message)]
#[reply("Result<RecorderModel, Error>")]
pub struct QueryRecordingRecorder {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<QueryRecordingRecorder> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingRecorder,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecorder as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecorder", %msg.program_quad);
        self.recorders
            .get(&msg.program_quad)
            .map(|recorder| recorder.get_model())
            .ok_or(Error::RecorderNotFound)
    }
}

// start recording

#[derive(Message)]
#[reply("Result<RecorderModel, Error>")]
pub struct StartRecording {
    pub schedule: Arc<Schedule>,
}

#[async_trait]
impl<T, E, O> Handler<StartRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: StartRecording,
        ctx: &mut Context<Self>,
    ) -> <StartRecording as Message>::Reply {
        tracing::debug!(
            msg.name = "StartRecording",
            %msg.schedule.program_quad,
            ?msg.schedule.content_path,
            %msg.schedule.priority,
            ?msg.schedule.pre_filters,
            ?msg.schedule.post_filters,
        );
        self.start_recording(msg.schedule, ctx).await
    }
}

// stop recording

#[derive(Message)]
#[reply("Result<(), Error>")]
pub struct StopRecording {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<StopRecording> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(
        &mut self,
        msg: StopRecording,
        _ctx: &mut Context<Self>,
    ) -> <StopRecording as Message>::Reply {
        tracing::debug!(msg.name = "StopRecording", %msg.program_quad);
        self.stop_recording(msg.program_quad)
    }
}

// services updated

#[async_trait]
impl<T, E, O> Handler<ServicesUpdated> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, msg: ServicesUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        self.update_services(msg.services);
        self.save_schedules();
        self.set_timer(ctx);
    }
}

// timer expired

#[derive(Message)]
struct TimerExpired;

#[async_trait]
impl<T, E, O> Handler<TimerExpired> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, _msg: TimerExpired, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "TimerExpired");
        self.check_schedules(ctx).await;
        self.save_schedules();
        self.set_timer(ctx);
    }
}

// register emitter

#[derive(Message)]
#[reply("()")]
pub enum RegisterEmitter {
    RecordingStarted(Emitter<RecordingStarted>),
    RecordingStopped(Emitter<RecordingStopped>),
    RecordingFailed(Emitter<RecordingFailed>),
    RecordingRetried(Emitter<RecordingRetried>),
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
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
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
            RegisterEmitter::RecordingRetried(emitter) => {
                self.recording_retried_emitters.push(emitter);
            }
            RegisterEmitter::RecordingRescheduled(emitter) => {
                self.recording_rescheduled_emitters.push(emitter);
            }
        }
    }
}

// recording started

#[derive(Clone, Message)]
pub struct RecordingStarted {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStarted> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, msg: RecordingStarted, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_quad);
        for emitter in self.recording_started_emitters.iter() {
            emitter.emit(msg.clone()).await;
        }
    }
}

// recording stopped

#[derive(Clone, Message)]
pub struct RecordingStopped {
    pub program_quad: ProgramQuad,
}

#[async_trait]
impl<T, E, O> Handler<RecordingStopped> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, msg: RecordingStopped, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_quad);

        for emitter in self.recording_stopped_emitters.iter() {
            emitter.emit(msg.clone()).await;
        }

        let recorder = self.recorders.remove(&msg.program_quad);
        let retry_enabled = self.config.recording.max_start_delay.is_some();
        match (recorder, retry_enabled) {
            (Some(mut recorder), true) => {
                // Manually drop the stop trigger so that we get the exit code
                // from the program-filter without killing its process.
                if let Some(stop_trigger) = recorder.stop_trigger.take() {
                    drop(stop_trigger);
                }
                if recorder.check_retry().await {
                    let program_quad = msg.program_quad;
                    tracing::warn!(%program_quad, "Recording stopped before the program starts");
                    let service_triple = program_quad.into();
                    let result = if self.need_adding_observer(service_triple) {
                        self.onair_tracker
                            .call(crate::onair_tracker::AddObserver {
                                service_triple,
                                name: "recording",
                                emitter: ctx.address().clone().into(),
                            })
                            .await
                    } else {
                        Ok(())
                    };
                    match result {
                        Ok(_) => {
                            tracing::info!(%program_quad, "Retry recording");
                            self.retry(recorder.schedule);
                            self.emit_recording_retried(program_quad).await;
                        }
                        Err(err) => {
                            tracing::error!(%err, %program_quad, "Failed to retry recording");
                            self.emit_recording_failed(
                                program_quad,
                                RecordingFailedReason::RetryFailed,
                            )
                            .await;
                        }
                    }
                }
            }
            // Retry is disabled.
            (Some(mut recorder), false) => {
                // Manually drop the stop trigger so that we get the exit code
                // from the program-filter without killing its process.
                if let Some(stop_trigger) = recorder.stop_trigger.take() {
                    drop(stop_trigger);
                }
                if let Some(exit_code) = recorder.get_first_error().await {
                    let program_quad = msg.program_quad;
                    tracing::error!(%program_quad, %exit_code, "Failed in the recording pipeline");
                    self.emit_recording_failed(
                        program_quad,
                        RecordingFailedReason::PipelineError { exit_code },
                    )
                    .await;
                }
            }
            (None, _) => {
                tracing::warn!(program_quad = %msg.program_quad, "Already removed");
            }
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    fn retry(&mut self, schedule: Arc<Schedule>) {
        let program_quad = schedule.program_quad;
        self.retries.insert(program_quad, schedule);
    }

    fn need_adding_observer(&self, service_triple: ServiceTriple) -> bool {
        !self
            .retries
            .keys()
            .any(|&quad| service_triple == quad.into())
    }
}

// recording failed

#[derive(Clone, Message)]
pub struct RecordingFailed {
    pub program_quad: ProgramQuad,
    pub reason: RecordingFailedReason,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum RecordingFailedReason {
    #[serde(rename_all = "camelCase")]
    IoError {
        message: String,
        os_error: Option<i32>,
    },
    #[serde(rename_all = "camelCase")]
    PipelineError {
        exit_code: i32,
    },
    RetryFailed,
}

#[async_trait]
impl<T, E, O> Handler<RecordingFailed> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, msg: RecordingFailed, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingFailed", %msg.program_quad, ?msg.reason);
        self.emit_recording_failed(msg.program_quad, msg.reason)
            .await;
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_failed(
        &self,
        program_quad: ProgramQuad,
        reason: RecordingFailedReason,
    ) {
        for emitter in self.recording_failed_emitters.iter() {
            let reason = reason.clone();
            emitter
                .emit(RecordingFailed {
                    program_quad,
                    reason,
                })
                .await;
        }
    }
}

// recording retried

#[derive(Clone, Message)]
pub struct RecordingRetried {
    pub program_quad: ProgramQuad,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_retried(&self, program_quad: ProgramQuad) {
        for emitter in self.recording_retried_emitters.iter() {
            emitter.emit(RecordingRetried { program_quad }).await;
        }
    }
}

// recording rescheduled

#[derive(Clone, Message)]
pub struct RecordingRescheduled {
    pub program_quad: ProgramQuad,
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn emit_recording_rescheduled(&self, program_quad: ProgramQuad) {
        for emitter in self.recording_rescheduled_emitters.iter() {
            emitter.emit(RecordingRescheduled { program_quad }).await;
        }
    }
}

// on-air program changed

#[async_trait]
impl<T, E, O> Handler<OnairProgramChanged> for RecordingManager<T, E, O>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair_tracker::AddObserver>,
    O: Call<crate::onair_tracker::RemoveObserver>,
{
    async fn handle(&mut self, msg: OnairProgramChanged, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "OnairProgramChanged", %msg.service_triple);

        if let Some(program) = msg.present {
            if let Some(schedule) = self.retries.remove(&program.quad) {
                let rescheduled = self.reschedule(&schedule, &program).await;
                if rescheduled {
                    self.set_timer(ctx);
                }
            }
        }

        if let Some(program) = msg.following {
            if let Some(schedule) = self.retries.remove(&program.quad) {
                let rescheduled = self.reschedule(&schedule, &program).await;
                if rescheduled {
                    self.set_timer(ctx);
                }
            }
        }

        let now = Jst::now();
        let duration = Duration::from_std(self.config.recording.max_start_delay.unwrap()).unwrap();
        self.retries.retain(|_, schedule| {
            let expired = schedule.start_at + duration > now;
            if expired {
                tracing::info!(%schedule.program_quad, "Retry expired");
            }
            expired
        });

        if self.need_removing_observer(msg.service_triple) {
            let _ = self
                .onair_tracker
                .call(crate::onair_tracker::RemoveObserver {
                    service_triple: msg.service_triple,
                    name: "recording",
                })
                .await;
        }
    }
}

impl<T, E, O> RecordingManager<T, E, O> {
    async fn reschedule(&mut self, schedule: &Schedule, program: &EpgProgram) -> bool {
        let result = self.add_schedule(Schedule {
            program_quad: program.quad,
            start_at: program.start_at,
            end_at: program.end_at(),
            content_path: schedule.content_path.clone(),
            priority: schedule.priority,
            pre_filters: schedule.pre_filters.clone(),
            post_filters: schedule.post_filters.clone(),
            tags: schedule.tags.clone(),
        });
        match result {
            Ok(_) => {
                tracing::info!(%program.quad, %program.start_at, "Rescheduled recording");
                self.emit_recording_rescheduled(program.quad).await;
                true
            }
            Err(err) => {
                tracing::error!(%err, %program.quad, "Retry failed");
                self.emit_recording_failed(program.quad, RecordingFailedReason::RetryFailed)
                    .await;
                false
            }
        }
    }

    fn need_removing_observer(&self, service_triple: ServiceTriple) -> bool {
        self.retries
            .keys()
            .filter(|&&quad| service_triple == quad.into())
            .count()
            == 0
    }
}

// models

#[derive(Clone, Debug, Deserialize, Eq, Serialize)]
pub struct Schedule {
    pub program_quad: ProgramQuad,
    #[serde(with = "ts_milliseconds")]
    pub start_at: DateTime<Jst>,
    #[serde(with = "ts_milliseconds")]
    pub end_at: DateTime<Jst>,
    pub content_path: PathBuf,
    pub priority: i32,
    pub pre_filters: Vec<String>,
    pub post_filters: Vec<String>,
    pub tags: HashSet<String>,
}

impl Ord for Schedule {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.start_at
            .cmp(&other.start_at)
            .reverse()
            .then_with(|| self.priority.cmp(&other.priority))
    }
}

impl PartialEq for Schedule {
    fn eq(&self, other: &Self) -> bool {
        self.program_quad == other.program_quad
    }
}

impl PartialOrd for Schedule {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct Recorder {
    schedule: Arc<Schedule>,
    started_at: DateTime<Jst>,
    pipeline: CommandPipeline<TunerSubscriptionId>,
    stop_trigger: Option<TunerStreamStopTrigger>,
}

impl Recorder {
    fn get_model(&self) -> RecorderModel {
        RecorderModel {
            schedule: self.schedule.clone(),
            started_at: self.started_at,
            pipeline: self.pipeline.get_model(),
        }
    }

    async fn check_retry(&mut self) -> bool {
        assert!(self.stop_trigger.is_none());
        self.pipeline
            .wait()
            .await
            .iter()
            .any(|result| match result {
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

    async fn get_first_error(&mut self) -> Option<i32> {
        assert!(self.stop_trigger.is_none());
        self.pipeline
            .wait()
            .await
            .iter()
            .flat_map(|result| match result {
                Ok(status) => status.code(),
                _ => None,
            })
            .find(|&code| code != 0)
    }
}

pub struct RecorderModel {
    pub schedule: Arc<Schedule>,
    pub started_at: DateTime<Jst>,
    pub pipeline: Vec<CommandPipelineProcessModel>,
}

#[derive(Deserialize, Serialize)]
pub struct Record {
    pub program: EpgProgram,
    pub content_path: PathBuf,
    pub content_type: String,
    pub pre_filters: Vec<String>,
    pub post_filters: Vec<String>,
    pub tags: HashSet<String>,
}

pub struct RecordId(DateTime<Jst>, ProgramQuad);

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::epg::stub::EpgStub;
    use crate::onair_tracker::stub::OnairTrackerStub;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use static_assertions::const_assert;
    use tempfile::TempDir;

    #[test]
    fn test_add_schedule() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairTrackerStub);

        let schedule =
            schedule_for_test((0, 0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let schedule =
            schedule_for_test((0, 0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::AlreadyExists));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 0, 2).into(), now);
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::ProgramEnded));
        assert_eq!(manager.schedules.len(), 1);

        let schedule =
            schedule_for_test((0, 0, 0, 4).into(), now + Duration::seconds(PREP_SECS + 2));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);
        assert_matches!(manager.schedules.peek(), Some(schedule) => {
            assert_eq!(schedule.program_quad, (0, 0, 0, 1).into());
        });
    }

    #[test]
    fn test_query_schedules() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairTrackerStub);

        let schedule =
            schedule_for_test((0, 0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let schedule =
            schedule_for_test((0, 0, 0, 2).into(), now + Duration::seconds(PREP_SECS + 2));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);

        let schedules = manager.query_schedules();
        assert_eq!(schedules.len(), 2);
        assert_matches!(schedules.get(0), Some(schedule) => {
            assert_eq!(schedule.program_quad, (0, 0, 0, 1).into());
        });
    }

    #[test]
    fn test_remove_schedules() {
        let now = Jst::now();

        let config = config_for_test("/tmp");

        let mut manager =
            RecordingManager::new(config, TunerManagerStub, EpgStub, OnairTrackerStub);

        let mut schedule =
            schedule_for_test((0, 0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        schedule.tags.insert("tag1".to_string());
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let mut schedule =
            schedule_for_test((0, 0, 0, 2).into(), now + Duration::seconds(PREP_SECS + 1));
        schedule.tags.insert("tag2".to_string());
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);

        manager.remove_schedules(RemoveTarget::Tag("tag2".to_string()));
        assert!(manager.schedule_map.contains_key(&(0, 0, 0, 1).into()));

        // Schedules which will start soon are always retained.
        let mut schedule =
            schedule_for_test((0, 0, 0, 3).into(), now + Duration::seconds(PREP_SECS - 1));
        schedule.tags.insert("tag1".to_string());
        let schedule = Arc::new(schedule);
        manager.schedules.push(schedule.clone());
        manager.schedule_map.insert((0, 0, 0, 3).into(), schedule);
        manager.remove_schedules(RemoveTarget::Tag("tag1".to_string()));
        assert!(!manager.schedule_map.contains_key(&(0, 0, 0, 1).into()));
        assert!(manager.schedule_map.contains_key(&(0, 0, 0, 3).into()));

        // Remove all schedules regardless of whether a schedule will start soon
        // or not.
        manager.remove_schedules(RemoveTarget::All);
        assert!(manager.schedules.is_empty());
    }

    #[tokio::test]
    async fn test_recorder_check_retry() {
        let now = Jst::now();
        let program_quad = (0, 0, 1, 1).into();
        let schedule = Arc::new(schedule_for_test(program_quad, now));

        // exit(0)
        let pipeline = spawn_pipeline(vec!["true".to_string()], Default::default()).unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let retry = recorder.check_retry().await;
        assert!(!retry);

        // exit(1)
        let pipeline = spawn_pipeline(vec!["false".to_string()], Default::default()).unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let retry = recorder.check_retry().await;
        assert!(!retry);

        // no such command
        let pipeline = spawn_pipeline(
            vec!["sh -c 'command_not_found'".to_string()],
            Default::default(),
        )
        .unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let retry = recorder.check_retry().await;
        assert!(!retry);

        // retry
        let pipeline = spawn_pipeline(
            vec![format!("sh -c 'exit {}'", EXIT_RETRY)],
            Default::default(),
        )
        .unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let retry = recorder.check_retry().await;
        assert!(retry);
    }

    #[tokio::test]
    async fn test_recorder_get_first_error() {
        let now = Jst::now();
        let program_quad = (0, 0, 1, 1).into();
        let schedule = Arc::new(schedule_for_test(program_quad, now));

        // exit(0)
        let pipeline = spawn_pipeline(vec!["true".to_string()], Default::default()).unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let code = recorder.get_first_error().await;
        assert_matches!(code, None);

        // exit(1)
        let pipeline = spawn_pipeline(vec!["false".to_string()], Default::default()).unwrap();
        let mut recorder = Recorder {
            schedule: schedule.clone(),
            started_at: now,
            pipeline,
            stop_trigger: None,
        };
        let code = recorder.get_first_error().await;
        assert_matches!(code, Some(1));
    }

    #[tokio::test]
    #[ignore]
    async fn test_recording() {
        let now = Jst::now();

        let temp_dir = TempDir::new().unwrap();

        let system = System::new();
        {
            let manager = system
                .spawn_actor(RecordingManager::new(
                    config_for_test(&temp_dir),
                    TunerManagerStub,
                    EpgStub,
                    OnairTrackerStub,
                ))
                .await;

            // Schedules for programs already started will be ignored.
            let schedule = Schedule {
                program_quad: (0, 0, 1, 1).into(),
                start_at: now,
                end_at: now,
                content_path: "1.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            // Schedules for programs start within PREP_SECS will be performed.
            assert_matches!(result, Ok(_));
            const_assert!(PREP_SECS >= 1);
            let schedule = Schedule {
                program_quad: (0, 0, 1, 2).into(),
                start_at: now + Duration::seconds(1),
                end_at: now + Duration::seconds(1),
                content_path: "2.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            // Schedules for programs start after PREP_SECS will be kept.
            let schedule = Schedule {
                program_quad: (0, 0, 1, 3).into(),
                start_at: now + Duration::seconds(PREP_SECS + 1),
                end_at: now + Duration::seconds(PREP_SECS + 1),
                content_path: "3.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            let start_time = std::time::Instant::now();
            loop {
                let schedules = manager.call(QueryRecordingSchedules).await.unwrap();
                let recorders = manager.call(QueryRecordingRecorders).await.unwrap();
                if !schedules.is_empty() && recorders.is_empty() {
                    assert_eq!(schedules.len(), 1);
                    break;
                }
                assert!(start_time.elapsed().as_secs() < 1);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            assert!(!temp_dir.path().join("1.m2ts").exists());
            assert!(temp_dir.path().join("2.m2ts").is_file());
            assert!(!temp_dir.path().join("3.m2ts").exists());

            let result = manager
                .inspect(|actor| {
                    assert_eq!(actor.schedules.len(), 1);
                    assert_eq!(actor.schedule_map.len(), 1);
                    assert!(actor.schedule_map.contains_key(&(0, 0, 1, 3).into()));
                })
                .await;
            assert_matches!(result, Ok(_));
        }
        system.stop();
    }

    fn config_for_test<P: AsRef<Path>>(dir: P) -> Arc<Config> {
        let mut config = Config::default();
        config.recording.records_dir = Some(dir.as_ref().to_owned());
        config.recording.contents_dir = config.recording.records_dir.clone();
        Arc::new(config)
    }

    fn schedule_for_test(program_quad: ProgramQuad, start_at: DateTime<Jst>) -> Schedule {
        Schedule {
            program_quad,
            start_at,
            end_at: start_at,
            content_path: format!("{}.m2ts", program_quad.eid().value()).into(),
            priority: Default::default(),
            pre_filters: Default::default(),
            post_filters: Default::default(),
            tags: Default::default(),
        }
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    pub(crate) struct RecordingManagerStub;

    #[async_trait]
    impl Call<AddRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: AddRecordingSchedule,
        ) -> Result<<AddRecordingSchedule as Message>::Reply, actlet::Error> {
            match msg.schedule.program_quad.eid().value() {
                // 0 is reserved for Error::ProgramNotFound
                1 => Ok(Err(Error::AlreadyExists)),
                2 => Ok(Err(Error::ProgramEnded)),
                _ => Ok(Ok(Arc::new(msg.schedule))),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: QueryRecordingSchedule,
        ) -> Result<<QueryRecordingSchedule as Message>::Reply, actlet::Error> {
            match msg.program_quad.eid().value() {
                0 => Ok(Err(Error::ProgramNotFound)),
                _ => Ok(Ok(Arc::new(Schedule {
                    program_quad: msg.program_quad,
                    start_at: Jst::now(),
                    end_at: Jst::now(),
                    content_path: "test.m2ts".into(),
                    priority: 1,
                    pre_filters: vec![],
                    post_filters: vec![],
                    tags: Default::default(),
                }))),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingSchedules> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: QueryRecordingSchedules,
        ) -> Result<<QueryRecordingSchedules as Message>::Reply, actlet::Error> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl Call<RegisterEmitter> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: RegisterEmitter,
        ) -> Result<<RegisterEmitter as Message>::Reply, actlet::Error> {
            Ok(())
        }
    }

    #[async_trait]
    impl Call<RemoveRecordingSchedule> for RecordingManagerStub {
        async fn call(
            &self,
            msg: RemoveRecordingSchedule,
        ) -> Result<<RemoveRecordingSchedule as Message>::Reply, actlet::Error> {
            match msg.program_quad.eid().value() {
                0 => Ok(Err(Error::ScheduleNotFound)),
                _ => Ok(Ok(Arc::new(Schedule {
                    program_quad: msg.program_quad,
                    start_at: Jst::now(),
                    end_at: Jst::now(),
                    content_path: "test.m2ts".into(),
                    priority: 1,
                    pre_filters: vec![],
                    post_filters: vec![],
                    tags: Default::default(),
                }))),
            }
        }
    }

    #[async_trait]
    impl Call<RemoveRecordingSchedules> for RecordingManagerStub {
        async fn call(
            &self,
            msg: RemoveRecordingSchedules,
        ) -> Result<<RemoveRecordingSchedules as Message>::Reply, actlet::Error> {
            match msg.target {
                RemoveTarget::All => Ok(()),
                RemoveTarget::Tag(tag) => {
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
        ) -> Result<<QueryRecordingRecorder as Message>::Reply, actlet::Error> {
            match msg.program_quad.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(RecorderModel {
                    schedule: Arc::new(Schedule {
                        program_quad: msg.program_quad,
                        start_at: Jst::now(),
                        end_at: Jst::now(),
                        content_path: "test.m2ts".into(),
                        priority: 1,
                        pre_filters: vec![],
                        post_filters: vec![],
                        tags: Default::default(),
                    }),
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
        ) -> Result<<QueryRecordingRecorders as Message>::Reply, actlet::Error> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl Call<StartRecording> for RecordingManagerStub {
        async fn call(
            &self,
            msg: StartRecording,
        ) -> Result<<StartRecording as Message>::Reply, actlet::Error> {
            match msg.schedule.program_quad.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(RecorderModel {
                    schedule: msg.schedule,
                    started_at: Jst::now(),
                    pipeline: vec![],
                })),
            }
        }
    }

    #[async_trait]
    impl Call<StopRecording> for RecordingManagerStub {
        async fn call(
            &self,
            msg: StopRecording,
        ) -> Result<<StopRecording as Message>::Reply, actlet::Error> {
            match msg.program_quad.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(())),
            }
        }
    }
}
// </coverage:exclude>
