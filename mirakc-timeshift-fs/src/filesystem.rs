use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::Range;
use std::sync::Arc;

use indexmap::IndexMap;

use mirakc_core::config::*;
use mirakc_core::error::Error;
use mirakc_core::models::*;
use mirakc_core::timeshift::*;

const BLOCK_SIZE: u64 = 4096;

pub struct TimeshiftFilesystemConfig {
    pub uid: u32,
    pub gid: u32,
    pub start_time_prefix: bool,
}

pub struct TimeshiftFilesystem {
    config: Arc<Config>,
    fs_config: TimeshiftFilesystemConfig,
    caches: HashMap<usize, Cache>,
    open_contexts: HashMap<u64, OpenContext>,
    next_handle: u64,
}

impl TimeshiftFilesystem {
    const MAX_TITLE_SIZE: usize = 200;
    const MAX_FILENAME_SIZE: usize = 255;
    const TTL: std::time::Duration = std::time::Duration::from_secs(1);

    pub fn new(config: Arc<Config>, fs_config: TimeshiftFilesystemConfig) -> Self {
        tracing::info!(
            fs_config.uid,
            fs_config.gid,
            fs_config.start_time_prefix,
            "Create a timeshift filesystem"
        );
        TimeshiftFilesystem {
            config,
            fs_config,
            caches: HashMap::new(),
            open_contexts: HashMap::new(),
            next_handle: 1,
        }
    }

    fn create_handle(&mut self, octx: OpenContext) -> u64 {
        loop {
            let handle = self.next_handle;
            self.next_handle = if handle == u64::MAX { 1 } else { handle + 1 };
            if let Entry::Vacant(entry) = self.open_contexts.entry(handle) {
                entry.insert(octx);
                return handle;
            }
        }
    }

    fn lookup_recorder(&self, name: &OsStr) -> Option<Ino> {
        name.to_str()
            .and_then(|name| {
                self.config
                    .timeshift
                    .recorders
                    .keys()
                    .position(|key| key == name)
            })
            .map(Ino::create_recorder_ino)
    }

    fn make_recorder_attr(&self, ino: Ino) -> Option<fuser::FileAttr> {
        self.config
            .timeshift
            .recorders
            .get_index(ino.recorder_index())
            .map(|_| {
                let start_time = self
                    .caches
                    .get(&ino.recorder_index())
                    .and_then(|cache| cache.records.first())
                    .map(|(_, record)| record.start.timestamp.timestamp())
                    .map(system_time_from_unix_time)
                    .unwrap_or(std::time::UNIX_EPOCH);

                let end_time = self
                    .caches
                    .get(&ino.recorder_index())
                    .and_then(|cache| cache.records.last())
                    .map(|(_, record)| record.end.timestamp.timestamp())
                    .map(system_time_from_unix_time)
                    .unwrap_or(std::time::UNIX_EPOCH);

                fuser::FileAttr {
                    ino: ino.0,
                    size: 0,
                    blocks: 0,
                    atime: std::time::UNIX_EPOCH,
                    mtime: end_time,
                    ctime: end_time,
                    crtime: start_time,
                    kind: fuser::FileType::Directory,
                    perm: 0o555,
                    nlink: 2,
                    uid: self.fs_config.uid,
                    gid: self.fs_config.gid,
                    rdev: 0,
                    blksize: BLOCK_SIZE as u32,
                    flags: 0,
                }
            })
    }

