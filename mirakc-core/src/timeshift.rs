use std::fmt;
use std::future::Future;
use std::io;
use std::io::SeekFrom;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use actlet::prelude::*;
use chrono::DateTime;
use chrono_jst::serde::ts_milliseconds;
use chrono_jst::Jst;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeek;
use tokio::io::AsyncSeekExt;
use tokio::io::BufReader;
use tokio::io::ReadBuf;
use tokio::io::Take;
use tokio::sync::oneshot;
use tokio_util::io::ReaderStream;

use crate::command_util::*;
use crate::config::*;
use crate::epg::*;
use crate::error::Error;
use crate::file_util;
use crate::filter::*;
use crate::models::*;
use crate::mpeg_ts_stream::*;
use crate::tuner::*;

// timeshift manager

type TimeshiftLiveStream = MpegTsStream<String, ReaderStream<TimeshiftFileReader>>;
type TimeshiftRecordStream = MpegTsStream<String, ReaderStream<Take<TimeshiftFileReader>>>;

pub struct TimeshiftManager<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    recorders: IndexMap<String, TimeshiftManagerRecorderHolder>,
}

struct TimeshiftManagerRecorderHolder {
    activated: bool,
    reactivation_count: usize,
    addr: Address<TimeshiftRecorder>,
}

impl<T, E> TimeshiftManager<T, E> {
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        TimeshiftManager {
            config,
            tuner_manager,
            epg,
            recorders: IndexMap::new(),
        }
    }
}

#[async_trait]
impl<T, E> Actor for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");

        self.epg
            .call(RegisterEmitter::ServicesUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register the emitter");

        let mut recorders = IndexMap::new();
        for (index, name) in self.config.timeshift.recorders.keys().enumerate() {
            let recorder = ctx
                .spawn_actor(TimeshiftRecorder::new(
                    index,
                    name.clone(),
                    self.config.clone(),
                    ctx.address().clone().into(),
                ))
                .await;
            recorders.insert(
                name.clone(),
                TimeshiftManagerRecorderHolder {
                    activated: false,
                    reactivation_count: 0,
                    addr: recorder,
                },
            );
        }
        self.recorders = recorders;
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

#[derive(Message)]
#[reply("Result<Vec<TimeshiftRecorderModel>, Error>")]
pub struct QueryTimeshiftRecorders;

#[async_trait]
impl<T, E> Handler<QueryTimeshiftRecorders> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn handle(
        &mut self,
        _msg: QueryTimeshiftRecorders,
        _ctx: &mut Context<Self>,
    ) -> <QueryTimeshiftRecorders as Message>::Reply {
        let mut models = vec![];
        for (index, recorder) in self.recorders.values().enumerate() {
            models.push(
                recorder
                    .addr
                    .call(QueryTimeshiftRecorder {
                        recorder: TimeshiftRecorderQuery::ByIndex(index),
                    })
                    .await??,
            );
        }
        Ok(models)
    }
}

macro_rules! impl_proxy_handler {
    ($msg:ty) => {
        #[async_trait]
        impl<T, E> Handler<$msg> for TimeshiftManager<T, E>
        where
            T: Clone + Send + Sync + 'static,
            T: Call<StartStreaming>,
            T: Into<Emitter<StopStreaming>>,
            E: Send + Sync + 'static,
            E: Call<RegisterEmitter>,
        {
            async fn handle(
                &mut self,
                msg: $msg,
                _ctx: &mut Context<Self>,
            ) -> <$msg as Message>::Reply {
                tracing::debug!(msg.name = stringify!(msg));
                let maybe_recorder = match msg.recorder {
                    TimeshiftRecorderQuery::ByIndex(index) => self
                        .recorders
                        .get_index(index)
                        .map(|(_, recorder)| recorder.addr.clone())
                        .ok_or(Error::RecordNotFound),
                    TimeshiftRecorderQuery::ByName(ref name) => self
                        .recorders
                        .get(name)
                        .map(|recorder| recorder.addr.clone())
                        .ok_or(Error::RecordNotFound),
                };
                maybe_recorder?.call(msg).await?
            }
        }
    };
}

pub enum TimeshiftRecorderQuery {
    ByIndex(usize),
    ByName(String),
}

impl fmt::Display for TimeshiftRecorderQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            TimeshiftRecorderQuery::ByIndex(index) => write!(f, "index({})", index),
            TimeshiftRecorderQuery::ByName(ref name) => write!(f, "name({})", name),
        }
    }
}

#[derive(Message)]
#[reply("Result<TimeshiftRecorderModel, Error>")]
pub struct QueryTimeshiftRecorder {
    pub recorder: TimeshiftRecorderQuery,
}

impl fmt::Display for QueryTimeshiftRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryTimeshiftRecorder: recorder={}", self.recorder)
    }
}

impl_proxy_handler!(QueryTimeshiftRecorder);

#[derive(Message)]
#[reply("Result<Vec<TimeshiftRecordModel>, Error>")]
pub struct QueryTimeshiftRecords {
    pub recorder: TimeshiftRecorderQuery,
}

impl fmt::Display for QueryTimeshiftRecords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryTimeshiftRecords: recoder={}", self.recorder)
    }
}

impl_proxy_handler!(QueryTimeshiftRecords);

#[derive(Message)]
#[reply("Result<TimeshiftRecordModel, Error>")]
pub struct QueryTimeshiftRecord {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: TimeshiftRecordId,
}

impl fmt::Display for QueryTimeshiftRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "QueryTimeshiftRecord: recoder={} record_id={}",
            self.recorder, self.record_id
        )
    }
}

impl_proxy_handler!(QueryTimeshiftRecord);

#[derive(Message)]
#[reply("Result<TimeshiftLiveStreamSource, Error>")]
pub struct CreateTimeshiftLiveStreamSource {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: Option<TimeshiftRecordId>,
}

impl fmt::Display for CreateTimeshiftLiveStreamSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.record_id {
            Some(id) => {
                write!(
                    f,
                    "CreateTimeshiftLiveStreamSource: recoder={} record_id={}",
                    self.recorder, id
                )
            }
            None => {
                write!(
                    f,
                    "CreateTimeshiftLiveStreamSource: recorder={}",
                    self.recorder
                )
            }
        }
    }
}

impl_proxy_handler!(CreateTimeshiftLiveStreamSource);

#[derive(Message)]
#[reply("Result<TimeshiftRecordStreamSource, Error>")]
pub struct CreateTimeshiftRecordStreamSource {
    pub recorder: TimeshiftRecorderQuery,
    pub record_id: TimeshiftRecordId,
    pub start_pos: Option<u64>,
}

