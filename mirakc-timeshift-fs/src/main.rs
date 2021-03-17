use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::ffi::OsStr;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::Range;
use std::sync::Arc;

use clap;
use fuser;
use indexmap::IndexMap;
use sanitize_filename;
use serde_json;

use mirakc_core::*;
use mirakc_core::config::*;
use mirakc_core::error::Error;
use mirakc_core::models::*;
use mirakc_core::timeshift::*;
use mirakc_core::tracing_ext::init_tracing;

fn main() -> Result<(), Error> {
    let args = clap::App::new(clap::crate_name!())
        .version(clap::crate_version!())
        .about(clap::crate_description!())
        .arg(clap::Arg::with_name("config")
             .short("c")
             .long("config")
             .takes_value(true)
             .value_name("FILE")
             .env("MIRAKC_CONFIG")
             .help("Path to a configuration file in a YAML format")
             .long_help(
                 "Path to a configuration file in a YAML format.\n\
                  \n\
                  The MIRAKC_CONFIG environment variable is used if this \
                  option is not specified.  Its value has to be an absolute \
                  path.\n\
                  \n\
                  See README.md for details of the YAML format."))
        .arg(clap::Arg::with_name("log-format")
             .long("log-format")
             .value_name("FORMAT")
             .env("MIRAKC_LOG_FORMAT")
             .takes_value(true)
             .possible_values(&["text", "json"])
             .default_value("text")
             .help("Logging format"))
        .arg(clap::Arg::with_name("MOUNT_POINT")
            .index(1)
            .required(true)
            .help("Path to the mount point"))
        .get_matches();

    init_tracing(args.value_of("log-format").unwrap());

    let config_path = args.value_of("config").expect(
        "--config option or MIRAKC_CONFIG environment must be specified");

    let config = config::load(config_path);

    let fs = TimeshiftFilesystem::new(config);

    let options = vec![
        fuser::MountOption::FSName("mirakc-timeshift".to_string()),
        fuser::MountOption::RO,
        fuser::MountOption::NoAtime,
        fuser::MountOption::AllowOther,
        fuser::MountOption::AutoUnmount,  // not work properly, see comments below.
    ];

    let mount_point = args.value_of("MOUNT_POINT").unwrap();

    // The auto-unmount option does NOT work properly when the process terminates by signals like
    // SIGTERM.  Because the process terminates before fuser::Session<FS>::drop() is called.
    //
    // For avoiding the situation above, we can use a signal handler in mirakc-timeshift-fs.
    // However, we adopt a simpler solution like below:
    //
    // ```shell
    // # Register a signal handler before mounting the mirakc-timeshift filesystem
    // trap "umount /path/to/mount_point" SIGINT SIGQUIT SIGTERM
    //
    // # Then mount it
    // mirakc-timeshift-fs /path/to/mount_point
    // ```
    Ok(fuser::mount2(fs, mount_point, &options)?)
}

struct TimeshiftFilesystem {
    config: Arc<Config>,
    caches: HashMap<usize, TimeshiftFilesystemCache>,
    open_contexts: HashMap<u64, TimeshiftFilesystemOpenContext>,
    next_handle: u64,
}

impl TimeshiftFilesystem {
    const MAX_TITLE_SIZE: usize = 200;
    const TTL: std::time::Duration = std::time::Duration::from_secs(1);

    fn new(config: Arc<Config>) -> Self {
        TimeshiftFilesystem {
            config,
            caches: HashMap::new(),
            open_contexts: HashMap::new(),
            next_handle: 1,
        }
    }

    fn create_handle(&mut self, octx: TimeshiftFilesystemOpenContext) -> u64 {
        loop {
            let handle = self.next_handle;
            self.next_handle = if handle == u64::max_value() {
                1
            } else {
                handle + 1
            };
            if !self.open_contexts.contains_key(&handle) {
                self.open_contexts.insert(handle, octx);
                return handle;
            }
        }
    }