    fn lookup_record(&self, ino: Ino, name: &OsStr) -> Option<Ino> {
        // to_string_lossy() may change <title>, but keeps <id> and the separator.
        let name = name.to_string_lossy();

        // Some applications open a file with another extension like ".srt".
        if !name.ends_with(".m2ts") {
            return None;
        }

        // [<datetime>.]<id>.<title>.m2ts
        let id_index = if self.fs_config.start_time_prefix {
            1
        } else {
            0
        };

        // DIRTY HACK
        // ----------
        // We don't compare `name` with a filename created from a record.
        //
        // On macOS, that doesn't work due to issues regarding the Unicode normalization.  HFS+
        // enforces all filenames to be valid UTF-16 in a 'variant' of NFD (Unicode Normalization
        // Form D).  If we make a filename using EpgProgram::name without normalization, the
        // following filename may be specified in `name`:
        //
        //   filename made in open_recorder_dir():
        //     6049B5AB.ごごナマ..[字].m2ts
        //
        //   LOOKUP name:
        //     6049B5AB.こ\u{3099}こ\u{3099}ナマ..[字].m2ts
        //
        // The normalization form applied to the filename depends on the implementation of
        // each application.  For example, VLC applies NFD before opening a file.  On the other
        // hand, `cat` on macOS doesn't change the normalization form.  Therefore, the following
        // command works properly even if this function searches for a record that matches `name`
        // exactly:
        //
        //   # `cat` seems not to change the filename
        //   cat 6049B5AB.ごごナマ..[字].m2ts | ffplay -
        //
        // Conversion between String and OsString may not be idempotent.  Therefore, normalizing
        // before comparison may not work in general.
        //
        // We first extract the record ID encoded in `name`, and then look for a record identified
        // with it.
        name.split('.')
            .nth(id_index)
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .map(TimeshiftRecordId::from)
            .map(|record_id| Ino::create_record_ino(ino.recorder_index(), record_id))
            .filter(|&ino| self.get_record(ino).is_some())
    }

    fn get_recorder_config(&self, ino: Ino) -> Option<&TimeshiftRecorderConfig> {
        self.config
            .timeshift
            .recorders
            .values()
            .nth(ino.recorder_index())
    }

    fn open_root_dir(&mut self) -> u64 {
        let mut entries = vec![
            (1, fuser::FileType::Directory, ".".to_string()),
            (1, fuser::FileType::Directory, "..".to_string()),
        ];
        for (index, name) in self.config.timeshift.recorders.keys().enumerate() {
            let ino = Ino::create_recorder_ino(index);
            let dirname = sanitize_filename::sanitize(name); // truncates within 255 bytes
            entries.push((ino.0, fuser::FileType::Directory, dirname));
        }
        let octx = OpenContext::Dir(entries);
        self.create_handle(octx)
    }

    fn open_recorder_dir(&mut self, ino: Ino) -> u64 {
        let records = self
            .caches
            .get(&ino.recorder_index())
            .map(|cache| cache.records.clone())
            .unwrap_or_default();
        let mut entries = vec![
            (ino.0, fuser::FileType::Directory, ".".to_string()),
            (1, fuser::FileType::Directory, "..".to_string()),
        ];
        for record in records.values() {
            let ino = Ino::create_record_ino(ino.recorder_index(), record.id);
            let title = record
                .program
                .name
                .clone()
                .map(|s| truncate_string_within(s, Self::MAX_TITLE_SIZE))
                .unwrap_or("no-title".to_string());
            let id = record.id.value();
            let filename = if self.fs_config.start_time_prefix {
                let datetime = record
                    .start
                    .timestamp
                    .naive_local()
                    .format("%Y-%m-%d-%H-%M-%S");
                sanitize_filename::sanitize(format!("{datetime}.{id:08X}.{title}.m2ts"))
            } else {
                sanitize_filename::sanitize(format!("{id:08X}.{title}.m2ts"))
            };
            assert!(filename.len() <= Self::MAX_FILENAME_SIZE);
            assert!(filename.ends_with(".m2ts"));
            entries.push((ino.0, fuser::FileType::RegularFile, filename));
        }
        let octx = OpenContext::Dir(entries);
        self.create_handle(octx)
    }

    fn get_record(&self, ino: Ino) -> Option<&TimeshiftRecord> {
        self.caches
            .get(&ino.recorder_index())
            .and_then(|cache| cache.records.get(&ino.record_id()))
    }

    fn make_record_attr(&self, ino: Ino) -> Option<fuser::FileAttr> {
        self.get_record(ino).map(|record| {
            let start_time = system_time_from_unix_time(record.start.timestamp.timestamp());
            let end_time = system_time_from_unix_time(record.end.timestamp.timestamp());
            let file_size = self
                .config
                .timeshift
                .recorders
                .get_index(ino.recorder_index())
                .unwrap()
                .1
                .max_file_size();
            let size = record.get_size(file_size);

            fuser::FileAttr {
                ino: ino.0,
                size,
                blocks: (size + BLOCK_SIZE - 1) / BLOCK_SIZE,
                atime: std::time::UNIX_EPOCH,
                mtime: end_time,
                ctime: end_time,
                crtime: start_time,
                kind: fuser::FileType::RegularFile,
                perm: 0o444,
                nlink: 1,
                uid: self.fs_config.uid,
                gid: self.fs_config.gid,
                rdev: 0,
                blksize: BLOCK_SIZE as u32,
                flags: 0,
            }
        })
    }

