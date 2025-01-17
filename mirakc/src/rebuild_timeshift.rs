use std::collections::HashMap;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use actlet::prelude::*;
use chrono::DateTime;
use clap::Args;
use mirakc_core::epg::SyncClock;
use mirakc_core::*;
use tempfile::TempDir;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;

const MIN_CHUNKS: u64 = 3;

/// Rebuild timeshift files.
///
/// This subcommand rebuilds the following files by using an **existing** TS file:
///
///   * `config.timeshift[<RECORDER>].ts-file`
///   * `config.timeshift[<RECORDER>].data-file`
///
/// Rebuilding is needed in the following situations:
///
///   * `config.timeshift[<RECORDER>].chunk-size` changes
///   * `config.timeshift[<RECORDER>].num-chunks` changes
///   * `config.timeshift[<RECORDER>].num-reserves` changes
///   * `config.timeshift[<RECORDER>].data-file` has been broken
///
/// Rebuilding copies TS packets in <TS_FILE>.  This will take time depending on the size of
/// <TS_FILE>.
#[derive(Args)]
#[clap(verbatim_doc_comment)]
pub struct CommandLine {
    /// Stop after the scan phase.
    #[arg(long)]
    scan_only: bool,

    /// Chunk size of <TS_FILE>.
    #[arg(long, default_value = "154009600")]
    chunk_size: u64,

    /// Path to `mirakc-arib`.
    #[arg(long, default_value = "mirakc-arib")]
    mirakc_arib: PathBuf,

    /// Target recorder name defined in config.yml.
    #[arg(long)]
    recorder: String,

    /// Path to a TS file used for rebuilding timeshift files for `<RECORDER>`.
    ///
    /// Don't specify the TS file specified in `config.timeshift[<RECORDER>].ts-file`.
    ///
    /// At least 3 chunks has to be contained in the TS file.
    ///
    /// Chunks in the TS file are divided into segments.  A segment consists of
    /// chronologically continuous chunks.  Then, a timeshift recording is
    /// performed for each segment.  While timeshift recording, a segment is
    /// used as a source of TS packets.
    ///
    /// Some of chunks will be dropped while rebuilding in the following reasons:
    ///
    ///   * The first chunk in the first segment or the last chunk in the last
    ///     segment may contain garbage
    ///
    ///   * The size of the last chunk in a segment is less than the chunk size
    ///     defined in config.yml
    ///
    /// In addition, some of bytes will be dropped due to TS packet resync.
    #[arg(verbatim_doc_comment)]
    ts_file: PathBuf,
}

pub async fn main(config: Arc<config::Config>, cl: &CommandLine) {
    let start_time = Instant::now();

    validate(&config, cl);

    std::env::set_var("MIRAKC_REBUILD_TIMESHIFT", "1");

    let segments = scan(cl);
    if segments.is_empty() {
        tracing::warn!("You can simply remove data-file and restart timeshift recording");
        return;
    }
    if cl.scan_only {
        return;
    }

    rebuild_all(&config, cl, segments).await;
    let elapsed = start_time.elapsed();
    tracing::info!(elapsed = %humantime::format_duration(elapsed), "Done");
}

fn validate(config: &config::Config, cl: &CommandLine) {
    if cl.chunk_size % 8192 != 0 {
        tracing::error!("<CHUNK_SIZE> must be a multiple of 8192");
        std::process::exit(1);
    }

    if !config.timeshift.recorders.contains_key(&cl.recorder) {
        tracing::error!("{} is not defined in config.yml", cl.recorder);
        std::process::exit(1);
    }

    let data_file = config
        .timeshift
        .recorders
        .get(&cl.recorder)
        .unwrap()
        .data_file
        .as_path();
    if data_file.exists() {
        fn backup_filepath(original: &Path, n: u32) -> PathBuf {
            PathBuf::from(format!("{}.backup{n}", original.display()))
        }
        let mut n = 0u32;
        let mut backup = backup_filepath(data_file, n);
        while backup.exists() {
            n += 1;
            backup = backup_filepath(data_file, n);
        }
        std::fs::rename(data_file, &backup).unwrap();
        tracing::info!(
            "{} exists, it's moved to {} for backup",
            data_file.display(),
            backup.display()
        );
    }

    if !cl.ts_file.exists() {
        tracing::error!("{} does not exist", cl.ts_file.display());
        std::process::exit(1);
    }

    let ts_file = config
        .timeshift
        .recorders
        .get(&cl.recorder)
        .unwrap()
        .ts_file
        .as_path();
    if ts_file.exists() {
        tracing::warn!("{} exists, its contents will be lost", ts_file.display());
    }

    if ts_file.is_file() && ts_file.canonicalize().unwrap() == cl.ts_file.canonicalize().unwrap() {
        tracing::error!(
            "<TS_FILE> must be different from `config.timeshift.recorders[<RECORDER>].ts-file`"
        );
        std::process::exit(1);
    }
}