impl fmt::Display for CreateTimeshiftRecordStreamSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.start_pos {
            Some(pos) => {
                write!(
                    f,
                    "CreateTimeshiftRecordStreamSource: \
                           recorder={} record_id={} start_pos={}",
                    self.recorder, self.record_id, pos
                )
            }
            None => {
                write!(
                    f,
                    "CreateTimeshiftRecordStreamSource: recorder={} record_id={}",
                    self.recorder, self.record_id
                )
            }
        }
    }
}

impl_proxy_handler!(CreateTimeshiftRecordStreamSource);

#[async_trait]
impl<T, E> Handler<ServicesUpdated> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, msg: ServicesUpdated, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ServicesUpdated");
        for (name, config) in self.config.clone().timeshift.recorders.iter() {
            let triple = ServiceTriple::from(config.service_triple.clone());
            if msg.services.contains_key(&triple) {
                tracing::info!("{}: Service#{} is available, activate", name, triple);
                let service = msg.services[&triple].clone();
                self.recorders[name].activated = true;
                self.recorders[name].reactivation_count = 0;
                self.recorders[name]
                    .addr
                    .emit(ActivateTimeshiftRecorder {
                        service,
                        tuner_manager: self.tuner_manager.clone(),
                    })
                    .await;
            } else {
                tracing::warn!("{}: Service#{} is unavailable, deactivate", name, triple);
                self.recorders[name].activated = false;
                self.recorders[name]
                    .addr
                    .emit(DeactivateTimeshiftRecorder)
                    .await;
            }
        }
    }
}

#[derive(Message)]
pub struct ReactivateTimeshiftRecorder {
    name: String,
    service: EpgService,
}

#[async_trait]
impl<T, E> Handler<ReactivateTimeshiftRecorder> for TimeshiftManager<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, msg: ReactivateTimeshiftRecorder, _ctx: &mut Context<Self>) {
        const MAX_REACTIVATION_COUNT: usize = 5;

        if self.recorders[&msg.name].activated {
            if self.recorders[&msg.name].reactivation_count < MAX_REACTIVATION_COUNT {
                tracing::warn!(
                    "{}: Stopped recording due to some accident, activate it again",
                    msg.name
                );
                self.recorders[&msg.name].reactivation_count += 1;
                self.recorders[&msg.name]
                    .addr
                    .emit(ActivateTimeshiftRecorder {
                        service: msg.service,
                        tuner_manager: self.tuner_manager.clone(),
                    })
                    .await;
            } else {
                tracing::error!(
                    "{}: Stopped recording due to some accident, \
                     reactivation count reached the maximum number",
                    msg.name
                );
            }
        } else {
            tracing::debug!("{}: Already deactivated", msg.name);
        }
    }
}

// recorder

struct TimeshiftRecorder {
    index: usize,
    name: String,
    config: Arc<Config>,
    activator: Emitter<ReactivateTimeshiftRecorder>,
    service: EpgService,
    records: IndexMap<TimeshiftRecordId, TimeshiftRecord>,
    points: Vec<TimeshiftPoint>,
    session: Option<TimeshiftRecorderSession>,
}

impl TimeshiftRecorder {
    fn new(
        index: usize,
        name: String,
        config: Arc<Config>,
        activator: Emitter<ReactivateTimeshiftRecorder>,
    ) -> Self {
        let recorder_config = &config.timeshift.recorders[&name];
        let service = EpgService {
            // dummy data
            nid: recorder_config.service_triple.0,
            tsid: recorder_config.service_triple.1,
            sid: recorder_config.service_triple.2,
            service_type: 0,
            logo_id: 0,
            remote_control_key_id: 0,
            name: "".to_string(),
            channel: EpgChannel {
                name: "".to_string(),
                channel_type: ChannelType::GR,
                channel: "".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
            },
        };
        let max_chunks = recorder_config.max_chunks();
        TimeshiftRecorder {
            index,
            name,
            config,
            activator,
            service,
            records: IndexMap::new(),
            points: Vec::with_capacity(max_chunks),
            session: None,
        }
    }

    fn config(&self) -> &TimeshiftRecorderConfig {
        &self.config.timeshift.recorders[&self.name]
    }