    fn update_cache(&mut self, ino: Ino) {
        let config = self
            .config
            .timeshift
            .recorders
            .values()
            .nth(ino.recorder_index())
            .unwrap();

        let data_mtime = std::fs::metadata(&config.data_file)
            .ok()
            .and_then(|metadata| metadata.modified().ok());

        let cache_mtime = self
            .caches
            .get(&ino.recorder_index())
            .map(|cache| cache.mtime);

        let mtime = match (data_mtime, cache_mtime) {
            (Some(data_mtime), Some(cache_mtime)) if data_mtime > cache_mtime => data_mtime,
            (Some(data_mtime), None) => data_mtime,
            _ => {
                tracing::debug!(%ino, "Reuse cached timeshift data");
                return;
            }
        };

        tracing::debug!(%ino, "Load timeshift data");
        let cache = Self::load_data(config).map(|data| Cache {
            mtime,
            records: data.records,
        });

        match cache {
            Ok(cache) => {
                self.caches.insert(ino.recorder_index(), cache);
            }
            Err(err) => {
                tracing::error!(%err, %ino, "Failed to read timeshift data");
            }
        }
    }

    fn load_data(config: &TimeshiftRecorderConfig) -> Result<TimeshiftRecorderData, Error> {
        let file = File::open(&config.data_file)?;
        let reader = BufReader::new(file);
        let data: TimeshiftRecorderData = serde_json::from_reader(reader)?;
        if data.service.id == config.service_id
            && data.chunk_size == config.chunk_size
            && data.max_chunks == config.max_chunks()
        {
            Ok(data)
        } else {
            Err(Error::NoContent)
        }
    }

    fn open_record(&mut self, ino: Ino) -> Result<u64, Error> {
        debug_assert!(ino.is_record());
        match self.get_record(ino).zip(self.get_recorder_config(ino)) {
            Some((_, config)) => {
                let file = File::open(&config.ts_file)?;
                let buf = RecordBuffer::new(ino, file);
                let octx = OpenContext::Record(buf);
                Ok(self.create_handle(octx))
            }
            _ => Err(Error::RecordNotFound),
        }
    }
}

impl fuser::Filesystem for TimeshiftFilesystem {
    fn lookup(
        &mut self,
        _req: &fuser::Request,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let ino = Ino::from(parent);
        let found = if ino.is_root() {
            self.lookup_recorder(name).and_then(|ino| {
                self.update_cache(ino);
                self.make_recorder_attr(ino)
            })
        } else if ino.is_recorder() {
            self.lookup_record(ino, name).and_then(|ino| {
                self.update_cache(ino);
                self.make_record_attr(ino)
            })
        } else {
            unreachable!();
        };
        match found {
            Some(attr) => reply.entry(&Self::TTL, &attr, 0),
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &fuser::Request, ino: u64, reply: fuser::ReplyAttr) {
        let ino = Ino::from(ino);
        let found = if ino.is_root() {
            Some(fuser::FileAttr {
                ino: 1,
                size: 0,
                blocks: 0,
                atime: std::time::UNIX_EPOCH,
                mtime: std::time::UNIX_EPOCH,
                ctime: std::time::UNIX_EPOCH,
                crtime: std::time::UNIX_EPOCH,
                kind: fuser::FileType::Directory,
                perm: 0o555,
                nlink: 2,
                uid: self.fs_config.uid,
                gid: self.fs_config.gid,
                rdev: 0,
                flags: 0,
                blksize: BLOCK_SIZE as u32,
            })
        } else if ino.is_recorder() {
            self.update_cache(ino);
            self.make_recorder_attr(ino)
        } else if ino.is_record() {
            self.update_cache(ino);
            self.make_record_attr(ino)
        } else {
            unreachable!();
        };
        match found {
            Some(attr) => reply.attr(&Self::TTL, &attr),
            None => reply.error(libc::ENOENT),
        }
    }

    fn opendir(&mut self, _req: &fuser::Request, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        let ino = Ino::from(ino);
        if ino.is_root() {
            let handle = self.open_root_dir();
            reply.opened(handle, 0);
        } else if ino.is_recorder() {
            self.update_cache(ino);
            let handle = self.open_recorder_dir(ino);
            reply.opened(handle, 0);
        } else {
            unreachable!();
        }
    }

    fn releasedir(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        fh: u64,
        _flags: i32,
        reply: fuser::ReplyEmpty,
    ) {
        let ino = Ino::from(ino);
        if ino.is_root() || ino.is_recorder() {
            match self.open_contexts.remove(&fh) {
                Some(_) => reply.ok(),
                None => reply.error(libc::EBADF),
            }
        } else {
            unreachable!();
        }
    }

    fn readdir(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: fuser::ReplyDirectory,
    ) {
        let ino = Ino::from(ino);
        match self.open_contexts.get(&fh) {
            Some(OpenContext::Dir(entries)) => {
                debug_assert!(ino.is_root() || ino.is_recorder());
                for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                    // `i + 1` means the index of the next entry.
                    if reply.add(entry.0, (i + 1) as i64, entry.1, &entry.2) {
                        break;
                    }
                }
                reply.ok();
            }
            _ => {
                tracing::error!(%ino, fh, "Invalid handle");
                reply.error(libc::EBADF);
            }
        }
    }