#[tracing::instrument(level = "info", skip_all)]
fn scan(cl: &CommandLine) -> Vec<Segment> {
    let start_time = Instant::now();
    let segments = Scanner::default().scan(cl);
    let elapsed = start_time.elapsed();
    tracing::debug!(elapsed = %humantime::format_duration(elapsed));
    segments
}

async fn rebuild_all(config: &config::Config, cl: &CommandLine, segments: Vec<Segment>) {
    for segment in segments.into_iter() {
        rebuild(config, cl, segment).await;
    }
}

#[tracing::instrument(level = "info", skip_all, fields(?segment))]
async fn rebuild(config: &config::Config, cl: &CommandLine, segment: Segment) {
    tracing::info!("Rebuilding...");
    let start_time = Instant::now();
    let tempdir = TempDir::new().unwrap();
    let script = create_tuner_script(cl, tempdir.path(), &segment);
    let config = create_config(config, cl, &script);
    let ok = do_recording(config).await;
    if !ok {
        tracing::warn!("Canceled");
        std::process::exit(1);
    }
    let elapsed = start_time.elapsed();
    tracing::debug!(elapsed = %humantime::format_duration(elapsed));
}

fn create_tuner_script(cl: &CommandLine, tempdir: &Path, segment: &Segment) -> PathBuf {
    let script = tempdir.join("tuner.sh");
    let mut file = File::create(&script).unwrap();
    for range in segment.0.iter() {
        if range.start == range.end {
            continue;
        }
        let line = format!(
            "dd if={} bs={} skip={} count={}",
            cl.ts_file.display(),
            cl.chunk_size,
            range.start,
            range.end - range.start
        );
        writeln!(&mut file, "{line}").unwrap();
        tracing::debug!(script = %script.display(), line);
    }
    script
}

fn create_config(config: &config::Config, cl: &CommandLine, script: &Path) -> Arc<config::Config> {
    let mut config = config.clone();
    // Disable EPG cache.
    config.epg.cache_dir = None;
    // Use a virtual tuner which provides TS packets contained in the segment.
    config.tuners = vec![config::TunerConfig {
        name: cl.recorder.clone(),
        channel_types: vec![
            models::ChannelType::GR,
            models::ChannelType::BS,
            models::ChannelType::CS,
            models::ChannelType::SKY,
        ],
        command: format!("sh -x {}", script.display()),
        decoded: true,
        ..Default::default()
    }];
    // Disable jobs other than the scan-services.
    config.jobs = Default::default();
    config.jobs.sync_clocks.disabled = true;
    config.jobs.update_schedules.disabled = true;
    // Disable timeshift recorders other than the specified one.
    config
        .timeshift
        .recorders
        .retain(|name, _| name == cl.recorder.as_str());
    Arc::new(config)
}

async fn do_recording(config: Arc<config::Config>) -> bool {
    let system = System::new();

    let tuner_manager = system
        .spawn_actor(tuner::TunerManager::new(config.clone()))
        .await;

    let epg = system
        .spawn_actor(epg::Epg::new(config.clone(), tuner_manager.clone()))
        .await;

    let timeshift_manager = system
        .spawn_actor(timeshift::TimeshiftManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
        ))
        .await;

    let notify = Arc::new(tokio::sync::Notify::new());
    let observer = Observer(notify.clone());
    let msg = timeshift::RegisterEmitter(Emitter::new(observer));
    timeshift_manager.call(msg).await.unwrap();

    let mut sigint = signal(SignalKind::interrupt()).unwrap();
    let mut sigterm = signal(SignalKind::terminate()).unwrap();

    let ok = tokio::select! {
        _ = notify.notified() => true,
        _ = sigint.recv() => {
            tracing::info!("SIGINT received");
            false
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGINT received");
            false
        }
    };

    system.shutdown().await;

    ok
}

// models

#[derive(Default)]
struct Scanner {
    timestamp_cache: HashMap<u64, i64>,
}