    fn load_data(&mut self) {
        match self.do_load_data() {
            Ok(n) => {
                if n == 0 {
                    tracing::debug!(recorder.name = self.name, "No records saved");
                } else {
                    tracing::info!(
                        recorder.name = self.name,
                        recorder.records.len = n,
                        "Loaded records from <data-file> successfully",
                    );
                }
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    recorder.name = self.name,
                    "Failed to load records from <data-file>",
                );
                tracing::error!(
                    recorder.name = self.name,
                    "Recover or simply remove <data-file>",
                );
                std::process::exit(libc::EXIT_FAILURE);
                // never reach here
            }
        }
    }

    fn do_load_data(&mut self) -> Result<usize, Error> {
        if !Path::new(&self.config().data_file).exists() {
            return Ok(0);
        }

        let file = std::fs::File::open(&self.config().data_file)?;
        // If the file is empty, serde_json::from_reader() always causes a parse
        // error even though serde_json reads no data actually.
        if file.metadata()?.len() == 0 {
            return Ok(0);
        }

        let data: TimeshiftRecorderData = serde_json::from_reader(file)?;
        let mut invalid = false;
        if self.service.triple() != data.service.triple() {
            tracing::error!(
                recorder.data_file = %self.config().data_file,
                recorder.service_triple =  %self.service.triple(),
                data.service_triple = %data.service.triple(),
                "Not matched",
            );
            invalid = true;
        }
        if self.config().chunk_size != data.chunk_size {
            tracing::error!(
                recorder.data_file = %self.config().data_file,
                recorder.chunk_size =  self.config().chunk_size,
                data.chunk_size,
                "Not matched",
            );
            invalid = true;
        }
        if self.config().max_chunks() != data.max_chunks {
            tracing::error!(
                recorder.data_file = %self.config().data_file,
                recorder.max_chunks = self.config().max_chunks(),
                data.max_chunks,
                "Not matched",
            );
            invalid = true;
        }
        if invalid {
            return Err(Error::TimeshiftConfigInconsistent);
        }
        self.records = data.records;
        self.points = data.points; // Don't remove the last item here.
        Ok(self.records.len())
    }

    fn save_data(&self) {
        let service = &self.service;
        let chunk_size = self.config().chunk_size;
        let max_chunks = self.config().max_chunks();

        let records = &self.records;
        if records.is_empty() {
            tracing::debug!(recorder.name = self.name, "No records to save");
            return;
        }

        // The last item will be used as a sentinel and removed before recording starts.
        let points = &self.points;
        let data = TimeshiftRecorderDataForSave {
            service,
            chunk_size,
            max_chunks,
            records,
            points,
        };

        // Serialize records in advance in order to improve error traceability.
        let buf = match serde_json::to_vec(&data) {
            Ok(buf) => {
                tracing::debug!(recorder.name = self.name, "Serialized records successfully");
                buf
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    recorder.name = self.name,
                    "Failed to serialize records",
                );
                return;
            }
        };

        // issue#676
        // ---------
        // In order to keep records as much as possible, we perform the following steps
        //
        //   1. Create <data-file>.new file and write the serialized data to it
        //   2. Rename <data-file>.new to <data-file>
        //
        // If this function fails, inconsistency between <data-file> and <ts-file> happens.
        // mirakc cannot recover this situation by itself and this must be resolved by
        // the user.  For example, the user might have to remove some files in order to
        // make enough space in the filesystem.
        //
        // If mirakc restarts before resolving the inconsistent situation, mirakc will
        // start timeshift recording based on the *old* data file.  As a result, newer
        // records will be lost.  Additionally, TS packets for older records will be
        // lost if a wrap-around occurred in the TS file.
        if !file_util::save_data(&buf, &self.config().data_file) {
            tracing::error!(
                recorder.name = self.name,
                "Sync between <ts-file> and <data-file> was lost"
            );
            return;
        }

        tracing::info!(
            recorder.name = self.name,
            recorder.records.len = records.len(),
            "Saved records to <data-file> successfully",
        );
    }

    fn deactivate(&mut self) {
        if self.session.is_some() {
            tracing::info!("{}: Deactivated", self.name);
        } else {
            tracing::warn!("{}: Deactivated, but inactive", self.name);
        }
        self.session = None;
    }

    fn create_live_stream_source(
        &self,
        record_id: Option<TimeshiftRecordId>,
    ) -> Result<TimeshiftLiveStreamSource, Error> {
        if self.points.len() < 2 {
            return Err(Error::RecordNotFound);
        }
        let name = self.name.clone();
        let file = self.config().ts_file.clone();
        let point = if let Some(id) = record_id {
            let record = self.records.get(&id).ok_or(Error::ProgramNotFound)?;
            record.start.clone()
        } else {
            self.points[0].clone()
        };
        Ok(TimeshiftLiveStreamSource { name, file, point })
    }

    fn create_record_stream_source(
        &self,
        record_id: TimeshiftRecordId,
        start_pos: Option<u64>,
    ) -> Result<TimeshiftRecordStreamSource, Error> {
        let record = self.records.get(&record_id).ok_or(Error::ProgramNotFound)?;
        record.create_record_stream_source(self.name.clone(), self.config(), start_pos)
    }

    fn handle_start_recording(&mut self) {
        tracing::info!("{}: Started recording", self.name);
        if let Some(point) = self.points.pop() {
            // remove the sentinel item if it exists
            tracing::debug!("{}: Removed the sentinel point {}", self.name, point);
        }
    }

    fn handle_stop_recording(&mut self, reset: bool) {
        tracing::info!("{}: Stopped recording", self.name);
        if reset {
            tracing::warn!("{}: Reset data", self.name);
            // TODO
        }
    }

    fn handle_chunk(&mut self, point: TimeshiftPoint) {
        self.maintain();
        self.append_point(&point);
        self.save_data();
    }

    fn maintain(&mut self) {
        if self.points.len() < self.config().max_chunks() {
            return;
        }
        self.invalidate_first_chunk();
        self.purge_expired_records();
        self.crop_record_to_be_overwritten();
    }

    fn invalidate_first_chunk(&mut self) {
        assert!(self.points.len() == self.config().max_chunks());
        let point = self.points.remove(0);
        let index = point.pos / (self.config().chunk_size as u64);
        tracing::debug!("{}: Chunk#{}: Invalidated", self.name, index);
    }

    // Purge records which ended recording before the first timestamp.
    //
    // A record which started recording before the timestamp and ended recording after the
    // timestamp it NOT purged.  Because it has data in available chunks and can be provided
    // via Web endpoints for streaming.
    fn purge_expired_records(&mut self) {
        assert!(!self.points.is_empty());
        let timestamp = self.points[0].timestamp;
        let n = self
            .records
            .values()
            .position(|record| record.end.timestamp > timestamp)
            .unwrap_or(self.records.len());
        for (_, record) in self.records.drain(0..n) {
            // remove first n records
            tracing::info!(
                "{}: {}: Purged: {}",
                self.name,
                record.id,
                record.program.name()
            );
        }
    }

    // Crop the beginning of a record to be overwritten.
    //
    // REMARKS
    // -------
    // The mtime and ctime of a file that corresponds to the cropped record exposed onto a
    // timeshift-fs are not changed.  That may cause issues in other applications.
    fn crop_record_to_be_overwritten(&mut self) {
        assert!(!self.points.is_empty());
        let start = self.points[0].clone();
        // Checking the first record is enough because old records have already been purged in
        // purge_expired_records().
        if let Some((_, record)) = self.records.first_mut() {
            if record.start.timestamp < start.timestamp {
                record.start = start;
                tracing::info!(
                    "{}: {}: Cropped: {}",
                    self.name,
                    record.id,
                    record.program.name()
                );
            }
        }
    }

    fn append_point(&mut self, point: &TimeshiftPoint) {
        let index = point.pos / (self.config().chunk_size as u64);
        assert!(point.pos % (self.config().chunk_size as u64) == 0);
        tracing::debug!(
            "{}: Chunk#{}: Timestamp: {}",
            self.name,
            index,
            point.timestamp
        );
        self.points.push(point.clone());
        assert!(self.points.len() <= self.config().max_chunks());
    }

    fn handle_event_start(&mut self, quad: ProgramQuad, event: EitEvent, point: TimeshiftPoint) {
        // Multiple records for the same TV program may be created when the timeshift recording
        // restarts.  Therefore, we use the recording start time instead of the start time in
        // the EPG data.
        let id = TimeshiftRecordId::from(point.timestamp.timestamp());
        let mut program = EpgProgram::new(quad);
        program.update(&event);
        tracing::info!(
            "{}: {}: Started: {}: {}",
            self.name,
            id,
            point,
            program.name()
        );
        self.records
            .insert(id, TimeshiftRecord::new(id, program, point));
    }

    fn handle_event_update(&mut self, quad: ProgramQuad, event: EitEvent, point: TimeshiftPoint) {
        let mut program = EpgProgram::new(quad);
        program.update(&event);
        self.update_last_record(program, point, false);
    }

    fn handle_event_end(&mut self, quad: ProgramQuad, event: EitEvent, point: TimeshiftPoint) {
        let mut program = EpgProgram::new(quad);
        program.update(&event);
        self.update_last_record(program, point, true);
    }

    fn update_last_record(&mut self, program: EpgProgram, point: TimeshiftPoint, end: bool) {
        match self.records.values_mut().last() {
            Some(record) => {
                record.update(program, point, end);
                if end {
                    tracing::debug!(
                        "{}: {}: Ended: {}: {}",
                        self.name,
                        record.id,
                        record.end,
                        record.program.name()
                    );
                } else {
                    tracing::debug!(
                        "{}: {}: Updated: {}: {}",
                        self.name,
                        record.id,
                        record.end,
                        record.program.name()
                    );
                }
            }
            None => {
                tracing::warn!("{}: No record to update", self.name);
            }
        }
    }

    fn get_model(&self) -> TimeshiftRecorderModel {
        let now = Jst::now();
        let start_time = if let Some(point) = self.points.iter().next() {
            point.timestamp.clone()
        } else {
            now.clone()
        };
        let end_time = if let Some(point) = self.points.iter().last() {
            point.timestamp.clone()
        } else {
            now.clone()
        };
        let (pipeline, recording) = match self.session {
            Some(ref session) => (session.pipeline.get_model(), true),
            _ => (vec![], false),
        };
        TimeshiftRecorderModel {
            index: self.index,
            name: self.name.clone(),
            service: self.service.clone(),
            start_time,
            end_time,
            pipeline,
            recording,
        }
    }

    async fn activate<T>(
        activation: TimeshiftActivation<T>,
        ctx: &mut Context<Self>,
    ) -> Result<TimeshiftActivationResult, Error>
    where
        T: Clone + Send + Sync + 'static,
        T: Call<StartStreaming>,
        T: Into<Emitter<StopStreaming>>,
    {
        let config = &activation.config.timeshift.recorders[&activation.name];
        let channel = &activation.service.channel;

        let user = TunerUser {
            info: TunerUserInfo::Recorder {
                name: format!("timeshift({})", activation.name),
            },
            priority: config.priority.into(),
        };

        let stream = activation
            .tuner_manager
            .call(StartStreaming {
                channel: channel.clone(),
                user,
                stream_id: None,
            })
            .await??;

        // stop_trigger must be created here in order to stop streaming when an error occurs.
        let stop_trigger =
            TunerStreamStopTrigger::new(stream.id(), activation.tuner_manager.clone().into());

        let data = mustache::MapBuilder::new()
            .insert_str("channel_name", &channel.name)
            .insert("channel_type", &channel.channel_type)?
            .insert_str("channel", &channel.channel)
            .insert("sid", &activation.service.sid)?
            .build();
        let mut builder = FilterPipelineBuilder::new(data);
        // NOTE
        // ----
        // We always decode stream before recording in order to make it easy to support seeking.
        // It's impossible to decode stream started from any position in the record.  Only streams
        // starting with PAT packets can be decoded.  This means that we need to seek a PAT
        // packet before streaming and we cannot start streaming from a specific position that
        // is specified by the media player using a HTTP Range header.
        if !stream.is_decoded() {
            builder.add_decode_filter(&activation.config.filters.decode_filter)?;
        }
        let (mut cmds, _) = builder.build();

        let data = mustache::MapBuilder::new()
            .insert("sid", &activation.service.sid)?
            .insert_str("file", &config.ts_file)
            .insert("chunk_size", &config.chunk_size)?
            .insert("num_chunks", &config.num_chunks)?
            .insert("start_pos", &activation.start_pos)?
            .build();
        let template = mustache::compile_str(&activation.config.timeshift.command)?;
        cmds.push(template.render_data_to_string(&data)?);

        let mut pipeline = spawn_pipeline(cmds, stream.id())?;

        let (input, output) = pipeline.take_endpoints()?;

        ctx.spawn_task(async move {
            let _ = stream.pipe(input).await;
        });

        let session = TimeshiftRecorderSession {
            pipeline,
            _stop_trigger: stop_trigger,
        };

        Ok(TimeshiftActivationResult { session, output })
    }
}