    fn open(&mut self, _req: &fuser::Request, ino: u64, _flags: i32, reply: fuser::ReplyOpen) {
        let ino = Ino::from(ino);
        if ino.is_record() {
            self.update_cache(ino);
            match self.open_record(ino) {
                Ok(handle) => reply.opened(handle, 0),
                Err(_) => {
                    tracing::debug!(%ino, "Record not found");
                    reply.error(libc::ENOENT);
                }
            }
        } else {
            unreachable!();
        }
    }

    fn release(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        let ino = Ino::from(ino);
        if ino.is_record() {
            match self.open_contexts.remove(&fh) {
                Some(_) => reply.ok(),
                None => {
                    tracing::error!(%ino, fh, "Invalid handle");
                    reply.error(libc::EBADF);
                }
            }
        } else {
            unreachable!();
        }
    }

    fn read(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyData,
    ) {
        let ino = Ino::from(ino);
        if ino.is_record() {
            self.update_cache(ino);

            let (record, config) = match self.get_record(ino).zip(self.get_recorder_config(ino)) {
                Some(tuple) => tuple,
                None => {
                    tracing::error!(%ino, "Record not found");
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            let file_size = config.max_file_size();
            let record_size = record.get_size(file_size);
            let ranges = calc_read_ranges(file_size, record_size, record.start.pos, offset, size);

            let buf = match self.open_contexts.get_mut(&fh) {
                Some(OpenContext::Record(buf)) => buf,
                _ => {
                    tracing::error!(%ino, fh, "Invalid handle");
                    reply.error(libc::EBADF);
                    return;
                }
            };

            match buf.fill(ranges) {
                Ok(_) => reply.data(buf.data()),
                Err(err) => {
                    tracing::error!(%err, %ino, "Faild to read data");
                    reply.error(libc::EIO);
                }
            }

            buf.reset();
        } else {
            unreachable!();
        }
    }
}

// Mapping of ino:
//
//   Root      1
//   Recorder  0x0100_0000_0000_0000 | recorder_index << 32
//   Record    0x0200_0000_0000_0000 | recorder_index << 32 | record_id
//
// Where:
//
//   recorder_index in 0..256 (8-bit)
//   record_id in 0..0xFFFF_FFFF (32-bit)
//
// `recorder_index` and `record_id` never changes even if its content changes.
//
// The most significant octet is used as the type of Ino:
//
//   0x01: record directory
//   0x02: record file
//
// 40..56 bits are filled with 0 at this point.  They are reserved for future use.

#[derive(Clone, Copy)]
struct Ino(u64);

impl Ino {
    fn create_recorder_ino(index: usize) -> Self {
        (0x0100_0000_0000_0000 | (index as u64) << 32).into()
    }

    fn create_record_ino(index: usize, record_id: TimeshiftRecordId) -> Self {
        (0x0200_0000_0000_0000 | ((index as u64) << 32) | (record_id.value() as u64)).into()
    }

    fn is_root(&self) -> bool {
        self.0 == 1
    }

    fn is_recorder(&self) -> bool {
        (self.0 & 0xFF00_0000_0000_0000) == 0x0100_0000_0000_0000
    }

    fn is_record(&self) -> bool {
        (self.0 & 0xFF00_0000_0000_0000) == 0x0200_0000_0000_0000
    }

    fn recorder_index(&self) -> usize {
        ((self.0 & 0x0000_00FF_0000_0000) >> 32) as usize
    }

    fn record_id(&self) -> TimeshiftRecordId {
        ((self.0 & 0x0000_0000_FFFF_FFFF) as u32).into()
    }
}

impl fmt::Display for Ino {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016X}", self.0)
    }
}