    fn system_time_from_unix_time(unix_time: i64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_time as u64)
    }

    fn lookup_recorder(&self, name: &OsStr) -> Option<TimeshiftFilesystemIno> {
        name.to_str()
            .and_then(|name| {
                self.config.timeshift.recorders.keys()
                    .position(|key| key == &name)
            })
            .map(TimeshiftFilesystemIno::create_recorder_ino)
    }

    fn make_recorder_attr(
        &self,
        ino: TimeshiftFilesystemIno,
    ) -> Option<fuser::FileAttr> {
        self.config
            .timeshift
            .recorders
            .get_index(ino.recorder_index())
            .map(|_| {
                let start_time = self.caches
                    .get(&ino.recorder_index())
                    .map(|cache| cache.records.values().next())
                    .flatten()
                    .map(|record| record.start.timestamp.timestamp())
                    .map(Self::system_time_from_unix_time)
                    .unwrap_or(std::time::UNIX_EPOCH);

                let end_time = self.caches
                    .get(&ino.recorder_index())
                    .map(|cache| cache.records.values().last())
                    .flatten()
                    .map(|record| record.end.timestamp.timestamp())
                    .map(Self::system_time_from_unix_time)
                    .unwrap_or(std::time::UNIX_EPOCH);

                fuser::FileAttr {
                    ino: ino.0,
                    size: 0,
                    blocks: 0,
                    atime: std::time::UNIX_EPOCH,
                    mtime: end_time.clone(),
                    ctime: end_time.clone(),
                    crtime: start_time.clone(),
                    kind: fuser::FileType::Directory,
                    perm: 0o555,
                    nlink: 2,
                    uid: 0,
                    gid: 0,
                    rdev: 0,
                    blksize: 512,
                    padding: 0,
                    flags: 0,
                }
            })
    }

    fn lookup_record(
        &self,
        ino: TimeshiftFilesystemIno,
        name: &OsStr,
    ) -> Option<TimeshiftFilesystemIno> {
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
        //     000000006049B5AB_ごごナマ..[字].m2ts
        //
        //   LOOKUP name:
        //     000000006049B5AB_こ\u{3099}こ\u{3099}ナマ..[字].m2ts
        //
        // The normalization form applied to the filename depends on the implementation of
        // each application.  For example, VLC applies NFD before opening a file.  On the other
        // hand, `cat` on macOS doesn't change the normalization form.  Therefore, the following
        // command works properly even if this function searches for a record that matches `name`
        // exactly:
        //
        //   # `cat` seems not to change the filename
        //   cat 000000006049B5AB_ごごナマ..[字].m2ts | ffplay -
        //
        // Conversion between String and OsString may not be idempotent.  Therefore, normalizing
        // before comparison may not work in general.
        //
        // We first extract the record ID encoded in `name`, and then look for a record identified
        // with it.
        name.to_string_lossy()  // may change <title>, but keeps <id> and the separator.
            .split('_')  // <id>_<title>.m2ts
            .next()  // <id>
            .and_then(|s| i64::from_str_radix(s, 16).ok())
            .map(TimeshiftRecordId::from)
            .map(|record_id| TimeshiftFilesystemIno::create_record_ino(
                ino.recorder_index(), record_id))
            .filter(|&ino| self.get_record(ino).is_some())
    }

    fn get_recorder_config(
        &self,
        ino: TimeshiftFilesystemIno,
    ) -> Option<&TimeshiftRecorderConfig> {
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
            let ino = TimeshiftFilesystemIno::create_recorder_ino(index);
            let dirname = sanitize_filename::sanitize(name);  // truncates within 255 bytes
            entries.push((ino.0, fuser::FileType::Directory, dirname));
        }
        let octx = TimeshiftFilesystemOpenContext::Dir(entries);
        self.create_handle(octx)
    }

    fn open_recorder_dir(&mut self, ino: TimeshiftFilesystemIno) -> u64 {
        let records = self.caches
            .get(&ino.recorder_index())
            .map(|cache| cache.records.clone())
            .unwrap_or(Default::default());
        let mut entries = vec![
            (ino.0, fuser::FileType::Directory, ".".to_string()),
            (1, fuser::FileType::Directory, "..".to_string()),
        ];
        for record in records.values() {
            let ino = TimeshiftFilesystemIno::create_record_ino(
                ino.recorder_index(), record.id);
            let title = record.program.name.clone()
                .map(|s| Self::truncate_string_within(s, Self::MAX_TITLE_SIZE))
                .unwrap_or("".to_string());
            let filename = sanitize_filename::sanitize(
                format!("{:016X}_{}.m2ts", record.id.value(), title));
            debug_assert!(filename.ends_with(".m2ts"));
            entries.push((ino.0, fuser::FileType::RegularFile, filename));
        }
        let octx = TimeshiftFilesystemOpenContext::Dir(entries);
        self.create_handle(octx)
    }

    fn get_record(&self, ino: TimeshiftFilesystemIno) -> Option<&TimeshiftRecord> {
        self.caches
            .get(&ino.recorder_index())
            .and_then(|cache| cache.records.get(&ino.record_id()))
    }

    fn make_record_attr(
        &self,
        ino: TimeshiftFilesystemIno,
    ) -> Option<fuser::FileAttr> {
        self.get_record(ino)
            .map(|record| {
                let start_time = Self::system_time_from_unix_time(
                    record.start.timestamp.timestamp());
                let end_time = Self::system_time_from_unix_time(
                    record.end.timestamp.timestamp());
                let file_size = self.config.timeshift.recorders
                    .get_index(ino.recorder_index())
                    .unwrap()
                    .1
                    .max_file_size();
                let size = record.get_size(file_size);

                fuser::FileAttr {
                    ino: ino.0,
                    size,
                    blocks: (size + 511) / 512,
                    atime: std::time::UNIX_EPOCH,
                    mtime: end_time.clone(),
                    ctime: end_time.clone(),
                    crtime: start_time.clone(),
                    kind: fuser::FileType::RegularFile,
                    perm: 0o444,
                    nlink: 1,
                    uid: 0,
                    gid: 0,
                    rdev: 0,
                    blksize: 512,
                    padding: 0,
                    flags: 0,
                }
            })
    }

    fn update_cache(&mut self, ino: TimeshiftFilesystemIno) {
        let config = self.config
            .timeshift
            .recorders
            .values()
            .nth(ino.recorder_index())
            .unwrap();

        let data_mtime = std::fs::metadata(&config.data_file)
            .ok()
            .and_then(|metadata| metadata.modified().ok());

        let cache_mtime = self.caches
            .get(&ino.recorder_index())
            .map(|cache| cache.mtime);

        let mtime = match (data_mtime, cache_mtime) {
            (Some(data_mtime), Some(cache_mtime)) if data_mtime > cache_mtime => {
                data_mtime
            }
            (Some(data_mtime), None) => {
                data_mtime
            }
            _ => {
                return;
            }
        };

        let cache = Self::load_data(config)
            .map(|data| TimeshiftFilesystemCache {
                mtime,
                records: data.records,
            });

        match cache {
            Ok(cache) => {
                self.caches.insert(ino.recorder_index(), cache);
            }
            Err(err) => {
                log::error!("Failed to read timeshift data: {}", err);
            }
        }
    }

    fn load_data(config: &TimeshiftRecorderConfig) -> Result<TimeshiftRecorderData, Error> {
        let reader = std::io::BufReader::new(std::fs::File::open(&config.data_file)?);
        let data: TimeshiftRecorderData = serde_json::from_reader(reader)?;
        if data.service.triple() == config.service_triple.into() &&
            data.chunk_size == config.chunk_size &&
            data.max_chunks == config.max_chunks() {
                Ok(data)
            } else {
                Err(Error::NoContent)
            }
    }

    fn open_record(&mut self, ino: TimeshiftFilesystemIno) -> Result<u64, Error> {
        debug_assert!(ino.is_record());
        match self.get_record(ino).zip(self.get_recorder_config(ino)) {
            Some((_, config)) => {
                let file = File::open(&config.ts_file)?;
                let octx = TimeshiftFilesystemOpenContext::Record(file);
                Ok(self.create_handle(octx))
            }
            _ => Err(Error::RecordNotFound),
        }
    }

    fn calc_read_ranges(
        record: &TimeshiftRecord,
        config: &TimeshiftRecorderConfig,
        offset: i64,
        size: u32,
    ) -> (Option<Range<u64>>, Option<Range<u64>>) {
        assert!(offset >= 0);

        let file_size = config.max_file_size();
        let record_size = record.get_size(file_size);

        if record_size == 0 {
            return (None, None);
        }

        if (offset as u64) >= record_size {
            // out of range
            return (None, None);
        }

        let remaining = record_size - (offset as u64);
        let read_size = remaining.min(size as u64);

        let start = record.start.pos + (offset as u64);
        let end = start + read_size;

        if start >= file_size || end <= file_size {
            return (Some((start % file_size)..(end % file_size)), None);
        }

        debug_assert!(start < file_size);
        debug_assert!(end > file_size);
        return (Some(start..file_size), Some(0..(end % file_size)));
    }

    fn map_record_data(
        file: &File,
        range: &Range<u64>,
    ) -> Result<memmap::Mmap, Error> {
        debug_assert!(range.end - range.start <= usize::max_value() as u64);
        unsafe {
            Ok(memmap::MmapOptions::new()
               .offset(range.start)
               .len((range.end - range.start) as usize)
               .map(file)?)
        }
    }

    fn read_record_data(
        file: &mut File,
        first: &Range<u64>,
        second: &Range<u64>,
    ) -> Result<Vec<u8>, Error> {
        debug_assert!(first.end - first.start <= usize::max_value() as u64);
        let first_len = (first.end - first.start) as usize;
        debug_assert!(second.end - second.start <= usize::max_value() as u64);
        let second_len = (second.end - second.start) as usize;
        debug_assert!((first_len as u64) + (second_len as u64) <= usize::max_value() as u64);
        let mut data = Vec::with_capacity(first_len + second_len);
        file.seek(SeekFrom::Start(first.start))?;
        let _ = file.read_to_end(&mut data)?;
        debug_assert!(data.len() == first_len);
        file.seek(SeekFrom::Start(0))?;
        let _ = file.take(second_len as u64).read_to_end(&mut data)?;
        debug_assert!(data.len() == first_len + second_len);
        Ok(data)
    }

    fn truncate_string_within(mut s: String, size: usize) -> String {
        if s.is_empty() || s.len() <= size {
            return s;
        }

        debug_assert!(size > 0);
        let mut i = size - 1;
        while i > 0 {
            if s.is_char_boundary(i) {
                break;
            }
            i -= 1;
        }

        s.truncate(i);
        s
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
        let ino = TimeshiftFilesystemIno::from(parent);
        let found = if ino.is_root() {
            self.lookup_recorder(name)
                .and_then(|ino| {
                    self.update_cache(ino);
                    self.make_recorder_attr(ino)
                })
        } else if ino.is_recorder() {
            self.lookup_record(ino, name)
                .and_then(|ino| {
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

    fn getattr(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        reply: fuser::ReplyAttr,
    ) {
        let ino = TimeshiftFilesystemIno::from(ino);
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
                uid: 0,
                gid: 0,
                rdev: 0,
                flags: 0,
                blksize: 512,
                padding: 0,
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

    fn opendir(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        _flags: i32,
        reply: fuser::ReplyOpen
    ) {
        let ino = TimeshiftFilesystemIno::from(ino);
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
        reply: fuser::ReplyEmpty
    ) {
        let ino = TimeshiftFilesystemIno::from(ino);
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
        let ino = TimeshiftFilesystemIno::from(ino);
        match self.open_contexts.get(&fh) {
            Some(TimeshiftFilesystemOpenContext::Dir(entries)) => {
                debug_assert!(ino.is_root() || ino.is_recorder());
                for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
                    // `i + 1` means the index of the next entry.
                    if reply.add(entry.0, (i + 1) as i64, entry.1, &entry.2) {
                        break;
                    }
                }
                reply.ok();
            }
            _ => {
                log::error!("{}: Invalid handle {}", ino, fh);
                reply.error(libc::EBADF);
            }
        }
    }

    fn open(
        &mut self,
        _req: &fuser::Request,
        ino: u64,
        _flags: i32,
        reply: fuser::ReplyOpen,
    ) {
        let ino = TimeshiftFilesystemIno::from(ino);
        if ino.is_record() {
            self.update_cache(ino);
            match self.open_record(ino) {
                Ok(handle) => reply.opened(handle, 0),
                Err(_) => {
                    log::debug!("{}: Record not found", ino);
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
        let ino = TimeshiftFilesystemIno::from(ino);
        if ino.is_record() {
            match self.open_contexts.remove(&fh) {
                Some(_) => reply.ok(),
                None => {
                    log::error!("{}: Invalid handle {}", ino, fh);
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
        let ino = TimeshiftFilesystemIno::from(ino);
        if ino.is_record() {
            self.update_cache(ino);

            let (record, config) = match self.get_record(ino).zip(self.get_recorder_config(ino)) {
                Some(tuple) => tuple,
                None => {
                    log::error!("{}: Record not found", ino);
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            let ranges = Self::calc_read_ranges(record, config, offset, size);

            let file = match self.open_contexts.get_mut(&fh) {
                Some(TimeshiftFilesystemOpenContext::Record(file)) => file,
                _ => {
                    log::error!("{}: Invalid handle {}", ino, fh);
                    reply.error(libc::EBADF);
                    return;
                }
            };

            match ranges {
                (None, None) => {
                    // No content to read.
                    let data: &[u8] = &[];
                    reply.data(data);
                }
                (Some(range), None) => {
                    match Self::map_record_data(file, &range) {
                        Ok(mmap) => {
                            log::trace!("{}: Mapped data in {:?} successfully",
                                        ino, range);
                            reply.data(&mmap[..]);
                        }
                        Err(_) => {
                            log::error!("{}: Faild to map data", ino);
                            reply.error(libc::EIO);
                        }
                    }
                }
                (Some(first), Some(second)) => {
                    match Self::read_record_data(file, &first, &second) {
                        Ok(data) => {
                            log::trace!("{}: Read data in {:?} and {:?} successfully",
                                        ino, first, second);
                            reply.data(&data[..]);
                        }
                        Err(_) => {
                            log::error!("{}: Faild to read data", ino);
                            reply.error(libc::EIO);
                        }
                    }
                }
                _ => unreachable!(),
            }
        } else {
            unreachable!();
        }
    }
}

// Mapping of ino:
//
//   Root      1
//   Recorder  0x8000_0000_0000_0000 | recorder_index << 56
//   Record    0x8000_0000_0000_0000 | recorder_index << 56 | record_id
//
// Where:
//
//   recorder_index in 0..128
//   record_id in 0..0x0100_0000_0000_0000
//
// `recorder_index` and `record_id` never changes even if its content changes.

#[derive(Clone, Copy)]
struct TimeshiftFilesystemIno(u64);

impl TimeshiftFilesystemIno {
    fn create_recorder_ino(index: usize) -> Self {
        (0x8000_0000_0000_0000 | (index as u64) << 56).into()
    }

    fn create_record_ino(index: usize, record_id: TimeshiftRecordId) -> Self {
        let id_value = record_id.value() & 0x00FF_FFFF_FFFF_FFFF;
        (0x8000_0000_0000_0000 | ((index as u64) << 56) | (id_value as u64)).into()
    }

    fn is_root(&self) -> bool {
        self.0 == 1
    }

    fn is_recorder(&self) -> bool {
        (self.0 & 0x8000_0000_0000_0000) != 0 && (self.0 & 0x00FF_FFFF_FFFF_FFFF) == 0
    }

    fn is_record(&self) -> bool {
        (self.0 & 0x8000_0000_0000_0000) != 0 && (self.0 & 0x00FF_FFFF_FFFF_FFFF) != 0
    }

    fn recorder_index(&self) -> usize {
        ((self.0 & 0x7F00_0000_0000_0000) >> 56) as usize
    }

    fn record_id(&self) -> TimeshiftRecordId {
        ((self.0 as i64) & 0x00FF_FFFF_FFFF_FFFF).into()
    }
}

impl fmt::Display for TimeshiftFilesystemIno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ino#{:016X}", self.0)
    }
}

impl From<u64> for TimeshiftFilesystemIno {
    fn from(ino: u64) -> Self {
        TimeshiftFilesystemIno(ino)
    }
}

struct TimeshiftFilesystemCache {
    mtime: std::time::SystemTime,
    records: IndexMap<TimeshiftRecordId, TimeshiftRecord>,
}

enum TimeshiftFilesystemOpenContext {
    Dir(Vec<(u64, fuser::FileType, String)>),
    Record(File),
}