#[async_trait]
impl Actor for TimeshiftRecorder {
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("{}: Started", self.name);
        self.load_data();
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        self.deactivate();
        tracing::debug!("{}: Stopped", self.name);
    }
}

#[derive(Deserialize)]
pub struct TimeshiftRecorderData {
    pub service: EpgService,
    pub chunk_size: usize,
    pub max_chunks: usize,
    pub records: IndexMap<TimeshiftRecordId, TimeshiftRecord>,
    pub points: Vec<TimeshiftPoint>,
}

#[derive(Serialize)]
struct TimeshiftRecorderDataForSave<'a> {
    service: &'a EpgService,
    chunk_size: usize,
    max_chunks: usize,
    records: &'a IndexMap<TimeshiftRecordId, TimeshiftRecord>,
    points: &'a Vec<TimeshiftPoint>,
}

#[derive(Message)]
struct ServiceUpdated {
    service: EpgService,
}

#[async_trait]
impl Handler<ServiceUpdated> for TimeshiftRecorder {
    async fn handle(&mut self, msg: ServiceUpdated, _ctx: &mut Context<Self>) {
        self.service = msg.service;
    }
}

#[derive(Message)]
struct ActivateTimeshiftRecorder<T: Send> {
    service: EpgService,
    tuner_manager: T,
}

#[async_trait]
impl<T> Handler<ActivateTimeshiftRecorder<T>> for TimeshiftRecorder
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    async fn handle(&mut self, msg: ActivateTimeshiftRecorder<T>, ctx: &mut Context<Self>) {
        self.service = msg.service;
        if self.session.is_none() {
            tracing::info!("{}: Start activation", self.name);
            let activation = TimeshiftActivation {
                config: self.config.clone(),
                name: self.name.clone(),
                service: self.service.clone(),
                start_pos: self.points.last().map_or(0, |point| point.pos),
                tuner_manager: msg.tuner_manager.clone(),
            };
            match Self::activate(activation, ctx).await {
                Ok(result) => {
                    tracing::debug!("{}: Activation finished successfully", self.name);
                    self.session = Some(result.session);
                    let reader = BufReader::new(result.output);
                    let addr = ctx.address().clone();
                    let name = self.name.clone();
                    ctx.spawn_task(async move {
                        let mut lines = reader.lines();
                        loop {
                            match lines.next_line().await {
                                Ok(Some(json)) => {
                                    let msg = match serde_json::from_str::<TimeshiftRecorderMessage>(
                                        &json,
                                    ) {
                                        Ok(msg) => msg,
                                        Err(err) => {
                                            tracing::error!(
                                                "{}: Failed parsing a JSON message: {}",
                                                name,
                                                err
                                            );
                                            break;
                                        }
                                    };
                                    addr.emit(msg).await;
                                }
                                Ok(None) => {
                                    break;
                                }
                                Err(err) => {
                                    tracing::error!(
                                        "{}: Failed reading output from the command pipeline: {}",
                                        name,
                                        err
                                    );
                                    break;
                                }
                            }
                        }
                        addr.emit(TimeshiftRecorderMessage::Finish).await;
                    });
                }
                Err(err) => {
                    tracing::error!("{}: Activation failed: {}", self.name, err);
                }
            }
        } else {
            tracing::info!("{}: Already activated", self.name);
        }
    }
}