impl From<u64> for Ino {
    fn from(ino: u64) -> Self {
        Ino(ino)
    }
}

struct Cache {
    mtime: std::time::SystemTime,
    records: IndexMap<TimeshiftRecordId, TimeshiftRecord>,
}

enum OpenContext {
    Dir(Vec<(u64, fuser::FileType, String)>),
    Record(RecordBuffer),
}

struct RecordBuffer {
    ino: Ino,
    file: File,
    buf: Vec<u8>,
}

impl RecordBuffer {
    const INITIAL_BUFSIZE: usize = 4096 * 16; // 16 pages = 64KiB

    fn new(ino: Ino, file: File) -> Self {
        RecordBuffer {
            ino,
            file,
            buf: Vec::with_capacity(Self::INITIAL_BUFSIZE),
        }
    }

    fn data(&self) -> &[u8] {
        &self.buf[..]
    }

    fn fill(&mut self, ranges: (Option<Range<u64>>, Option<Range<u64>>)) -> Result<(), Error> {
        debug_assert!(self.data().is_empty());
        let ino = self.ino;
        match ranges {
            (None, None) => {
                tracing::trace!(%ino, "EOF");
                Ok(())
            }
            (Some(range), None) => {
                tracing::trace!(%ino, ?range, "Read data");
                self.fill1(&range)
            }
            (Some(first), Some(second)) => {
                tracing::trace!(%ino, ?first, ?second, "Read data");
                self.fill2(&first, &second)
            }
            _ => unreachable!(),
        }
    }

    fn fill1(&mut self, range: &Range<u64>) -> Result<(), Error> {
        debug_assert!(self.buf.is_empty());
        debug_assert!(range.end - range.start <= usize::MAX as u64);
        let len = (range.end - range.start) as usize;
        self.buf.reserve(len);
        self.file.seek(SeekFrom::Start(range.start))?;
        let _ = (&mut self.file)
            .take(len as u64)
            .read_to_end(&mut self.buf)?;
        debug_assert!(self.buf.len() == len);
        Ok(())
    }

    fn fill2(&mut self, first: &Range<u64>, second: &Range<u64>) -> Result<(), Error> {
        debug_assert!(self.buf.is_empty());
        debug_assert!(first.end - first.start <= usize::MAX as u64);
        let first_len = (first.end - first.start) as usize;
        debug_assert!(second.end - second.start <= usize::MAX as u64);
        let second_len = (second.end - second.start) as usize;
        debug_assert!((first_len as u64) + (second_len as u64) <= usize::MAX as u64);
        self.buf.reserve(first_len + second_len);
        self.file.seek(SeekFrom::Start(first.start))?;
        let _ = (&mut self.file)
            .take(first_len as u64)
            .read_to_end(&mut self.buf)?;
        debug_assert!(self.buf.len() == first_len);
        self.file.seek(SeekFrom::Start(0))?;
        let _ = (&mut self.file)
            .take(second_len as u64)
            .read_to_end(&mut self.buf)?;
        debug_assert!(self.buf.len() == first_len + second_len);
        Ok(())
    }

    fn reset(&mut self) {
        self.buf.truncate(0);
    }
}

