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
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::BufWriter;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::command_util::spawn_pipeline;
use crate::command_util::CommandPipeline;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::datetime_ext::serde_jst;
use crate::datetime_ext::Jst;
use crate::epg;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::QueryClock;
use crate::epg::QueryProgram;
use crate::epg::QueryService;
use crate::epg::ServicesUpdated;
use crate::error::Error;
use crate::filter::FilterPipelineBuilder;
use crate::models::MirakurunProgramId;
use crate::models::MirakurunServiceId;
use crate::models::ServiceTriple;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerStreamStopTrigger;
use crate::tuner::TunerSubscriptionId;

pub struct RecordingManager<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    schedules: BinaryHeap<Arc<Schedule>>,
    schedule_map: HashMap<MirakurunProgramId, Arc<Schedule>>,
    recorders: HashMap<MirakurunProgramId, Recorder>,
    timer_token: Option<CancellationToken>,
    recording_started_emitters: Vec<Emitter<RecordingStarted>>,
    recording_stopped_emitters: Vec<Emitter<RecordingStopped>>,
}

const PREP_SECS: i64 = 15;

impl<T, E> RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        RecordingManager {
            config,
            tuner_manager,
            epg,
            schedules: Default::default(),
            schedule_map: Default::default(),
            recorders: Default::default(),
            timer_token: None,
            recording_started_emitters: Default::default(),
            recording_stopped_emitters: Default::default(),
        }
    }

    fn update_services(&mut self, services: Arc<IndexMap<ServiceTriple, EpgService>>) {
        let mut schedules = BinaryHeap::new();
        for schedule in self.schedules.drain() {
            let nid = schedule.program_id.nid();
            let sid = schedule.program_id.sid();
            if services
                .keys()
                .any(|triple| triple.nid() == nid && triple.sid() == sid)
            {
                schedules.push(schedule);
            } else {
                let msid = MirakurunServiceId::from(schedule.program_id);
                tracing::warn!(%schedule.program_id, %msid, "Invalidated");
                self.schedule_map.remove(&schedule.program_id);
            }
        }
        self.schedules = schedules;
        assert_eq!(self.schedules.len(), self.schedule_map.len());
    }

    async fn check_schedules(&mut self, ctx: &Context<Self>) {
        let prep_secs = Duration::seconds(PREP_SECS);
        let now = Jst::now();
        while let Some(schedule) = self.schedules.peek() {
            if schedule.start_at <= now {
                tracing::warn!(%schedule.program_id, "Expired, remove schedule");
                self.schedule_map.remove(&schedule.program_id);
                self.schedules.pop();
            } else if schedule.start_at - now <= prep_secs {
                self.schedule_map.remove(&schedule.program_id);
                let schedule = self.schedules.pop().unwrap();
                match self.start_recording(schedule.clone(), ctx).await {
                    Ok(_) => {
                        tracing::info!(%schedule.program_id, "Start recording");
                    }
                    Err(err) => {
                        tracing::error!(%err, %schedule.program_id, "Failed to start recording");
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
        let program_id = schedule.program_id;

        if self.recorders.contains_key(&program_id) {
            return Err(Error::AlreadyExists);
        }

        // TODO: copied from web::program_stream_g().

        let program = self
            .epg
            .call(QueryProgram::ByMirakurunProgramId(program_id))
            .await??;

        let service = self
            .epg
            .call(QueryService::ByMirakurunServiceId(program_id.into()))
            .await??;

        let clock = self
            .epg
            .call(QueryClock {
                triple: service.triple(),
            })
            .await??;

        let stream = self
            .tuner_manager
            .call(StartStreaming {
                channel: service.channel.clone(),
                user: TunerUser {
                    info: TunerUserInfo::Recorder {
                        name: format!("program#{}", program_id),
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
        if let Some(max_start_delay) = self.config.recording.max_start_delay {
            // Round off the fractional (nanosecond) part of the duration.
            //
            // The value can be safely converted into i64 because the value is less
            // than 24h.
            let duration = Duration::seconds(max_start_delay.as_secs() as i64);
            let wait_until = program.start_at + duration;
            builder = builder.insert("wait_until", &wait_until.timestamp_millis())?;
        }
        let data = builder.build();

        let mut builder = FilterPipelineBuilder::new(data);
        builder.add_pre_filters(&self.config.pre_filters, &schedule.pre_filters)?;
        if !stream.is_decoded() {
            builder.add_decode_filter(&self.config.filters.decode_filter)?;
        }
        builder.add_program_filter(&self.config.filters.program_filter)?;
        builder.add_post_filters(&self.config.post_filters, &schedule.post_filters)?;
        let (filters, content_type) = builder.build();

        let records_dir = self.config.recording.records_dir.as_ref().unwrap();
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
        let fut = fut.instrument(tracing::info_span!("pipeline", id = %pipeline.id()));
        ctx.spawn_task(fut);

        // Metadata file is always saved in the `records_dir` and no additional
        // sub-directories are created.
        let metadata_path = records_dir.join(program.metadata_filename());
        serde_json::to_writer_pretty(
            std::fs::File::create(&metadata_path)?,
            &Record {
                program: program.clone(),
                content_path: content_path.clone(),
                content_type,
                pre_filters: schedule.pre_filters.clone(),
                post_filters: schedule.post_filters.clone(),
                tags: schedule.tags.clone(),
            },
        )?;
        tracing::info!(%program_id, ?metadata_path, "Saved metadata");

        // Inner future in order to capture the result in an outer future.
        let inner_fut = {
            let content_path = content_path.clone();
            async move {
                let record = tokio::fs::File::create(&content_path).await?;
                let mut writer = BufWriter::new(record);
                // TODO: use Stdio
                Ok::<_, Error>(tokio::io::copy(&mut output, &mut writer).await?)
            }
        };
        // Outer future emits messages to observers.
        let outer_fut = {
            let addr = ctx.address().clone();
            async move {
                addr.emit(RecordingStarted { program_id }).await;
                let result = inner_fut.await.map_err(|err| format!("{}", err));
                addr.emit(RecordingStopped { program_id, result }).await;
            }
        };
        let fut = outer_fut.instrument(tracing::info_span!("writer", ?content_path));

        let recorder = Recorder {
            schedule,
            pipeline,
            start_time: Jst::now(),
            stop_trigger: Some(stop_trigger),
        };
        let model = recorder.get_model();
        self.recorders.insert(program_id, recorder);

        // Spawn the following task after the recorder is inserted so that
        // actors receiving RecordingStarted messages can access the recorder.
        ctx.spawn_task(fut);

        Ok(model)
    }

    fn stop_recording(&mut self, program_id: MirakurunProgramId) -> Result<(), Error> {
        match self.recorders.get_mut(&program_id) {
            Some(recorder) => {
                match recorder.stop_trigger.take() {
                    Some(_) => tracing::info!(%program_id, "Stop recording"),
                    None => tracing::warn!(%program_id, "Already stopped"),
                }
                Ok(())
            }
            None => {
                tracing::error!(%program_id, "No such recorder, maybe already stopped");
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
impl<T, E> Actor for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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

impl<T, E> RecordingManager<T, E> {
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
                    let _ = self.add_schedule(schedule);
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
impl<T, E> Handler<QueryRecordingSchedules> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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

impl<T, E> RecordingManager<T, E> {
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
    pub program_id: MirakurunProgramId,
}

#[async_trait]
impl<T, E> Handler<QueryRecordingSchedule> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingSchedule,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingSchedule", %msg.program_id);
        self.get_schedule(msg.program_id)
    }
}

impl<T, E> RecordingManager<T, E> {
    fn get_schedule(&self, program_id: MirakurunProgramId) -> Result<Arc<Schedule>, Error> {
        self.schedule_map
            .get(&program_id)
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
impl<T, E> Handler<AddRecordingSchedule> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: AddRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <AddRecordingSchedule as Message>::Reply {
        tracing::debug!(
            msg.name = "AddRecordingSchedule",
            %msg.schedule.program_id,
            ?msg.schedule.content_path,
            %msg.schedule.priority,
            ?msg.schedule.pre_filters,
            ?msg.schedule.post_filters,
            %msg.schedule.start_at,
        );
        let program_id = msg.schedule.program_id;
        self.add_schedule(msg.schedule)?;
        self.save_schedules();
        self.set_timer(ctx);
        self.get_schedule(program_id)
    }
}

impl<T, E> RecordingManager<T, E> {
    fn add_schedule(&mut self, schedule: Schedule) -> Result<(), Error> {
        let now = Jst::now();
        let program_id = schedule.program_id;
        if self.schedule_map.contains_key(&program_id) {
            let err = Error::AlreadyExists;
            tracing::warn!(%schedule.program_id, %err);
            Err(err)
        } else if schedule.start_at <= now {
            let err = Error::ProgramAlreadyStarted;
            tracing::error!(%schedule.program_id, %err);
            Err(err)
        } else if schedule.start_at - now <= Duration::seconds(PREP_SECS) {
            let err = Error::ProgramWillStartSoon;
            tracing::error!(%schedule.program_id, %err);
            Err(err)
        } else {
            tracing::info!(%schedule.program_id, "Added");
            let schedule = Arc::new(schedule);
            self.schedules.push(schedule.clone());
            self.schedule_map.insert(program_id, schedule);
            assert_eq!(self.schedules.len(), self.schedule_map.len());
            Ok(())
        }
    }
}

// remove recording schedule

#[derive(Message)]
#[reply("Result<Arc<Schedule>, Error>")]
pub struct RemoveRecordingSchedule {
    pub program_id: MirakurunProgramId,
}

#[async_trait]
impl<T, E> Handler<RemoveRecordingSchedule> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedule,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedule as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedule", %msg.program_id);
        match self.schedule_map.remove(&msg.program_id) {
            Some(schedule) => {
                self.sync_schedules();
                self.save_schedules();
                self.set_timer(ctx);
                Ok(schedule)
            }
            None => {
                tracing::warn!(%msg.program_id, "No such schedule");
                Err(Error::ScheduleNotFound)
            }
        }
    }
}

impl<T, E> RecordingManager<T, E> {
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
impl<T, E> Handler<RemoveRecordingSchedules> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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

impl<T, E> RecordingManager<T, E> {
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
impl<T, E> Handler<QueryRecordingRecorders> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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
    pub program_id: MirakurunProgramId,
}

#[async_trait]
impl<T, E> Handler<QueryRecordingRecorder> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingRecorder,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecorder as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecorder", %msg.program_id);
        self.recorders
            .get(&msg.program_id)
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
impl<T, E> Handler<StartRecording> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: StartRecording,
        ctx: &mut Context<Self>,
    ) -> <StartRecording as Message>::Reply {
        tracing::debug!(
            msg.name = "StartRecording",
            %msg.schedule.program_id,
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
    pub program_id: MirakurunProgramId,
}

#[async_trait]
impl<T, E> Handler<StopRecording> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: StopRecording,
        _ctx: &mut Context<Self>,
    ) -> <StopRecording as Message>::Reply {
        tracing::debug!(msg.name = "StopRecording", %msg.program_id);
        self.stop_recording(msg.program_id)
    }
}

// query recording records

#[derive(Message)]
#[reply("Result<Vec<Record>, Error>")]
pub struct QueryRecordingRecords;

#[async_trait]
impl<T, E> Handler<QueryRecordingRecords> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryRecordingRecords,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecords as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecords");
        self.query_records().await
    }
}

impl<T, E> RecordingManager<T, E> {
    async fn query_records(&self) -> Result<Vec<Record>, Error> {
        let records_dir = self
            .config
            .recording
            .records_dir
            .as_ref()
            .unwrap()
            .to_str()
            .unwrap();
        let pattern = format!("{}/*.record.json", records_dir);
        let mut records = vec![];
        for path in glob::glob(&pattern).unwrap() {
            if let Ok(path) = path {
                let record = serde_json::from_reader(std::fs::File::open(&path)?)?;
                records.push(record);
            }
        }
        Ok(records)
    }
}

// query recording record

#[derive(Message)]
#[reply("Result<Record, Error>")]
pub struct QueryRecordingRecord {
    pub id: String,
}

#[async_trait]
impl<T, E> Handler<QueryRecordingRecord> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: QueryRecordingRecord,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingRecord as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingRecord", msg.id);
        self.query_record(&msg.id)
    }
}

impl<T, E> RecordingManager<T, E> {
    fn query_record(&self, id: &str) -> Result<Record, Error> {
        let path = self
            .config
            .recording
            .records_dir
            .as_ref()
            .unwrap()
            .join(id)
            .with_extension("record.json");
        if !path.exists() {
            return Err(Error::RecordNotFound);
        }
        Ok(serde_json::from_reader(std::fs::File::open(&path)?)?)
    }
}

// remove recording record

#[derive(Message)]
#[reply("Result<(), Error>")]
pub struct RemoveRecordingRecord {
    pub id: String,
    pub remove_content: bool,
}

#[async_trait]
impl<T, E> Handler<RemoveRecordingRecord> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingRecord,
        _ctx: &mut Context<Self>,
    ) -> <RemoveRecordingRecord as Message>::Reply {
        tracing::debug!(
            msg.name = "RemoveRecordingRecord",
            msg.id,
            msg.remove_content
        );
        self.remove_record(&msg.id, msg.remove_content)
    }
}

impl<T, E> RecordingManager<T, E> {
    fn remove_record(&self, id: &str, remove_content: bool) -> Result<(), Error> {
        let metadata_path = self
            .config
            .recording
            .records_dir
            .as_ref()
            .unwrap()
            .join(id)
            .with_extension("record.json");
        if !metadata_path.exists() {
            return Err(Error::RecordNotFound);
        }
        let record: Record = serde_json::from_reader(std::fs::File::open(&metadata_path)?)?;
        if remove_content {
            tracing::info!(?record.content_path, "Remove");
            std::fs::remove_file(&record.content_path)?;
        }
        tracing::info!(?metadata_path, "Remove");
        std::fs::remove_file(&metadata_path)?;
        Ok(())
    }
}

// services updated

#[async_trait]
impl<T, E> Handler<ServicesUpdated> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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
impl<T, E> Handler<TimerExpired> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
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
}

#[async_trait]
impl<T, E> Handler<RegisterEmitter> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RegisterEmitter,
        _ctx: &mut Context<Self>,
    ) -> <RegisterEmitter as Message>::Reply {
        match msg {
            RegisterEmitter::RecordingStarted(emitter) => {
                self.recording_started_emitters.push(emitter)
            }
            RegisterEmitter::RecordingStopped(emitter) => {
                self.recording_stopped_emitters.push(emitter)
            }
        }
    }
}