#[derive(Message)]
struct DeactivateTimeshiftRecorder;

#[async_trait]
impl Handler<DeactivateTimeshiftRecorder> for TimeshiftRecorder {
    async fn handle(&mut self, _msg: DeactivateTimeshiftRecorder, _ctx: &mut Context<Self>) {
        self.deactivate();
    }
}

#[async_trait]
impl Handler<QueryTimeshiftRecorder> for TimeshiftRecorder {
    async fn handle(
        &mut self,
        _msg: QueryTimeshiftRecorder,
        _ctx: &mut Context<Self>,
    ) -> <QueryTimeshiftRecorder as Message>::Reply {
        Ok(self.get_model())
    }
}

#[async_trait]
impl Handler<QueryTimeshiftRecords> for TimeshiftRecorder {
    async fn handle(
        &mut self,
        _msg: QueryTimeshiftRecords,
        _ctx: &mut Context<Self>,
    ) -> <QueryTimeshiftRecords as Message>::Reply {
        let records = self
            .records
            .values()
            .map(|record| record.get_model(self.config()))
            .collect();
        Ok(records)
    }
}

#[async_trait]
impl Handler<QueryTimeshiftRecord> for TimeshiftRecorder {
    async fn handle(
        &mut self,
        msg: QueryTimeshiftRecord,
        _ctx: &mut Context<Self>,
    ) -> <QueryTimeshiftRecord as Message>::Reply {
        let result = self
            .records
            .get(&msg.record_id)
            .map(|record| record.get_model(self.config()))
            .ok_or(Error::RecordNotFound);
        result
    }
}

#[async_trait]
impl Handler<CreateTimeshiftLiveStreamSource> for TimeshiftRecorder {
    async fn handle(
        &mut self,
        msg: CreateTimeshiftLiveStreamSource,
        _ctx: &mut Context<Self>,
    ) -> <CreateTimeshiftLiveStreamSource as Message>::Reply {
        self.create_live_stream_source(msg.record_id)
    }
}

#[async_trait]
impl Handler<CreateTimeshiftRecordStreamSource> for TimeshiftRecorder {
    async fn handle(
        &mut self,
        msg: CreateTimeshiftRecordStreamSource,
        _ctx: &mut Context<Self>,
    ) -> <CreateTimeshiftRecordStreamSource as Message>::Reply {
        self.create_record_stream_source(msg.record_id, msg.start_pos)
    }
}

#[async_trait]
impl Handler<TimeshiftRecorderMessage> for TimeshiftRecorder {
    async fn handle(&mut self, msg: TimeshiftRecorderMessage, _ctx: &mut Context<Self>) {
        match msg {
            TimeshiftRecorderMessage::Start => {
                self.handle_start_recording();
            }
            TimeshiftRecorderMessage::Stop(msg) => {
                self.handle_stop_recording(msg.reset);
            }
            TimeshiftRecorderMessage::Chunk(msg) => {
                self.handle_chunk(msg.chunk);
            }
            TimeshiftRecorderMessage::EventStart(msg) => {
                let quad = ProgramQuad::new(
                    msg.original_network_id,
                    msg.transport_stream_id,
                    msg.service_id,
                    msg.event.event_id,
                );
                self.handle_event_start(quad, msg.event, msg.record);
            }
            TimeshiftRecorderMessage::EventUpdate(msg) => {
                let quad = ProgramQuad::new(
                    msg.original_network_id,
                    msg.transport_stream_id,
                    msg.service_id,
                    msg.event.event_id,
                );
                self.handle_event_update(quad, msg.event, msg.record);
            }
            TimeshiftRecorderMessage::EventEnd(msg) => {
                let quad = ProgramQuad::new(
                    msg.original_network_id,
                    msg.transport_stream_id,
                    msg.service_id,
                    msg.event.event_id,
                );
                self.handle_event_end(quad, msg.event, msg.record);
            }
            TimeshiftRecorderMessage::Finish => {
                if self.session.is_some() {
                    tracing::warn!("{}: Recording pipeline broken, reactivate", self.name);
                    self.deactivate();
                    self.activator
                        .emit(ReactivateTimeshiftRecorder {
                            name: self.name.clone(),
                            service: self.service.clone(),
                        })
                        .await;
                }
            }
        }
    }
}

#[derive(Deserialize, Message)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type", content = "data")]
enum TimeshiftRecorderMessage {
    Start,
    Stop(TimeshiftRecorderStopMessage),
    Chunk(TimeshiftRecorderChunkMessage),
    EventStart(TimeshiftRecorderEventMessage),
    EventUpdate(TimeshiftRecorderEventMessage),
    EventEnd(TimeshiftRecorderEventMessage),
    Finish,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeshiftRecorderStartMessage;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeshiftRecorderStopMessage {
    reset: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeshiftRecorderChunkMessage {
    chunk: TimeshiftPoint,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeshiftRecorderEventMessage {
    original_network_id: Nid,
    transport_stream_id: Tsid,
    service_id: Sid,
    event: EitEvent,
    record: TimeshiftPoint,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TimeshiftPoint {
    #[serde(with = "ts_milliseconds")]
    pub timestamp: DateTime<Jst>,
    pub pos: u64,
}

impl fmt::Display for TimeshiftPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.timestamp, self.pos)
    }
}

struct TimeshiftRecorderSession {
    pipeline: CommandPipeline<TunerSubscriptionId>,
    _stop_trigger: TunerStreamStopTrigger,
}

struct TimeshiftActivation<T> {
    config: Arc<Config>,
    name: String,
    service: EpgService,
    start_pos: u64,
    tuner_manager: T,
}

struct TimeshiftActivationResult {
    session: TimeshiftRecorderSession,
    output: CommandPipelineOutput<TunerSubscriptionId>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TimeshiftRecord {
    pub id: TimeshiftRecordId,
    pub program: EpgProgram,
    pub start: TimeshiftPoint,
    pub end: TimeshiftPoint,
    #[serde(skip)]
    pub recording: bool,
}

impl TimeshiftRecord {
    fn new(id: TimeshiftRecordId, program: EpgProgram, point: TimeshiftPoint) -> Self {
        TimeshiftRecord {
            id,
            program,
            start: point.clone(),
            end: point.clone(),
            recording: true,
        }
    }

    fn update(&mut self, program: EpgProgram, point: TimeshiftPoint, end: bool) {
        self.program = program;
        self.end = point;
        if end {
            self.recording = false;
        }
    }