impl Scanner {
    fn scan(&mut self, cl: &CommandLine) -> Vec<Segment> {
        let ts_file_size = cl.ts_file.metadata().unwrap().len();
        if ts_file_size == 0 {
            tracing::info!("No data contained in ts-file");
            return vec![];
        }

        let num_chunks = ts_file_size / cl.chunk_size;
        if num_chunks < MIN_CHUNKS {
            tracing::warn!("At least {MIN_CHUNKS} chunks are needed for the rebuild");
            return vec![];
        }

        tracing::info!("{num_chunks} chunks contained in ts-file");

        let frac = ts_file_size % cl.chunk_size;
        if frac != 0 {
            tracing::warn!("Last {frac} bytes are discarded");
        }

        // Find the start chunk index.
        //
        // ts_file is a ring buffer, and there is a ring boundary in the middle
        // if a wrap-around occurred.
        let last_chunk = num_chunks - 1;
        let first_timestamp = self.scan_timestamp(cl, 0);
        tracing::debug!(first_timestamp);
        let last_timestamp = self.scan_timestamp(cl, last_chunk);
        tracing::debug!(last_timestamp);
        let start_chunk = if first_timestamp < last_timestamp {
            tracing::debug!("No wrap-around in ts-file");
            0
        } else {
            tracing::debug!("Find the boundary of the ring buffer");
            let start_time = Instant::now();
            let start_chunk = self.find_start_chunk(cl, 0, last_chunk);
            let elapsed = start_time.elapsed();
            tracing::debug!(find_start_chunk.elapsed = %humantime::format_duration(elapsed));
            start_chunk
        };
        tracing::info!("ts-file starts from chunk#{start_chunk}");

        // Collect segments.
        // A segment consists of continuous chunks.
        let start_time = Instant::now();
        let segments = self.collect_segments(cl, start_chunk, num_chunks);
        let elapsed = start_time.elapsed();
        tracing::debug!(collect_segments.elapsed = %humantime::format_duration(elapsed));

        tracing::info!("Found {} segments in ts_file", segments.len());
        segments
    }

    fn find_start_chunk(&mut self, cl: &CommandLine, left: u64, right: u64) -> u64 {
        tracing::debug!(left, right);
        // binary search
        let delta = right - left;
        if delta < 2 {
            return right;
        }
        let mid = left + delta / 2;
        let left_timestamp = self.scan_timestamp(cl, left);
        tracing::debug!(left, left_timestamp);
        let mid_timestamp = self.scan_timestamp(cl, mid);
        tracing::debug!(mid, mid_timestamp);
        if left_timestamp < mid_timestamp {
            self.find_start_chunk(cl, mid, right)
        } else {
            self.find_start_chunk(cl, left, mid)
        }
    }

    fn collect_segments(
        &mut self,
        cl: &CommandLine,
        start_chunk: u64,
        num_chunks: u64,
    ) -> Vec<Segment> {
        const DISCONTINUITY_THRESHOLD_MS: i64 = 10 * 1000; // 10s

        let mut segments = vec![];
        let mut segment = Segment::default();
        let mut seg_start_chunk = start_chunk;

        for i in 0..(num_chunks - 1) {
            let chunk = (start_chunk + i) % num_chunks;
            let next_chunk = (start_chunk + i + 1) % num_chunks;

            if chunk > next_chunk {
                let range = seg_start_chunk..num_chunks;
                tracing::debug!("Wrap-around, add {:?} to the segment", range);
                segment.push(range);
                seg_start_chunk = 0;
            }

            let last_timestamp = self.scan_last_timestamp(cl, chunk);
            tracing::debug!(chunk, last_timestamp);
            let next_timestamp = self.scan_timestamp(cl, next_chunk);
            tracing::debug!(next_chunk, next_timestamp);
            let gap = next_timestamp - last_timestamp;
            tracing::debug!(gap, chunk, next_chunk);
            assert!(gap >= 0);
            if gap > DISCONTINUITY_THRESHOLD_MS {
                tracing::info!(
                    "Found a segment boundary between chunk#{chunk} and chunk#{next_chunk}"
                );
                self.check_sync_bytes(cl, next_chunk);
                segment.push(seg_start_chunk..next_chunk);
                if !segment.is_empty() {
                    segments.push(std::mem::take(&mut segment));
                }
                seg_start_chunk = next_chunk;
            }
        }

        let end_chunk = if start_chunk == 0 {
            num_chunks
        } else {
            start_chunk
        };
        segment.push(seg_start_chunk..end_chunk);
        if !segment.is_empty() {
            segments.push(segment);
        }

        Self::validate_segments(&segments, start_chunk, end_chunk, num_chunks, num_chunks);

        // The first chunk in the first segment or the last chunk in the last
        // segment may contain garbage.  Drop the both chunks for safety.
        let last_chunk = if start_chunk == 0 {
            num_chunks - 1
        } else {
            start_chunk - 1
        };
        tracing::info!("Remove chunk#{start_chunk} from segments, because it may contain garbage");
        tracing::info!("Remove chunk#{last_chunk} from segments, because it may contain garbage");
        if let Some(segment) = segments.first_mut() {
            let range = segment.0.first_mut().unwrap();
            assert!(range.start < num_chunks);
            assert_eq!(range.start, start_chunk);
            range.start += 1;
            segment.0.retain(|range| !range.is_empty());
        }
        if let Some(segment) = segments.last_mut() {
            let range = segment.0.last_mut().unwrap();
            assert!(range.end > 0);
            assert_eq!(range.end, end_chunk);
            range.end -= 1;
            segment.0.retain(|range| !range.is_empty());
        }
        segments.retain(|segment| !segment.is_empty());

        Self::validate_segments(
            &segments,
            (start_chunk + 1) % num_chunks,
            end_chunk - 1,
            num_chunks,
            num_chunks - 2,
        );

        segments
    }