// recording started

#[derive(Clone, Message)]
pub struct RecordingStarted {
    pub program_id: MirakurunProgramId,
}

#[async_trait]
impl<T, E> Handler<RecordingStarted> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingStarted, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_id);
        for emitter in self.recording_started_emitters.iter() {
            emitter.emit(msg.clone()).await;
        }
    }
}

// recording stopped

#[derive(Clone, Message)]
pub struct RecordingStopped {
    pub program_id: MirakurunProgramId,
    pub result: Result<u64, String>,
}

#[async_trait]
impl<T, E> Handler<RecordingStopped> for RecordingManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<epg::RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingStopped, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_id, ?msg.result);
        self.recorders.remove(&msg.program_id);
        for emitter in self.recording_stopped_emitters.iter() {
            emitter.emit(msg.clone()).await;
        }
    }
}

// models

#[derive(Clone, Debug, Deserialize, Eq, Serialize)]
pub struct Schedule {
    pub program_id: MirakurunProgramId,
    pub content_path: PathBuf,
    pub priority: i32,
    pub pre_filters: Vec<String>,
    pub post_filters: Vec<String>,
    pub tags: HashSet<String>,
    #[serde(with = "serde_jst")]
    pub start_at: DateTime<Jst>,
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
        self.program_id == other.program_id
    }
}