    pub fn create_record_stream_source(
        &self,
        recorder_name: String,
        config: &TimeshiftRecorderConfig,
        start_pos: Option<u64>,
    ) -> Result<TimeshiftRecordStreamSource, Error> {
        let file = config.ts_file.clone();
        let file_size = config.max_file_size();
        let id = self.id;
        let size = self.get_size(file_size);
        let (start, range) = if let Some(pos) = start_pos {
            (
                (self.start.pos + pos) % file_size,
                self.make_range(pos, size)?,
            )
        } else {
            (self.start.pos, self.make_range(0, size)?)
        };
        Ok(TimeshiftRecordStreamSource {
            recorder_name,
            file,
            id,
            start,
            range,
        })
    }

    fn get_model(&self, config: &TimeshiftRecorderConfig) -> TimeshiftRecordModel {
        TimeshiftRecordModel {
            id: self.id,
            program: self.program.clone(),
            start_time: self.start.timestamp.clone(),
            end_time: self.end.timestamp.clone(),
            size: self.get_size(config.max_file_size()),
            recording: self.recording,
        }
    }

    pub fn get_size(&self, file_size: u64) -> u64 {
        if self.end.pos < self.start.pos {
            file_size - self.start.pos + self.end.pos
        } else {
            self.end.pos - self.start.pos
        }
    }

    fn make_range(&self, first: u64, size: u64) -> Result<MpegTsStreamRange, Error> {
        if self.recording {
            MpegTsStreamRange::unbound(first, size)
        } else {
            MpegTsStreamRange::bound(first, size)
        }
    }
}

// 32 KiB, large enough for 10 ms buffering.
const CHUNK_SIZE: usize = 4096 * 8;

pub struct TimeshiftLiveStreamSource {
    name: String,
    file: String,
    point: TimeshiftPoint,
}

impl TimeshiftLiveStreamSource {
    pub async fn create_stream(
        self,
    ) -> Result<(TimeshiftLiveStream, TimeshiftStreamStopTrigger), Error> {
        tracing::debug!("{}: Start live streaming from {}", self.name, self.point);
        let (mut reader, stop_trigger) = TimeshiftFileReader::open(&self.file)
            .await?
            .with_stop_trigger();
        reader.set_position(self.point.pos).await?;
        let stream = ReaderStream::with_capacity(reader, CHUNK_SIZE);
        let id = format!("timeshift({})", self.name);
        Ok((MpegTsStream::new(id, stream).decoded(), stop_trigger))
    }

    #[cfg(test)]
    pub fn new_for_test(name: &str) -> Self {
        TimeshiftLiveStreamSource {
            name: name.to_string(),
            file: "/dev/zero".to_string(),
            point: TimeshiftPoint {
                timestamp: Jst::now(),
                pos: 0,
            },
        }
    }
}

pub struct TimeshiftRecordStreamSource {
    recorder_name: String,
    file: String,
    id: TimeshiftRecordId,
    start: u64,
    range: MpegTsStreamRange,
}

impl TimeshiftRecordStreamSource {
    pub async fn create_stream(
        self,
        seekable: bool,
    ) -> Result<(TimeshiftRecordStream, TimeshiftStreamStopTrigger), Error> {
        tracing::debug!(
            "{}: Start streaming {} bytes of {} from {}",
            self.recorder_name,
            self.range.bytes(),
            self.id,
            self.start
        );
        let (mut reader, stop_trigger) = TimeshiftFileReader::open(&self.file)
            .await?
            .with_stop_trigger();
        reader.set_position(self.start).await?;
        let stream = ReaderStream::with_capacity(reader.take(self.range.bytes()), CHUNK_SIZE);
        let id = format!("timeshift({})/{}", self.recorder_name, self.id);
        if seekable {
            Ok((
                MpegTsStream::with_range(id, stream, self.range).decoded(),
                stop_trigger,
            ))
        } else {
            Ok((MpegTsStream::new(id, stream).decoded(), stop_trigger))
        }
    }

    #[cfg(test)]
    pub fn new_for_test(recorder_name: &str) -> Self {
        TimeshiftRecordStreamSource {
            recorder_name: recorder_name.to_string(),
            file: "/dev/zero".to_string(),
            id: 1u32.into(),
            start: 0,
            range: MpegTsStreamRange::bound(0, 1).unwrap(),
        }
    }

    pub async fn read(&self, size: u32) -> Result<Vec<u8>, Error> {
        let mut reader = TimeshiftFileReader::open(&self.file).await?;
        reader.set_position(self.start).await?;
        let size = if (size as u64) < self.range.bytes() {
            size as usize
        } else {
            self.range.bytes() as usize
        };
        let mut data = Vec::with_capacity(size);
        let _ = reader.take(size as u64).read_to_end(&mut data).await?;
        Ok(data)
    }
}

pub struct TimeshiftFileReader {
    state: TimeshiftFileReaderState,
    path: String,
    file: File,
    stop_signal: Option<oneshot::Receiver<()>>,
}

enum TimeshiftFileReaderState {
    Read,
    Seek,
    Wait,
}

impl TimeshiftFileReader {
    pub async fn open(path: &str) -> Result<Self, Error> {
        let reader = TimeshiftFileReader {
            state: TimeshiftFileReaderState::Read,
            path: path.to_string(),
            file: File::open(path).await?,
            stop_signal: None,
        };
        Ok(reader)
    }

    fn with_stop_trigger(mut self) -> (Self, TimeshiftStreamStopTrigger) {
        let (tx, rx) = oneshot::channel();
        let stop_trigger = TimeshiftStreamStopTrigger(Some(tx));
        self.stop_signal = Some(rx);
        (self, stop_trigger)
    }

    async fn set_position(&mut self, pos: u64) -> Result<(), Error> {
        let _ = self.file.seek(SeekFrom::Start(pos)).await;
        Ok(())
    }
}

impl AsyncRead for TimeshiftFileReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;