fn truncate_string_within(mut s: String, size: usize) -> String {
    if size == 0 {
        return String::new();
    }

    if s.is_empty() || s.len() <= size {
        return s;
    }

    debug_assert!(size > 0);
    let mut i = size;
    while i > 0 {
        if s.is_char_boundary(i) {
            break;
        }
        i -= 1;
    }

    s.truncate(i);
    s
}

fn calc_read_ranges(
    file_size: u64,
    record_size: u64,
    record_pos: u64,
    offset: i64,
    size: u32,
) -> (Option<Range<u64>>, Option<Range<u64>>) {
    assert!(offset >= 0);
    assert!(record_pos < file_size);

    if record_size == 0 {
        return (None, None);
    }

    if (offset as u64) >= record_size {
        // out of range
        return (None, None);
    }

    let remaining = record_size - (offset as u64);
    let read_size = remaining.min(size as u64);

    let start = (record_pos + (offset as u64)) % file_size;
    let end = (start + read_size) % file_size;

    if end == 0 {
        (Some(start..file_size), None)
    } else if start < end {
        (Some(start..end), None)
    } else {
        (Some(start..file_size), Some(0..end))
    }
}

fn system_time_from_unix_time(unix_time: i64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_time as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use test_log::test;

    #[test]
    fn test_record_buffer_fill() {
        let mut buf = RecordBuffer::new(Ino::from(0), File::open("/dev/zero").unwrap());
        assert_matches!(buf.fill((None, None)), Ok(_) => {
            assert!(buf.data().is_empty());
        });
        buf.reset();
        assert_matches!(buf.fill((Some(10..50), None)), Ok(_) => {
            assert_eq!(buf.data().len(), 40);
        });
        buf.reset();
        assert_matches!(buf.fill((Some(100..200), Some(0..10))), Ok(_) => {
            assert_eq!(buf.data().len(), 110);
        });
        buf.reset();
    }

    #[test]
    fn test_truncate_string_within() {
        assert_eq!(truncate_string_within("".to_string(), 10), "");
        assert_eq!(truncate_string_within("ab".to_string(), 0), "");
        assert_eq!(truncate_string_within("ab".to_string(), 1), "a");
        assert_eq!(truncate_string_within("ab".to_string(), 10), "ab");
        assert_eq!(truncate_string_within("あい".to_string(), 0), "");
        assert_eq!(truncate_string_within("あい".to_string(), 1), "");
        assert_eq!(truncate_string_within("あい".to_string(), 2), "");
        assert_eq!(truncate_string_within("あい".to_string(), 3), "あ");
        assert_eq!(truncate_string_within("あい".to_string(), 4), "あ");
        assert_eq!(truncate_string_within("あい".to_string(), 5), "あ");
        assert_eq!(truncate_string_within("あい".to_string(), 6), "あい");
        assert_eq!(truncate_string_within("あい".to_string(), 10), "あい");
    }

    #[test]
    fn test_calc_read_ranges() {
        assert_eq!(calc_read_ranges(100, 0, 10, 10, 10), (None, None));
        assert_eq!(calc_read_ranges(100, 1, 10, 10, 10), (None, None));
        assert_eq!(calc_read_ranges(100, 30, 10, 10, 10), (Some(20..30), None));
        assert_eq!(calc_read_ranges(100, 30, 10, 20, 20), (Some(30..40), None));
        assert_eq!(calc_read_ranges(100, 30, 10, 30, 10), (None, None));
        assert_eq!(calc_read_ranges(100, 30, 70, 20, 10), (Some(90..100), None));
        assert_eq!(calc_read_ranges(100, 30, 70, 20, 20), (Some(90..100), None));
        assert_eq!(calc_read_ranges(100, 30, 70, 30, 10), (None, None));
        assert_eq!(
            calc_read_ranges(100, 30, 80, 10, 20),
            (Some(90..100), Some(0..10))
        );
        assert_eq!(calc_read_ranges(100, 30, 80, 20, 20), (Some(0..10), None));
        assert_eq!(calc_read_ranges(100, 30, 80, 30, 10), (None, None));
    }

    #[test]
    fn test_system_time_from_unix_time() {
        assert_eq!(system_time_from_unix_time(0), std::time::UNIX_EPOCH);
    }
}