impl PartialOrd for Schedule {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct Recorder {
    schedule: Arc<Schedule>,
    pipeline: CommandPipeline<TunerSubscriptionId>,
    start_time: DateTime<Jst>,
    stop_trigger: Option<TunerStreamStopTrigger>,
}

impl Recorder {
    fn get_model(&self) -> RecorderModel {
        RecorderModel {
            schedule: self.schedule.clone(),
            pipeline: self.pipeline.get_model(),
            start_time: self.start_time,
        }
    }
}

pub struct RecorderModel {
    pub schedule: Arc<Schedule>,
    pub pipeline: Vec<CommandPipelineProcessModel>,
    pub start_time: DateTime<Jst>,
}

impl EpgProgram {
    const METADATA_FILE_DATE_FORMAT: &'static str = "%Y%d%m%H%M";

    pub(crate) fn record_id(&self) -> String {
        let datetime = self.start_at.format(Self::METADATA_FILE_DATE_FORMAT);
        format!("{}_{}", datetime, self.quad)
    }

    fn metadata_filename(&self) -> String {
        let datetime = self.start_at.format(Self::METADATA_FILE_DATE_FORMAT);
        format!("{}_{}.record.json", datetime, self.quad)
    }
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

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::epg::stub::EpgStub;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use static_assertions::const_assert;
    use tempfile::TempDir;