    fn scan_timestamp(&mut self, cl: &CommandLine, chunk: u64) -> i64 {
        match self.timestamp_cache.get(&chunk) {
            Some(&timestamp) => timestamp,
            None => {
                tracing::debug!(chunk, "Scan timestamp");
                let mut ts_file = File::open(&cl.ts_file).unwrap();
                ts_file
                    .seek(SeekFrom::Start(cl.chunk_size * chunk))
                    .unwrap();
                let result = Command::new(&cl.mirakc_arib)
                    .arg("sync-clocks")
                    .stdin(ts_file)
                    .stderr(Stdio::null())
                    .output()
                    .unwrap();
                let clocks: Vec<SyncClock> = serde_json::from_slice(&result.stdout).unwrap();
                let timestamp = clocks.first().unwrap().clock.time;
                self.timestamp_cache.insert(chunk, timestamp);
                timestamp
            }
        }
    }

    fn scan_last_timestamp(&mut self, cl: &CommandLine, chunk: u64) -> i64 {
        tracing::debug!(chunk, "Scan last timestamp");

        const BLOCK_SIZE: u64 = 4096;
        const LAST_TIMESTAMP_SCAN_BLOCKS: u64 = 1000;

        let mut last_timestamp = None;
        let num_blocks = cl.chunk_size / BLOCK_SIZE;
        let mut scan_blocks = LAST_TIMESTAMP_SCAN_BLOCKS;
        let mut skip_blocks = (chunk + 1) * num_blocks - scan_blocks;
        let mut line = String::with_capacity(4096);
        loop {
            let mut dd = Command::new("dd")
                .arg(format!("if={}", cl.ts_file.display()))
                .arg(format!("bs={BLOCK_SIZE}"))
                .arg(format!("skip={skip_blocks}"))
                .arg(format!("count={scan_blocks}"))
                .stdin(File::open(&cl.ts_file).unwrap())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut print_pes = Command::new(&cl.mirakc_arib)
                .arg("print-pes")
                .stdin(dd.stdout.take().unwrap())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let output = print_pes.stdout.take().unwrap();
            let mut reader = BufReader::new(output);
            while let Ok(n) = reader.read_line(&mut line) {
                if n == 0 {
                    break;
                }
                let parts: Vec<&str> = line.split('|').collect();
                let date = parts.first().unwrap();
                let label = parts.get(2).unwrap();
                let date = date.trim();
                if date.is_empty() {
                    line.clear();
                    continue;
                }
                if !label.starts_with("PCR#") {
                    line.clear();
                    continue;
                }
                let jst = format!("{date}+09:00");
                let datetime = DateTime::parse_from_str(&jst, "%Y/%m/%d %H:%M:%S.%f%z").unwrap();
                let timestamp = datetime.timestamp_millis();
                last_timestamp = Some(timestamp);
                line.clear();
            }
            let _ = dd.wait();
            let _ = print_pes.wait();
            if let Some(timestamp) = last_timestamp {
                return timestamp;
            }
            scan_blocks += LAST_TIMESTAMP_SCAN_BLOCKS;
            if scan_blocks > num_blocks {
                panic!("Cannot scan last timestamp in chunk#{chunk}");
            }
            skip_blocks -= LAST_TIMESTAMP_SCAN_BLOCKS;
        }
        // never reach here
    }