        loop {
            if let Some(ref mut stop_signal) = self.stop_signal {
                if Pin::new(stop_signal).poll(cx).is_ready() {
                    tracing::debug!("{}: Stopped reading", self.path);
                    return Poll::Ready(Ok(()));
                }
            }
            match self.state {
                TimeshiftFileReaderState::Read => {
                    let len = buf.filled().len();
                    match Pin::new(&mut self.file).poll_read(cx, buf) {
                        Poll::Ready(Ok(_)) if buf.filled().len() == len => {
                            self.state = TimeshiftFileReaderState::Seek;
                            tracing::debug!("{}: EOF reached", self.path);
                        }
                        poll => {
                            return poll;
                        }
                    }
                }
                TimeshiftFileReaderState::Seek => {
                    match Pin::new(&mut self.file).start_seek(SeekFrom::Start(0)) {
                        Ok(_) => {
                            self.state = TimeshiftFileReaderState::Wait;
                            tracing::debug!("{}: Seek to the beginning", self.path);
                        }
                        Err(err) => {
                            return Poll::Ready(Err(err));
                        }
                    }
                }
                TimeshiftFileReaderState::Wait => {
                    match Pin::new(&mut self.file).poll_complete(cx) {
                        Poll::Ready(Ok(pos)) => {
                            assert!(pos == 0);
                            self.state = TimeshiftFileReaderState::Read;
                            tracing::debug!("{}: The seek completed, restart streaming", self.path);
                        }
                        Poll::Ready(Err(err)) => {
                            return Poll::Ready(Err(err));
                        }
                        Poll::Pending => {
                            return Poll::Pending;
                        }
                    }
                }
            }
        }
    }
}

pub struct TimeshiftStreamStopTrigger(Option<oneshot::Sender<()>>);

impl Drop for TimeshiftStreamStopTrigger {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

pub struct TimeshiftRecorderModel {
    pub index: usize,
    pub name: String,
    pub service: EpgService,
    pub start_time: DateTime<Jst>,
    pub end_time: DateTime<Jst>,
    pub pipeline: Vec<CommandPipelineProcessModel>,
    pub recording: bool,
}

pub struct TimeshiftRecordModel {
    pub id: TimeshiftRecordId,
    pub program: EpgProgram,
    pub start_time: DateTime<Jst>,
    pub end_time: DateTime<Jst>,
    pub size: u64,
    pub recording: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcaster::BroadcasterStream;
    use chrono::TimeZone;
    use tempfile::TempDir;
    use tokio::sync::watch;