    #[test]
    fn test_add_schedule() {
        let now = Jst::now();
        Jst::freeze(now);

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager = RecordingManager::new(config, TunerManagerStub, EpgStub);

        let schedule = schedule_for_test((0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::AlreadyExists));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 2).into(), now + Duration::seconds(PREP_SECS));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::ProgramWillStartSoon));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 3).into(), now);
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Err(Error::ProgramAlreadyStarted));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 4).into(), now + Duration::seconds(PREP_SECS + 2));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);
        assert_matches!(manager.schedules.peek(), Some(schedule) => {
            assert_eq!(schedule.program_id, (0, 0, 1).into());
        });
    }

    #[test]
    fn test_query_schedules() {
        let now = Jst::now();
        Jst::freeze(now);

        let temp_dir = TempDir::new().unwrap();
        let config = config_for_test(temp_dir.path());

        let mut manager = RecordingManager::new(config, TunerManagerStub, EpgStub);

        let schedule = schedule_for_test((0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let schedule = schedule_for_test((0, 0, 2).into(), now + Duration::seconds(PREP_SECS + 2));
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);

        let schedules = manager.query_schedules();
        assert_eq!(schedules.len(), 2);
        assert_matches!(schedules.get(0), Some(schedule) => {
            assert_eq!(schedule.program_id, (0, 0, 1).into());
        });
    }

    #[test]
    fn test_remove_schedules() {
        let now = Jst::now();
        Jst::freeze(now);

        let config = config_for_test("/tmp");

        let mut manager = RecordingManager::new(config, TunerManagerStub, EpgStub);

        let mut schedule =
            schedule_for_test((0, 0, 1).into(), now + Duration::seconds(PREP_SECS + 1));
        schedule.tags.insert("tag1".to_string());
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 1);

        let mut schedule =
            schedule_for_test((0, 0, 2).into(), now + Duration::seconds(PREP_SECS + 1));
        schedule.tags.insert("tag2".to_string());
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        assert_eq!(manager.schedules.len(), 2);

        manager.remove_schedules(RemoveTarget::Tag("tag2".to_string()));
        assert!(manager.schedule_map.contains_key(&(0, 0, 1).into()));

        // Schedules which will start soon are always retained.
        Jst::freeze(now + Duration::seconds(PREP_SECS));
        manager.remove_schedules(RemoveTarget::Tag("tag1".to_string()));
        assert!(manager.schedule_map.contains_key(&(0, 0, 1).into()));

        // Remove all schedules regardless of whether a schedule will start soon
        // or not.
        manager.remove_schedules(RemoveTarget::All);
        assert!(manager.schedules.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_recording() {
        let now = Jst::now();
        Jst::freeze(now);

        let temp_dir = TempDir::new().unwrap();

        let system = System::new();
        {
            let manager = system
                .spawn_actor(RecordingManager::new(
                    config_for_test(&temp_dir),
                    TunerManagerStub,
                    EpgStub,
                ))
                .await;

            // Schedules for programs already started will be ignored.
            let schedule = Schedule {
                program_id: (0, 1, 1).into(),
                content_path: "1.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
                start_at: now,
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            // Schedules for programs start within PREP_SECS will be performed.
            assert_matches!(result, Ok(_));
            const_assert!(PREP_SECS >= 1);
            let schedule = Schedule {
                program_id: (0, 1, 2).into(),
                content_path: "2.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
                start_at: now + Duration::seconds(1),
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            // Schedules for programs start after PREP_SECS will be kept.
            let schedule = Schedule {
                program_id: (0, 1, 3).into(),
                content_path: "3.m2ts".into(),
                priority: 0,
                pre_filters: vec![],
                post_filters: vec![],
                tags: Default::default(),
                start_at: now + Duration::seconds(PREP_SECS + 1),
            };
            let result = manager.call(AddRecordingSchedule { schedule }).await;
            assert_matches!(result, Ok(Ok(_)));

            let start_time = std::time::Instant::now();
            loop {
                let schedules = manager.call(QueryRecordingSchedules).await.unwrap();
                let recorders = manager.call(QueryRecordingRecorders).await.unwrap();
                let records = manager.call(QueryRecordingRecords).await.unwrap().unwrap();
                if !schedules.is_empty() && recorders.is_empty() && !records.is_empty() {
                    assert_eq!(schedules.len(), 1);
                    assert_eq!(records.len(), 1);
                    break;
                }
                assert!(start_time.elapsed().as_secs() < 1);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            assert!(!temp_dir.path().join("1.m2ts").exists());
            assert!(!temp_dir
                .path()
                .join(EpgProgram::new((0, 0, 1, 1).into()).metadata_filename())
                .exists());
            assert!(temp_dir.path().join("2.m2ts").is_file());
            assert!(temp_dir
                .path()
                .join(EpgProgram::new((0, 0, 1, 2).into()).metadata_filename())
                .is_file());
            assert!(!temp_dir.path().join("3.m2ts").exists());
            assert!(!temp_dir
                .path()
                .join(EpgProgram::new((0, 0, 1, 3).into()).metadata_filename())
                .exists());

            let result = manager
                .inspect(|actor| {
                    assert_eq!(actor.schedules.len(), 1);
                    assert_eq!(actor.schedule_map.len(), 1);
                    assert!(actor.schedule_map.contains_key(&(0, 1, 3).into()));
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

    fn schedule_for_test(program_id: MirakurunProgramId, start_at: DateTime<Jst>) -> Schedule {
        Schedule {
            program_id,
            content_path: format!("{}.m2ts", program_id.eid().value()).into(),
            priority: Default::default(),
            pre_filters: Default::default(),
            post_filters: Default::default(),
            tags: Default::default(),
            start_at,
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
            match msg.schedule.program_id.eid().value() {
                // 0 is reserved for Error::ProgramNotFound
                1 => Ok(Err(Error::AlreadyExists)),
                2 => Ok(Err(Error::ProgramAlreadyStarted)),
                3 => Ok(Err(Error::ProgramWillStartSoon)),
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
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::ProgramNotFound)),
                _ => Ok(Ok(Arc::new(Schedule {
                    program_id: msg.program_id,
                    content_path: "test.m2ts".into(),
                    priority: 1,
                    pre_filters: vec![],
                    post_filters: vec![],
                    tags: Default::default(),
                    start_at: Jst::now(),
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
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::ScheduleNotFound)),
                _ => Ok(Ok(Arc::new(Schedule {
                    program_id: msg.program_id,
                    content_path: "test.m2ts".into(),
                    priority: 1,
                    pre_filters: vec![],
                    post_filters: vec![],
                    tags: Default::default(),
                    start_at: Jst::now(),
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
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(RecorderModel {
                    schedule: Arc::new(Schedule {
                        program_id: msg.program_id,
                        content_path: "test.m2ts".into(),
                        priority: 1,
                        pre_filters: vec![],
                        post_filters: vec![],
                        tags: Default::default(),
                        start_at: Jst::now(),
                    }),
                    pipeline: vec![],
                    start_time: Jst::now(),
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
            match msg.schedule.program_id.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(RecorderModel {
                    schedule: msg.schedule,
                    pipeline: vec![],
                    start_time: Jst::now(),
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
            match msg.program_id.eid().value() {
                0 => Ok(Err(Error::RecorderNotFound)),
                _ => Ok(Ok(())),
            }
        }
    }

    #[async_trait]
    impl Call<QueryRecordingRecords> for RecordingManagerStub {
        async fn call(
            &self,
            _msg: QueryRecordingRecords,
        ) -> Result<<QueryRecordingRecords as Message>::Reply, actlet::Error> {
            Ok(Ok(vec![]))
        }
    }

    #[async_trait]
    impl Call<QueryRecordingRecord> for RecordingManagerStub {
        async fn call(
            &self,
            msg: QueryRecordingRecord,
        ) -> Result<<QueryRecordingRecord as Message>::Reply, actlet::Error> {
            match msg.id.as_str() {
                "0" => Ok(Err(Error::RecordNotFound)),
                _ => Ok(Ok(Record {
                    program: EpgProgram::new((0, 0, 1, 1).into()),
                    content_path: "/dev/null".into(),
                    content_type: "video/MP2T".into(),
                    pre_filters: vec![],
                    post_filters: vec![],
                    tags: Default::default(),
                })),
            }
        }
    }

    #[async_trait]
    impl Call<RemoveRecordingRecord> for RecordingManagerStub {
        async fn call(
            &self,
            msg: RemoveRecordingRecord,
        ) -> Result<<RemoveRecordingRecord as Message>::Reply, actlet::Error> {
            match msg.id.as_str() {
                "0" => Ok(Err(Error::RecordNotFound)),
                "remove_content" => {
                    assert!(msg.remove_content);
                    Ok(Ok(()))
                }
                _ => {
                    assert!(!msg.remove_content);
                    Ok(Ok(()))
                }
            }
        }
    }
}
// </coverage:exclude>