    fn check_sync_bytes(&mut self, cl: &CommandLine, chunk: u64) -> bool {
        let mut ts_file = File::open(&cl.ts_file).unwrap();
        let offset = cl.chunk_size * chunk;
        ts_file.seek(SeekFrom::Start(offset)).unwrap();
        let mut buf = [0u8; 1];
        ts_file.read_exact(&mut buf).unwrap();
        if buf[0] != b'G' {
            tracing::warn!("chunk#{chunk} does not start with TS sync byte (1st packet)");
            return false;
        }
        ts_file.seek(SeekFrom::Start(offset + 188)).unwrap();
        ts_file.read_exact(&mut buf).unwrap();
        if buf[0] != b'G' {
            tracing::warn!("chunk#{chunk} does not start with TS sync byte (2nd packet)");
            return false;
        }
        ts_file.seek(SeekFrom::Start(offset + 188 * 2)).unwrap();
        ts_file.read_exact(&mut buf).unwrap();
        if buf[0] != b'G' {
            tracing::warn!("chunk#{chunk} does not start with TS sync byte (3rd packet)");
            return false;
        }
        true
    }

    fn validate_segments(
        segments: &[Segment],
        start_chunk: u64,
        end_chunk: u64,
        num_chunks: u64,
        total_chunks: u64,
    ) {
        tracing::debug!(?segments, start_chunk, end_chunk, num_chunks);
        let mut prev = start_chunk;
        let mut total = 0;
        for segment in segments.iter() {
            for range in segment.0.iter() {
                assert!(!range.is_empty());
                assert_eq!(range.start, prev % num_chunks);
                total += range.end - range.start;
                prev = range.end;
            }
        }
        assert_eq!(prev, end_chunk);
        assert_eq!(total, total_chunks);
    }
}

struct Observer(Arc<tokio::sync::Notify>);

#[async_trait]
impl Emit<timeshift::TimeshiftEvent> for Observer {
    async fn emit(&self, msg: timeshift::TimeshiftEvent) {
        use timeshift::TimeshiftEvent::*;
        if let Stopped { .. } = msg {
            self.0.notify_one();
        }
    }
}

#[derive(Debug, Default)]
struct Segment(Vec<Range<u64>>);

impl Segment {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(&mut self, range: Range<u64>) {
        if !range.is_empty() {
            self.0.push(range);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use tempfile::TempDir;
    use test_log::test;

    #[test]
    fn test_validate_data_file_exists() {
        let temp_dir = TempDir::new().unwrap();

        let ts_file = temp_dir.as_ref().join("test.timeshift.m2ts");
        std::fs::write(&ts_file, b"").unwrap();

        let data_file = temp_dir.as_ref().join("test.timeshift.json");
        std::fs::write(&data_file, b"0").unwrap();

        let config: config::Config = serde_yaml::from_str(&format!(
            r#"
            timeshift:
              recorders:
                test:
                  service-id: 1
                  ts-file: {}
                  data-file: {}
                  num-chunks: 10
                  uses:
                    tuner: tuner
                    channel-type: GR
                    channel: ch
            "#,
            ts_file.display(),
            data_file.display(),
        ))
        .unwrap();

        let ts_file = temp_dir.as_ref().join("new.timeshift.m2ts");
        std::fs::write(&ts_file, b"").unwrap();

        let cl = CommandLine {
            scan_only: true,
            chunk_size: 154009600,
            mirakc_arib: "true".into(),
            recorder: "test".to_string(),
            ts_file,
        };

        validate(&config, &cl);

        let backup_file = temp_dir.as_ref().join("test.timeshift.json.backup0");
        assert!(backup_file.exists());
        assert_matches!(std::fs::read_to_string(&backup_file), Ok(content) => {
            assert_eq!(content, "0");
        });

        let data_file = temp_dir.as_ref().join("test.timeshift.json");
        std::fs::write(&data_file, b"1").unwrap();

        validate(&config, &cl);

        let backup_file = temp_dir.as_ref().join("test.timeshift.json.backup0");
        assert!(backup_file.exists());
        assert_matches!(std::fs::read_to_string(&backup_file), Ok(content) => {
            assert_eq!(content, "0");
        });

        let backup_file = temp_dir.as_ref().join("test.timeshift.json.backup1");
        assert!(backup_file.exists());
        assert_matches!(std::fs::read_to_string(&backup_file), Ok(content) => {
            assert_eq!(content, "1");
        });
    }
}
