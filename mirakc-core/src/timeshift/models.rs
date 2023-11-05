use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Duration;
use chrono_jst::serde::ts_milliseconds;
use chrono_jst::Jst;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::io::Take;
use tokio::sync::oneshot;
use tokio_util::io::ReaderStream;

use super::filereader::TimeshiftFileReader;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::TimeshiftRecorderConfig;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::error::Error;
use crate::models::TimeshiftRecordId;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamRange;

#[derive(Debug)]
pub struct TimeshiftRecorderModel {
    pub index: usize,
    pub name: String,
    pub service: EpgService,
    pub start_time: Option<DateTime<Jst>>,
    pub end_time: Option<DateTime<Jst>>,
    pub duration: Duration,
    pub num_records: usize,
    pub pipeline: Vec<CommandPipelineProcessModel>,
    pub recording: bool,
    pub current_record_id: Option<TimeshiftRecordId>,
}

pub struct TimeshiftRecordModel {
    pub id: TimeshiftRecordId,
    pub program: EpgProgram,
    pub start_time: DateTime<Jst>,
    pub end_time: DateTime<Jst>,
    pub size: u64,
    pub recording: bool,
}

// 32 KiB, large enough for 10 ms buffering.
const CHUNK_SIZE: usize = 4096 * 8;

type TimeshiftLiveStream = MpegTsStream<String, ReaderStream<TimeshiftFileReader>>;
type TimeshiftRecordStream = MpegTsStream<String, ReaderStream<Take<TimeshiftFileReader>>>;

pub struct TimeshiftLiveStreamSource {
    pub(super) name: String,
    pub(super) file: PathBuf,
    pub(super) point: TimeshiftPoint,
}

impl TimeshiftLiveStreamSource {
    pub async fn create_stream(
        self,
    ) -> Result<(TimeshiftLiveStream, TimeshiftStreamStopTrigger), Error> {
        tracing::debug!(recorder.name = self.name, point = %self.point, "Start live streaming");
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
            file: "/dev/zero".into(),
            point: TimeshiftPoint {
                timestamp: Jst::now(),
                pos: 0,
            },
        }
    }
}

pub struct TimeshiftRecordStreamSource {
    recorder_name: String,
    file: PathBuf,
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
            recorder.name = self.recorder_name,
            record.id = %self.id,
            range.start = self.start,
            range.bytes = self.range.bytes(),
            "Start streaming"
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
            file: "/dev/zero".into(),
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

pub struct TimeshiftStreamStopTrigger(Option<oneshot::Sender<()>>);

impl TimeshiftStreamStopTrigger {
    pub(super) fn new(inner: oneshot::Sender<()>) -> Self {
        TimeshiftStreamStopTrigger(Some(inner))
    }
}

impl Drop for TimeshiftStreamStopTrigger {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TimeshiftPoint {
    #[serde(with = "ts_milliseconds")]
    pub timestamp: DateTime<Jst>,
    pub pos: u64,
}

impl std::fmt::Display for TimeshiftPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.timestamp.to_rfc3339(), self.pos)
    }
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
    pub(super) fn new(id: TimeshiftRecordId, program: EpgProgram, point: TimeshiftPoint) -> Self {
        TimeshiftRecord {
            id,
            program,
            start: point.clone(),
            end: point.clone(),
            recording: true,
        }
    }

    pub(super) fn update(&mut self, program: EpgProgram, point: TimeshiftPoint, end: bool) {
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

    pub(super) fn get_model(&self, config: &TimeshiftRecorderConfig) -> TimeshiftRecordModel {
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

#[derive(Deserialize)]
pub struct TimeshiftRecorderData {
    pub service: EpgService,
    pub chunk_size: usize,
    pub max_chunks: usize,
    pub records: IndexMap<TimeshiftRecordId, TimeshiftRecord>,
    pub points: VecDeque<TimeshiftPoint>,
}
