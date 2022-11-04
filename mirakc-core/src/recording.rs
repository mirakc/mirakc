use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use chrono::DateTime;
use chrono::Duration;
use indexmap::IndexMap;
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
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::QueryClock;
use crate::epg::QueryProgram;
use crate::epg::QueryService;
use crate::epg::RegisterEmitter;
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
    E: Call<RegisterEmitter>,
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
            let program_id = schedule.program_id;
            if schedule.start_at <= now {
                tracing::warn!(schedule.program_id = %program_id, "Expired, remove schedule");
                self.schedule_map.remove(&program_id);
                self.schedules.pop();
            } else if schedule.start_at - now <= prep_secs {
                self.schedule_map.remove(&program_id);
                let schedule = self.schedules.pop().unwrap();
                match self.start_recording(schedule, ctx).await {
                    Ok(_) => {
                        tracing::info!(schedule.program_id = %program_id, "Start recording");
                    }
                    Err(err) => {
                        tracing::error!(schedule.program_id = %program_id, %err, "Failed to start recording");
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
        if let Some(max_start_delay) = self.config.recorder.max_start_delay {
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

        let mut pipeline = spawn_pipeline(filters, stream.id())?;
        let (input, mut output) = pipeline.take_endpoints()?;

        let fut = async move {
            let _ = stream.pipe(input).await;
        };
        let fut = fut.instrument(tracing::info_span!("pipeline", id = %pipeline.id()));
        ctx.spawn_task(fut);

        let record_dir = self.config.recorder.record_dir.as_ref().unwrap();

        let content_path = if schedule.content_path.is_absolute() {
            schedule.content_path.clone()
        } else {
            record_dir.join(&schedule.content_path)
        };
        // We assumed that schedule.content_path has already been normalized.
        if let Some(dir) = content_path.parent() {
            // Create missing directories if they don't exist.
            tokio::fs::create_dir_all(dir).await?;
        }

        // Metadata file is always saved in the `record_dir` and no additional
        // sub-directories are created.
        let metadata_path = record_dir.join(program.metadata_filename());
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

        // TODO: use Stdio
        let fut = {
            let content_path = content_path.clone();
            let addr = ctx.address().clone();
            async move {
                match tokio::fs::File::create(&content_path).await {
                    Ok(record) => {
                        tracing::info!(%program_id, "Recording...");
                        let mut writer = BufWriter::new(record);
                        match tokio::io::copy(&mut output, &mut writer).await {
                            Ok(nwritten) => {
                                tracing::info!(%program_id, nwritten, "Stopped recording")
                            }
                            Err(err) => tracing::error!(%program_id, %err),
                        }
                    }
                    Err(err) => tracing::error!(%err),
                }
                addr.emit(RecordingStopped { program_id }).await;
            }
        };
        let fut = fut.instrument(tracing::info_span!("writer", ?content_path));
        ctx.spawn_task(fut);

        let recorder = Recorder {
            schedule,
            pipeline,
            start_time: Jst::now(),
            stop_trigger: Some(stop_trigger),
        };
        let model = recorder.get_model();
        self.recorders.insert(program_id, recorder);

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
    E: Call<RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");
        self.epg
            .call(RegisterEmitter::ServicesUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for ServicesUpdated");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
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
    E: Call<RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryRecordingSchedules,
        _ctx: &mut Context<Self>,
    ) -> <QueryRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "QueryRecordingSchedules");
        self.schedules.clone().into_sorted_vec()
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
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
        self.set_timer(ctx);
        self.get_schedule(program_id)
    }
}

impl<T, E> RecordingManager<T, E> {
    fn add_schedule(&mut self, schedule: Schedule) -> Result<(), Error> {
        let program_id = schedule.program_id;
        if self.schedule_map.contains_key(&program_id) {
            tracing::warn!(%schedule.program_id, "Already exists");
            Err(Error::AlreadyExists)
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
{
    async fn handle(
        &mut self,
        msg: RemoveRecordingSchedules,
        ctx: &mut Context<Self>,
    ) -> <RemoveRecordingSchedules as Message>::Reply {
        tracing::debug!(msg.name = "RemoveRecordingSchedules", ?msg.target);
        self.remove_schedules(msg.target);
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
        tracing::info!("Remove schedules tagged with {}", tag);
        self.schedule_map.retain(|_, v| !v.tags.contains(tag));
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
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
    E: Call<RegisterEmitter>,
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
        let record_dir = self
            .config
            .recorder
            .record_dir
            .as_ref()
            .unwrap()
            .to_str()
            .unwrap();
        let pattern = format!("{}/*.record.json", record_dir);
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
    E: Call<RegisterEmitter>,
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
            .recorder
            .record_dir
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
    E: Call<RegisterEmitter>,
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
            .recorder
            .record_dir
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
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, msg: ServicesUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        self.update_services(msg.services);
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
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, _msg: TimerExpired, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "TimerExpired");
        self.check_schedules(ctx).await;
        self.set_timer(ctx);
    }
}

// recording stopped

#[derive(Message)]
struct RecordingStopped {
    program_id: MirakurunProgramId,
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
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, msg: RecordingStopped, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_id);
        self.recorders.remove(&msg.program_id);
    }
}

// models

#[derive(Clone, Debug, Deserialize, Eq)]
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

    #[tokio::test]
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

            // TODO: Wait until all recorders stop.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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

    fn config_for_test(temp_dir: &TempDir) -> Arc<Config> {
        let mut config = Config::default();
        config.recorder.record_dir = Some(temp_dir.path().to_owned());
        Arc::new(config)
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
                0 => Ok(Err(Error::ScheduleNotFound)),
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