    #[tokio::test]
    async fn test_timeshift_record_purge_expired_records() {
        let activator = ReactivatorStub;

        let temp_dir = TempDir::new().unwrap();
        let mut recorder = TimeshiftRecorder {
            index: 0,
            name: "test".to_string(),
            config: create_config(temp_dir.path()),
            activator: activator.clone().into(),
            service: create_epg_service(),
            records: indexmap::indexmap! {
                1u32.into() => TimeshiftRecord {
                    id: 1u32.into(),
                    program: EpgProgram::new((0, 0, 0, 1).into()),
                    start: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    end: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    recording: false,
                },
            },
            points: vec![TimeshiftPoint {
                timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
                pos: 0,
            }],
            session: None,
        };
        recorder.purge_expired_records();
        assert!(recorder.records.is_empty());

        let temp_dir = TempDir::new().unwrap();
        let mut recorder = TimeshiftRecorder {
            index: 0,
            name: "test".to_string(),
            config: create_config(temp_dir.path()),
            activator: activator.clone().into(),
            service: create_epg_service(),
            records: indexmap::indexmap! {
                1u32.into() => TimeshiftRecord {
                    id: 1u32.into(),
                    program: EpgProgram::new((0, 0, 0, 1).into()),
                    start: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    end: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    recording: false,
                },
                2u32.into() => TimeshiftRecord {
                    id: 2u32.into(),
                    program: EpgProgram::new((0, 0, 0, 2).into()),
                    start: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    end: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
                        pos: 0,
                    },
                    recording: false,
                },
                3u32.into() => TimeshiftRecord {
                    id: 3u32.into(),
                    program: EpgProgram::new((0, 0, 0, 3).into()),
                    start: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap(),
                        pos: 0,
                    },
                    end: TimeshiftPoint {
                        timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 2, 0).unwrap(),
                        pos: 0,
                    },
                    recording: false,
                },
            },
            points: vec![TimeshiftPoint {
                timestamp: Jst.with_ymd_and_hms(2021, 1, 1, 0, 1, 0).unwrap(),
                pos: 0,
            }],
            session: None,
        };
        recorder.purge_expired_records();
        assert_eq!(recorder.records.len(), 1);
        assert_eq!(recorder.records[0].program.quad, (0, 0, 0, 3).into());
    }

    #[tokio::test]
    async fn test_timeshift_recorder_broken_pipeline() {
        let (pipeline_tx, pipeline_rx) = watch::channel(false);
        let (reactivate_tx, mut reactivate_rx) = watch::channel(false);

        let system = System::new();
        let tuner_manager = TunerManagerStub(pipeline_rx);
        let activator = TimeshiftManagerStub(reactivate_tx).into();

        let temp_dir = TempDir::new().unwrap();
        let recorder = system
            .spawn_actor(TimeshiftRecorder::new(
                0,
                "test".to_string(),
                create_config(temp_dir.path()),
                activator,
            ))
            .await;

        recorder
            .emit(ActivateTimeshiftRecorder {
                service: create_epg_service(),
                tuner_manager: tuner_manager.clone(),
            })
            .await;

        let active = recorder.call(QueryTimeshiftRecorderState).await.unwrap();
        assert!(active);

        // emulate a broken pipeline
        pipeline_tx.send(true).unwrap();
        assert!(reactivate_rx.changed().await.is_ok());

        let active = recorder.call(QueryTimeshiftRecorderState).await.unwrap();
        assert!(!active);

        // reactivate
        recorder
            .emit(ActivateTimeshiftRecorder {
                service: create_epg_service(),
                tuner_manager: tuner_manager.clone(),
            })
            .await;

        let active = recorder.call(QueryTimeshiftRecorderState).await.unwrap();
        assert!(active);

        system.stop();
    }

    #[tokio::test]
    async fn test_timeshift_recorder_deactivate() {
        let (_pipeline_tx, pipeline_rx) = watch::channel(false);

        let system = System::new();
        let tuner_manager = TunerManagerStub(pipeline_rx);
        let activator = ReactivatorStub.into();

        let temp_dir = TempDir::new().unwrap();
        let recorder = system
            .spawn_actor(TimeshiftRecorder::new(
                0,
                "test".to_string(),
                create_config(temp_dir.path()),
                activator,
            ))
            .await;

        recorder
            .emit(ActivateTimeshiftRecorder {
                service: create_epg_service(),
                tuner_manager: tuner_manager.clone(),
            })
            .await;

        let active = recorder.call(QueryTimeshiftRecorderState).await.unwrap();
        assert!(active);

        recorder.emit(DeactivateTimeshiftRecorder).await;

        let active = recorder.call(QueryTimeshiftRecorderState).await.unwrap();
        assert!(!active);

        system.stop();
    }

    fn create_config<P: AsRef<Path>>(dir: P) -> Arc<Config> {
        let ts_file = dir.as_ref().join("ts-file.m2ts");
        let data_file = dir.as_ref().join("data-file.json");
        Arc::new(
            serde_yaml::from_str::<Config>(&format!(
                r#"
                timeshift:
                  command: true
                  recorders:
                    test:
                      service-triple: [1, 2, 3]
                      ts-file: {}
                      data-file: {}
                      num-chunks: 100
                "#,
                ts_file.display(),
                data_file.display()
            ))
            .unwrap(),
        )
    }

    fn create_epg_service() -> EpgService {
        EpgService {
            nid: 1.into(),
            tsid: 2.into(),
            sid: 3.into(),
            service_type: 1,
            logo_id: 0,
            remote_control_key_id: 0,
            name: "Service".to_string(),
            channel: EpgChannel {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "ch".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
            },
        }
    }

    #[derive(Clone)]
    struct TunerManagerStub(watch::Receiver<bool>);

    #[async_trait]
    impl Call<StartStreaming> for TunerManagerStub {
        async fn call(
            &self,
            _msg: StartStreaming,
        ) -> actlet::Result<<StartStreaming as Message>::Reply> {
            let (tx, stream) = BroadcasterStream::new_for_test();
            let mut rx = self.0.clone();
            tokio::spawn(async move {
                while rx.changed().await.is_ok() {
                    break;
                }
                drop(tx);
            });
            Ok(Ok(MpegTsStream::new(
                TunerSubscriptionId::default(),
                stream,
            )))
        }
    }

    #[async_trait]
    impl Emit<StopStreaming> for TunerManagerStub {
        async fn emit(&self, _msg: StopStreaming) {}
        fn fire(&self, _msg: StopStreaming) {}
    }

    impl Into<Emitter<StopStreaming>> for TunerManagerStub {
        fn into(self) -> Emitter<StopStreaming> {
            Emitter::new(self)
        }
    }

    #[derive(Clone)]
    struct ReactivatorStub;

    #[async_trait]
    impl Emit<ReactivateTimeshiftRecorder> for ReactivatorStub {
        async fn emit(&self, _msg: ReactivateTimeshiftRecorder) {}
    }

    impl Into<Emitter<ReactivateTimeshiftRecorder>> for ReactivatorStub {
        fn into(self) -> Emitter<ReactivateTimeshiftRecorder> {
            Emitter::new(self)
        }
    }

    struct TimeshiftManagerStub(watch::Sender<bool>);

    #[async_trait]
    impl Emit<ReactivateTimeshiftRecorder> for TimeshiftManagerStub {
        async fn emit(&self, _msg: ReactivateTimeshiftRecorder) {
            self.0.send(true).unwrap();
        }
    }

    impl Into<Emitter<ReactivateTimeshiftRecorder>> for TimeshiftManagerStub {
        fn into(self) -> Emitter<ReactivateTimeshiftRecorder> {
            Emitter::new(self)
        }
    }

    #[derive(Message)]
    #[reply("bool")]
    struct QueryTimeshiftRecorderState;

    #[async_trait]
    impl Handler<QueryTimeshiftRecorderState> for TimeshiftRecorder {
        async fn handle(
            &mut self,
            _msg: QueryTimeshiftRecorderState,
            _ctx: &mut Context<Self>,
        ) -> <QueryTimeshiftRecorderState as Message>::Reply {
            self.session.is_some()
        }
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    #[derive(Clone)]
    pub(crate) struct TimeshiftManagerStub;

    #[async_trait]
    impl Call<QueryTimeshiftRecorders> for TimeshiftManagerStub {
        async fn call(
            &self,
            _msg: QueryTimeshiftRecorders,
        ) -> actlet::Result<<QueryTimeshiftRecorders as Message>::Reply> {
            Ok(Ok(vec![]))
        }
    }

    #[async_trait]
    impl Call<QueryTimeshiftRecorder> for TimeshiftManagerStub {
        async fn call(
            &self,
            msg: QueryTimeshiftRecorder,
        ) -> actlet::Result<<QueryTimeshiftRecorder as Message>::Reply> {
            match msg.recorder {
                TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                    Ok(Ok(TimeshiftRecorderModel {
                        index: 0,
                        name: name.clone(),
                        service: EpgService {
                            nid: 1.into(),
                            tsid: 2.into(),
                            sid: 3.into(),
                            service_type: 1,
                            logo_id: 0,
                            remote_control_key_id: 0,
                            name: "test".to_string(),
                            channel: EpgChannel {
                                name: "test".to_string(),
                                channel_type: ChannelType::GR,
                                channel: "test".to_string(),
                                extra_args: "".to_string(),
                                services: Vec::new(),
                                excluded_services: Vec::new(),
                            },
                        },
                        start_time: Jst::now(),
                        end_time: Jst::now(),
                        pipeline: vec![],
                        recording: true,
                    }))
                }
                _ => Ok(Err(Error::RecordNotFound)),
            }
        }
    }

    #[async_trait]
    impl Call<QueryTimeshiftRecords> for TimeshiftManagerStub {
        async fn call(
            &self,
            _msg: QueryTimeshiftRecords,
        ) -> actlet::Result<<QueryTimeshiftRecords as Message>::Reply> {
            Ok(Ok(vec![]))
        }
    }

    #[async_trait]
    impl Call<QueryTimeshiftRecord> for TimeshiftManagerStub {
        async fn call(
            &self,
            msg: QueryTimeshiftRecord,
        ) -> actlet::Result<<QueryTimeshiftRecord as Message>::Reply> {
            if msg.record_id == 1u32.into() {
                Ok(Ok(TimeshiftRecordModel {
                    id: msg.record_id,
                    program: EpgProgram::new((0, 0, 0, 0).into()),
                    start_time: Jst::now(),
                    end_time: Jst::now(),
                    size: 0,
                    recording: true,
                }))
            } else {
                Ok(Err(Error::RecordNotFound))
            }
        }
    }

    #[async_trait]
    impl Call<CreateTimeshiftLiveStreamSource> for TimeshiftManagerStub {
        async fn call(
            &self,
            msg: CreateTimeshiftLiveStreamSource,
        ) -> actlet::Result<<CreateTimeshiftLiveStreamSource as Message>::Reply> {
            match msg.recorder {
                TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                    Ok(Ok(TimeshiftLiveStreamSource::new_for_test(name)))
                }
                _ => Ok(Err(Error::NoContent)),
            }
        }
    }

    #[async_trait]
    impl Call<CreateTimeshiftRecordStreamSource> for TimeshiftManagerStub {
        async fn call(
            &self,
            msg: CreateTimeshiftRecordStreamSource,
        ) -> actlet::Result<<CreateTimeshiftRecordStreamSource as Message>::Reply> {
            match msg.recorder {
                TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                    Ok(Ok(TimeshiftRecordStreamSource::new_for_test(name)))
                }
                _ => Ok(Err(Error::NoContent)),
            }
        }
    }
}
