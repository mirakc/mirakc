use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use indexmap::IndexMap;
use itertools::Itertools;
use serde::Deserialize;

use crate::models::*;

pub fn load<P: AsRef<Path>>(config_path: P) -> Arc<Config> {
    let config_path = config_path.as_ref();

    let reader = File::open(config_path).unwrap_or_else(|err| {
        panic!("Failed to open {:?}: {}", config_path, err);
    });
    let mut config: Config = serde_yaml::from_reader(reader).unwrap_or_else(|err| {
        panic!("Failed to parse {:?}: {}", config_path, err);
    });

    config = config.normalize();

    config.validate();

    config.last_modified = std::fs::metadata(config_path)
        .map(|metadata| metadata.modified().ok())
        .ok()
        .flatten();
    Arc::new(config)
}

// result

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct Config {
    #[serde(skip)]
    pub last_modified: Option<SystemTime>,
    #[serde(default)]
    pub epg: EpgConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
    #[serde(default)]
    pub tuners: Vec<TunerConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub pre_filters: HashMap<String, FilterConfig>,
    #[serde(default)]
    pub post_filters: HashMap<String, PostFilterConfig>,
    #[serde(default)]
    pub jobs: JobsConfig,
    #[serde(default)]
    pub recording: RecordingConfig,
    #[serde(default)]
    pub timeshift: TimeshiftConfig,
    #[serde(default)]
    pub events: EventsConfig,
    #[serde(default)]
    pub onair_program_trackers: HashMap<String, OnairProgramTrackerConfig>,
    #[serde(default)]
    pub resource: ResourceConfig,
}

impl Config {
    pub fn normalize(mut self) -> Self {
        self.channels = ChannelConfig::normalize(self.channels);
        self.jobs = JobsConfig::normalize(self.jobs);
        self
    }

    fn validate(&self) {
        self.epg.validate();
        self.server.validate();
        self.channels
            .iter()
            .enumerate()
            .for_each(|(i, config)| config.validate(i));
        // The channels[].name property should be a unique, but some scripts generating a
        // channels.yml use the same name in multiple channels.
        //
        // Allow duplicate names in a practical point of view.
        //
        // assert_eq!(self.channels.len(),
        //            self.channels.iter()
        //            .map(|config| &config.name)
        //            .unique()
        //            .count(),
        //            "config.channels: `name` must be a unique");
        self.tuners.iter().enumerate().for_each(|(i, config)| {
            config.validate(i);
            if let Some(ref name) = config.dedicated_for {
                assert!(
                    self.onair_program_trackers.contains_key(name),
                    "config.tuners: `dedicated-for` must hold an existing name"
                );
            }
        });
        assert_eq!(
            self.tuners.len(),
            self.tuners
                .iter()
                .map(|config| &config.name)
                .unique()
                .count(),
            "config.tuners: `name` must be a unique"
        );
        self.filters.validate();
        self.pre_filters
            .iter()
            .for_each(|(name, config)| config.validate("pre-filters", name));
        self.post_filters
            .iter()
            .for_each(|(name, config)| config.validate(name));
        self.jobs.validate();
        self.recording.validate();
        self.timeshift.validate();
        self.onair_program_trackers
            .iter()
            .for_each(|(name, config)| config.validate(name));
        self.resource.validate();
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct EpgConfig {
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

impl EpgConfig {
    fn validate(&self) {
        if let Some(cache_dir) = self.cache_dir.as_ref() {
            assert!(
                cache_dir.is_dir(),
                "config.epg: `cache_dir` must be a path to an existing directory"
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_addrs")]
    pub addrs: Vec<ServerAddr>,
    #[serde(default = "ServerConfig::default_stream_max_chunks")]
    pub stream_max_chunks: usize,
    #[serde(default = "ServerConfig::default_stream_chunk_size")]
    pub stream_chunk_size: usize,
    #[serde(default = "ServerConfig::default_stream_time_limit")]
    pub stream_time_limit: u64,
    #[serde(default, with = "humantime_serde")]
    pub program_stream_max_start_delay: Option<Duration>,
    #[serde(default)]
    pub mounts: IndexMap<String, MountConfig>, // keeps the insertion order
    #[serde(default)]
    pub folder_view_template_path: Option<PathBuf>,
}

impl ServerConfig {
    pub(crate) fn http_addrs(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        self.addrs
            .iter()
            .filter_map(|addr| match addr {
                ServerAddr::Http(addr) => addr.to_socket_addrs().ok(),
                _ => None,
            })
            .flatten()
    }

    pub(crate) fn uds_paths(&self) -> impl Iterator<Item = &Path> {
        self.addrs.iter().filter_map(|addr| match addr {
            ServerAddr::Unix(path) => Some(path.as_path()),
            _ => None,
        })
    }

    fn default_addrs() -> Vec<ServerAddr> {
        vec![Default::default()]
    }

    fn default_stream_max_chunks() -> usize {
        // TODO: The same value is defined in the broadcaster module.
        1000
    }

    fn default_stream_chunk_size() -> usize {
        // TODO: The same value is defined in the broadcaster module.
        4096 * 8
    }

    fn default_stream_time_limit() -> u64 {
        16 * 1000 // 16s
    }

    fn validate(&self) {
        const SERVER_STREAM_TIME_LIMIT_MIN: u64 = 15_000;

        assert!(
            self.stream_time_limit >= SERVER_STREAM_TIME_LIMIT_MIN,
            "config.server: `stream_time_limit` must be larger than or equal to {}",
            SERVER_STREAM_TIME_LIMIT_MIN
        );

        if let Some(max_start_delay) = self.program_stream_max_start_delay {
            assert!(
                max_start_delay < Duration::from_secs(24 * 3600),
                "config.server: `program-stream-max-start-delay` \
                 must not be less than 24h"
            );
        }

        self.addrs.iter().for_each(|addr| addr.validate());
        self.mounts
            .iter()
            .for_each(|(mp, config)| config.validate(mp));

        if let Some(ref path) = self.folder_view_template_path {
            assert!(
                path.is_file(),
                "config.server: `folder_view_template_path` must be a path to an existing file"
            );
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            addrs: Self::default_addrs(),
            stream_max_chunks: Self::default_stream_max_chunks(),
            stream_chunk_size: Self::default_stream_chunk_size(),
            stream_time_limit: Self::default_stream_time_limit(),
            program_stream_max_start_delay: None,
            mounts: Default::default(),
            folder_view_template_path: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ServerAddr {
    Http(String),
    Unix(PathBuf),
}

impl ServerAddr {
    fn validate(&self) {
        match self {
            Self::Http(addr) => assert!(
                addr.to_socket_addrs().is_ok(),
                "config.server.addrs.{}: invalid socket address",
                addr
            ),
            Self::Unix(_) => (),
        }
    }
}

impl Default for ServerAddr {
    fn default() -> Self {
        ServerAddr::Http("localhost:40772".to_string())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct MountConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub index: Option<PathBuf>,
    #[serde(default)]
    pub listing: bool,
}

impl MountConfig {
    fn validate(&self, mount_point: &str) {
        assert!(
            mount_point.starts_with("/"),
            "config.server.mounts[{}]: \
                 a mount point must starts with '/'",
            mount_point
        );
        assert!(
            self.path.exists(),
            "config.server.mounts[{}]: \
                 `path` must be a path to an existing entry",
            mount_point
        );
        if let Some(index) = self.index.as_ref() {
            let path = self.path.join(index);
            assert!(
                path.is_file(),
                "config.server.mounts[{}]: \
                 `index` must be an existing file if it exists",
                mount_point
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub services: Vec<ServiceId>,
    #[serde(default)]
    pub excluded_services: Vec<ServiceId>,
    #[serde(default)]
    pub disabled: bool,
}

impl ChannelConfig {
    fn normalize(channels: Vec<ChannelConfig>) -> Vec<ChannelConfig> {
        let mut normalized: Vec<ChannelConfig> = vec![];
        // Remove disabled channels and merge channels having the same `type`
        // and `channel`.
        for mut channel in channels.into_iter().filter(|ch| !ch.disabled) {
            match normalized.iter_mut().find(|ch| **ch == channel) {
                Some(ch) => {
                    if ch.extra_args != channel.extra_args {
                        tracing::warn!(
                            "Channels having the same `type` and `channel` \
                             should have the same `extra-args`"
                        );
                    }
                    if ch.services.is_empty() {
                        // Collect all services.
                    } else if channel.services.is_empty() {
                        // Collect all services.
                        ch.services = vec![];
                    } else {
                        ch.services.append(&mut channel.services);
                    }
                    ch.excluded_services.append(&mut channel.excluded_services);
                }
                None => normalized.push(channel),
            }
        }
        // Normalize
        for mut channel in normalized.iter_mut() {
            channel.services = channel.services.iter().sorted().unique().cloned().collect();
            channel.excluded_services = channel
                .excluded_services
                .iter()
                .sorted()
                .unique()
                .cloned()
                .collect();
        }
        normalized
    }

    fn validate(&self, index: usize) {
        debug_assert!(!self.disabled);
        assert!(
            !self.name.is_empty(),
            "config.channels[{}]: `name` must be a non-empty string",
            index
        );
        assert!(
            !self.channel.is_empty(),
            "config.channels[{}]: `channel` must be a non-empty string",
            index
        );
    }
}

impl PartialEq for ChannelConfig {
    fn eq(&self, other: &Self) -> bool {
        self.channel_type == other.channel_type && self.channel == other.channel
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct TunerConfig {
    pub name: String,
    #[serde(rename = "types")]
    pub channel_types: Vec<ChannelType>,
    pub command: String,
    #[serde(default = "TunerConfig::default_time_limit")]
    pub time_limit: u64,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub decoded: bool,
    #[serde(default)]
    pub dedicated_for: Option<String>,
}

impl TunerConfig {
    fn default_time_limit() -> u64 {
        30 * 1000 // 30s
    }

    fn validate(&self, index: usize) {
        if self.disabled {
            return;
        }
        assert!(
            !self.name.is_empty(),
            "config.tuners[{}]: `name` must be a non-empty string",
            index
        );
        assert!(
            !self.channel_types.is_empty(),
            "config.tuners[{}]: `types` must be a non-empty list",
            index
        );
        assert!(
            !self.command.is_empty(),
            "config.tuners[{}]: `command` must be a non-empty string",
            index
        );
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct FiltersConfig {
    #[serde(default)]
    pub tuner_filter: FilterConfig,
    #[serde(default = "FiltersConfig::default_service_filter")]
    pub service_filter: FilterConfig,
    #[serde(default = "FiltersConfig::default_program_filter")]
    pub program_filter: FilterConfig,
    #[serde(default)]
    pub decode_filter: FilterConfig,
}

impl FiltersConfig {
    fn default_service_filter() -> FilterConfig {
        FilterConfig {
            command: "mirakc-arib filter-service --sid={{{sid}}}".to_string(),
        }
    }

    fn default_program_filter() -> FilterConfig {
        // Historical Notes
        // ----------------
        // The --pre-streaming option is NOT used anymore due to the issue#30.
        // Without the --pre-streaming option, actix-web cannot detect the
        // client disconnection until trying to write data to the socket.  In
        // the worst case, the client disconnect is not detected for a few
        // seconds which depends on a transmission interval of EIT p/f packets.
        //
        // The --pre-streaming was added in order to avoid the issue#1313 in
        // actix/actix-web.  PAT TS packets will be sent before the program
        // starts when this option is specified.  See masnagam/rust-case-studies
        // for details about the issue#1313.
        FilterConfig {
            command: "mirakc-arib filter-program --sid={{{sid}}} --eid={{{eid}}} \
                      --clock-pid={{{clock_pid}}} --clock-pcr={{{clock_pcr}}} \
                      --clock-time={{{clock_time}}} --end-margin=2000\
                      {{#video_tags}} --video-tag={{{.}}}{{/video_tags}}\
                      {{#audio_tags}} --audio-tag={{{.}}}{{/audio_tags}}\
                      {{#wait_until}} --wait-until={{{.}}}{{/wait_until}}\
                      "
            .to_string(),
        }
    }

    fn validate(&self) {
        // self.tuner_filter.command may be an empty string.
        self.service_filter.validate("filters", "service-filter");
        self.program_filter.validate("filters", "program-filter");
        // self.decode_filter.command may be an empty string.
    }
}

impl Default for FiltersConfig {
    fn default() -> Self {
        FiltersConfig {
            tuner_filter: Default::default(),
            service_filter: Self::default_service_filter(),
            decode_filter: Default::default(),
            program_filter: Self::default_program_filter(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct FilterConfig {
    #[serde(default)]
    pub command: String,
}

impl FilterConfig {
    fn validate(&self, group: &str, name: &str) {
        assert!(
            !self.command.is_empty(),
            "config.{}.{}: `command` must be a non-empty string",
            group,
            name
        );
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PostFilterConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

impl PostFilterConfig {
    fn validate(&self, name: &str) {
        assert!(
            !self.command.is_empty(),
            "config.post-filters.{}: `command` must be a non-empty string",
            name
        );
        if let Some(content_type) = self.content_type.as_ref() {
            assert!(
                !content_type.is_empty(),
                "config.post-filters.{}: `content-type` must be a non-empty string",
                name
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct JobsConfig {
    #[serde(default = "JobsConfig::default_scan_services")]
    pub scan_services: JobConfig,
    #[serde(default = "JobsConfig::default_sync_clocks")]
    pub sync_clocks: JobConfig,
    #[serde(default = "JobsConfig::default_update_schedules")]
    pub update_schedules: JobConfig,
}

impl JobsConfig {
    fn default_scan_services() -> JobConfig {
        JobConfig {
            command: "mirakc-arib scan-services\
                      {{#sids}} --sids={{{.}}}{{/sids}}\
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}"
                .to_string(),
            schedule: "0 1 6,18 * * * *".to_string(),
            disabled: false,
        }
    }

    fn default_sync_clocks() -> JobConfig {
        JobConfig {
            command: "mirakc-arib sync-clocks\
                      {{#sids}} --sids={{{.}}}{{/sids}}\
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}"
                .to_string(),
            schedule: "0 11 6,18 * * * *".to_string(),
            disabled: false,
        }
    }

    fn default_update_schedules() -> JobConfig {
        JobConfig {
            command: "mirakc-arib collect-eits\
                      {{#sids}} --sids={{{.}}}{{/sids}}\
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}"
                .to_string(),
            schedule: "0 21 6,18 * * * *".to_string(),
            disabled: false,
        }
    }

    fn normalize(mut self) -> Self {
        if !self.scan_services.disabled {
            if self.scan_services.command.is_empty() {
                self.scan_services.command = Self::default_scan_services().command;
            }
            if self.scan_services.schedule.is_empty() {
                self.scan_services.schedule = Self::default_scan_services().schedule;
            }
        }
        if !self.sync_clocks.disabled {
            if self.sync_clocks.command.is_empty() {
                self.sync_clocks.command = Self::default_sync_clocks().command;
            }
            if self.sync_clocks.schedule.is_empty() {
                self.sync_clocks.schedule = Self::default_sync_clocks().schedule;
            }
        }
        if !self.update_schedules.disabled {
            if self.update_schedules.command.is_empty() {
                self.update_schedules.command = Self::default_update_schedules().command;
            }
            if self.update_schedules.schedule.is_empty() {
                self.update_schedules.schedule = Self::default_update_schedules().schedule;
            }
        }
        self
    }

    fn validate(&self) {
        self.scan_services.validate("scan_services");
        self.sync_clocks.validate("sync_clocks");
        self.update_schedules.validate("update_schedules");
    }
}

impl Default for JobsConfig {
    fn default() -> Self {
        JobsConfig {
            scan_services: Self::default_scan_services(),
            sync_clocks: Self::default_sync_clocks(),
            update_schedules: Self::default_update_schedules(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct JobConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub disabled: bool,
}

impl JobConfig {
    fn validate(&self, name: &str) {
        if !self.disabled {
            assert!(
                !self.command.is_empty(),
                "config.jobs.{}: `command` must be a non-empty string",
                name
            );
            assert!(
                cron::Schedule::from_str(&self.schedule).is_ok(),
                "config.jobs.{}: `schedule` is not valid",
                name
            );
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct RecordingConfig {
    pub basedir: Option<PathBuf>,
    #[serde(default = "RecordingConfig::default_track_airtime_command")]
    pub track_airtime_command: String,
}

impl RecordingConfig {
    pub fn is_enabled(&self) -> bool {
        self.basedir.is_some()
    }

    fn validate(&self) {
        if let Some(ref basedir) = self.basedir {
            assert!(
                basedir.is_absolute(),
                "config.recording: `basedir` must be an absolute path"
            );
            assert!(
                basedir.is_dir(),
                "config.recording: `basedir` must be a path to an existing directory"
            );
        }
        assert!(
            !self.track_airtime_command.is_empty(),
            "config.recording: `track-airtime-command` must be a non-empty string"
        );
    }

    fn default_track_airtime_command() -> String {
        "mirakc-arib track-airtime --sid={{{sid}}} --eid={{{eid}}}".to_string()
    }
}

impl Default for RecordingConfig {
    fn default() -> Self {
        RecordingConfig {
            basedir: None,
            track_airtime_command: Self::default_track_airtime_command(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct TimeshiftConfig {
    #[serde(default = "TimeshiftConfig::default_command")]
    pub command: String,
    #[serde(default)]
    pub recorders: IndexMap<String, TimeshiftRecorderConfig>,
}

impl TimeshiftConfig {
    pub fn is_enabled(&self) -> bool {
        !self.command.is_empty() && !self.recorders.is_empty()
    }

    fn default_command() -> String {
        "mirakc-arib record-service --sid={{{sid}}} --file={{{file}}} \
         --chunk-size={{{chunk_size}}} --num-chunks={{{num_chunks}}} \
         --start-pos={{{start_pos}}}"
            .to_string()
    }

    fn validate(&self) {
        assert!(
            !self.command.is_empty(),
            "config.timeshift: `command` must be a non-empty string"
        );
        self.recorders
            .iter()
            .for_each(|(name, config)| config.validate(name));
    }
}

impl Default for TimeshiftConfig {
    fn default() -> Self {
        TimeshiftConfig {
            command: TimeshiftConfig::default_command(),
            recorders: IndexMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct TimeshiftRecorderConfig {
    pub service_triple: (NetworkId, TransportStreamId, ServiceId),
    pub ts_file: String,
    pub data_file: String,
    #[serde(default = "TimeshiftRecorderConfig::default_chunk_size")]
    pub chunk_size: usize,
    pub num_chunks: usize,
    #[serde(default = "TimeshiftRecorderConfig::default_num_reserves")]
    pub num_reserves: usize,
    #[serde(default = "TimeshiftRecorderConfig::default_priority")]
    pub priority: i32,
}

impl TimeshiftRecorderConfig {
    const BUFSIZE: usize = 8192;
    const TS_PACKET_SIZE: usize = 188;

    pub fn max_file_size(&self) -> u64 {
        (self.chunk_size as u64) * (self.num_chunks as u64)
    }

    pub fn max_chunks(&self) -> usize {
        assert!(self.num_chunks > self.num_reserves);
        self.num_chunks - self.num_reserves
    }

    fn validate(&self, name: &str) {
        assert!(
            !self.ts_file.is_empty(),
            "config.timeshift.recorders.{}: `ts-file` must be a non-empty path",
            name
        );
        if let Some(parent) = Path::new(&self.ts_file).parent() {
            assert!(
                parent.is_dir(),
                "config.timeshift.recorders.{}: \
                     The parent directory of `ts-file` must exists",
                name
            );
        } else {
            unreachable!(
                "config.timeshift.recorders.{}: `ts-file` must be a path to a file",
                name
            );
        }
        assert!(
            !self.data_file.is_empty(),
            "config.timeshift.recorders.{}: `data-file` must be a non-empty path",
            name
        );
        if let Some(parent) = Path::new(&self.data_file).parent() {
            assert!(
                parent.is_dir(),
                "config.timeshift.recorders.{}: \
                     The parent directory of `data-file` must exists",
                name
            );
        } else {
            unreachable!(
                "config.timeshift.recorders.{}: `data-file` must be a path to a file",
                name
            );
        }
        // TODO
        // ----
        // We should save data in a binary format in a performance point of view.  However, the
        // following crates don't work properly (maybe, due to bugs...).
        //
        //   * bincode
        //   * rmp_serde
        //
        // We may support a binary format in the future if there is a crate that works well with
        // our data formats.
        assert!(
            self.data_file.ends_with(".json"),
            "config.timeshift.recorders.{}: `data-file` must be a JSON file",
            name
        );
        assert!(
            self.chunk_size > 0,
            "config.timeshift.recorders.{}: `chunk-size` must be larger than 0",
            name
        );
        assert!(
            self.chunk_size % (Self::BUFSIZE * Self::TS_PACKET_SIZE) == 0,
            "config.timeshift.recorders.{}: `chunk-size` must be a multiple of {}x{}",
            name,
            Self::BUFSIZE,
            Self::TS_PACKET_SIZE
        );
        assert!(
            self.num_chunks > 2,
            "config.timeshift.recorders.{}: `num-chunks` must be larger than 2",
            name
        );
        assert!(
            self.num_reserves > 0,
            "config.timeshift.recorders.{}: `num-reserves` must be larger than 0",
            name
        );
        assert!(
            self.num_chunks - self.num_reserves > 1,
            "config.timeshift.recorders.{}: Maximum number of available chunks \
                 (`num-chunks` - `num-reserves`) must be larger than 1",
            name
        );
    }

    fn default_chunk_size() -> usize {
        Self::BUFSIZE * Self::TS_PACKET_SIZE * 100
    }

    fn default_num_reserves() -> usize {
        1
    }

    fn default_priority() -> i32 {
        TunerUserPriority::MAX
    }
}

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct EventsConfig {
    #[serde(default)]
    pub concurrency: Concurrency,
    #[serde(default)]
    pub epg: EpgEventsConfig,
    #[serde(default)]
    pub recording: RecordingEventsConfig,
}

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct EpgEventsConfig {
    #[serde(default)]
    pub programs_updated: String,
}

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct RecordingEventsConfig {
    #[serde(default)]
    pub started: String,
    #[serde(default)]
    pub stopped: String,
    #[serde(default)]
    pub failed: String,
    #[serde(default)]
    pub rescheduled: String,
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(test, derive(Debug))]
pub enum Concurrency {
    Unlimited,
    Number(usize),
    NumCpus(f32),
}

impl Default for Concurrency {
    fn default() -> Self {
        Concurrency::Number(1)
    }
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub enum OnairProgramTrackerConfig {
    Local(Arc<LocalOnairProgramTrackerConfig>),
}

impl OnairProgramTrackerConfig {
    fn validate(&self, name: &str) {
        match self {
            Self::Local(config) => config.validate(name),
        }
    }
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct LocalOnairProgramTrackerConfig {
    pub channel_types: HashSet<ChannelType>,
    #[serde(default)]
    pub services: HashSet<MirakurunServiceId>,
    #[serde(default)]
    pub excluded_services: HashSet<MirakurunServiceId>,
    #[serde(default = "LocalOnairProgramTrackerConfig::default_command")]
    pub command: String,
}

impl LocalOnairProgramTrackerConfig {
    fn default_command() -> String {
        "mirakc-arib collect-eitpf --sids={{{sid}}}".to_string()
    }

    fn validate(&self, name: &str) {
        assert!(
            !self.channel_types.is_empty(),
            "config.onair-program-trackers[{name}]: \
             `channel-types` must be a non-empty list"
        );
        assert!(
            !self.command.is_empty(),
            "config.onair-program-trackers[{name}]: \
             `command` must be a non-empty string"
        );
    }
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(Debug))]
pub struct ResourceConfig {
    #[serde(default = "ResourceConfig::default_strings_yaml")]
    pub strings_yaml: String,
    #[serde(default)]
    #[serde(deserialize_with = "ResourceConfig::deserialize_logos")]
    pub logos: HashMap<ServiceTriple, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct LogoConfig {
    pub service_triple: (NetworkId, TransportStreamId, ServiceId),
    pub image: String,
}

impl ResourceConfig {
    fn default_strings_yaml() -> String {
        "/etc/mirakc/strings.yml".to_string()
    }

    fn validate(&self) {
        assert!(
            Path::new(&self.strings_yaml).is_file(),
            "config.resources: `strings-yaml` must be a path to an existing YAML file"
        );
        for (triple, image) in self.logos.iter() {
            assert!(
                Path::new(image).is_file(),
                "config.resources: `logos[({}, {}, {})]` must be a path to an existing \
                     file",
                triple.nid(),
                triple.tsid(),
                triple.sid()
            );
        }
    }

    fn deserialize_logos<'de, D>(
        deserializer: D,
    ) -> Result<HashMap<ServiceTriple, String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values: Vec<LogoConfig> = Vec::deserialize(deserializer)?;
        let mut logos = HashMap::new();
        for logo in values.into_iter() {
            logos.insert(logo.service_triple.into(), logo.image);
        }
        Ok(logos)
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        ResourceConfig {
            strings_yaml: Self::default_strings_yaml(),
            logos: Default::default(),
        }
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::indexmap;
    use maplit::hashmap;
    use maplit::hashset;

    #[test]
    fn test_config() {
        assert_eq!(
            serde_yaml::from_str::<Config>("{}").unwrap(),
            Default::default()
        );

        let result = serde_yaml::from_str::<Config>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validate() {
        let mut config = Config::default();
        config.resource.strings_yaml = "/bin/sh".to_string();
        config.validate();
    }

    #[test]
    fn test_config_validate_channel_names() {
        let config = serde_yaml::from_str::<Config>(
            r#"
            channels:
              - name: test
                type: GR
                channel: test
              - name: test
                type: GR
                channel: test1
            resource:
              strings-yaml: /bin/sh
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_config_validate_tuner_names() {
        let config = serde_yaml::from_str::<Config>(
            r#"
            tuners:
              - name: test
                types: [GR]
                command: test
              - name: test
                types: [GR]
                command: test
            resource:
              strings-yaml: /bin/sh
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_config_validate_tuner_dedicated_for() {
        let config = serde_yaml::from_str::<Config>(
            r#"
            tuners:
              - name: test
                types: [GR]
                dedicated-for: test
                command: test
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    fn test_epg_config() {
        assert_eq!(
            serde_yaml::from_str::<EpgConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<EpgConfig>(
                r#"
                cache-dir: /path/to/epg
            "#
            )
            .unwrap(),
            EpgConfig {
                cache_dir: Some("/path/to/epg".into()),
            }
        );

        let result = serde_yaml::from_str::<EpgConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_epg_config_validate() {
        let config = EpgConfig::default();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_epg_config_validate_cache_dir() {
        let mut config = EpgConfig::default();
        config.cache_dir = Some("/path/to/dir".into());
        config.validate();
    }

    #[test]
    fn test_server_config() {
        assert_eq!(
            serde_yaml::from_str::<ServerConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                addrs:
                  - !http '0.0.0.0:40772'
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: vec![ServerAddr::Http("0.0.0.0:40772".to_string()),],
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                addrs:
                  - !unix /path/to/sock
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: vec![ServerAddr::Unix("/path/to/sock".into()),],
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                addrs:
                  - !http '0.0.0.0:40772'
                  - !unix /path/to/sock
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: vec![
                    ServerAddr::Http("0.0.0.0:40772".to_string()),
                    ServerAddr::Unix("/path/to/sock".into()),
                ],
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                stream-max-chunks: 1000
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: 1000,
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                stream-chunk-size: 10000
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: 10000,
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                stream-time-limit: 10000
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: 10000,
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                program-stream-max-start-delay: 1h
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: Some(Duration::from_secs(3600)),
                mounts: Default::default(),
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                mounts:
                  /ui:
                    path: /path/to/ui
                  /public:
                    path: /path/to/public
                    listing: true
                  /:
                    path: /path/to/folder
                    index: index.html
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: indexmap! {
                    "/ui".to_string() => MountConfig {
                        path: "/path/to/ui".into(),
                        index: None,
                        listing: false,
                    },
                    "/public".to_string() => MountConfig {
                        path: "/path/to/public".into(),
                        index: None,
                        listing: true,
                    },
                    "/".to_string() => MountConfig {
                        path: "/path/to/folder".into(),
                        index: Some("index.html".into()),
                        listing: false,
                    },
                },
                folder_view_template_path: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(
                r#"
                folder-view-template-path: /path/to/listing.html.mustache
            "#
            )
            .unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                program_stream_max_start_delay: None,
                mounts: Default::default(),
                folder_view_template_path: Some("/path/to/listing.html.mustache".into()),
            }
        );

        let result = serde_yaml::from_str::<ServerConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_server_config_validate() {
        let config = ServerConfig::default();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_server_config_validate_stream_time_limit() {
        let mut config = ServerConfig::default();
        config.stream_time_limit = 1;
        config.validate();
    }

    #[test]
    fn test_server_config_validate_max_start_delay() {
        let config = serde_yaml::from_str::<ServerConfig>(
            r#"
            program-stream-max-start-delay: 1h
        "#,
        )
        .unwrap();
        config.validate();

        let config = serde_yaml::from_str::<ServerConfig>(
            r#"
            program-stream-max-start-delay: 23h 59m 59s
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_server_config_validate_max_start_delay_less_than_24h() {
        let config = serde_yaml::from_str::<ServerConfig>(
            r#"
            program-stream-max-start-delay: 24h
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_server_config_validate_addrs() {
        let mut config = ServerConfig::default();
        config.addrs = vec![ServerAddr::Http("invalid".to_string())];
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_server_config_validate_folder_view_template_path() {
        let mut config = ServerConfig::default();
        config.folder_view_template_path = Some("not_found".into());
        config.validate();
    }

    #[test]
    fn test_mount_config_validate() {
        let config = MountConfig {
            path: env!("CARGO_MANIFEST_DIR").into(),
            index: None,
            listing: false,
        };
        config.validate("/test");
        config.validate("/");
    }

    #[test]
    #[should_panic]
    fn test_mount_config_validate_empty_mount_point() {
        let config = MountConfig {
            path: env!("CARGO_MANIFEST_DIR").into(),
            index: None,
            listing: false,
        };
        config.validate("");
    }

    #[test]
    #[should_panic]
    fn test_mount_config_validate_non_existing_path() {
        let config = MountConfig {
            path: "/path/to/dir".into(),
            index: None,
            listing: false,
        };
        config.validate("/test");
    }

    #[test]
    fn test_mount_config_validate_path_file() {
        let config = MountConfig {
            path: "/dev/null".into(),
            index: None,
            listing: false,
        };
        config.validate("/test");
    }

    #[test]
    #[should_panic]
    fn test_mount_config_validate_index() {
        let config = MountConfig {
            path: env!("CARGO_MANIFEST_DIR").into(),
            index: Some("not_found".into()),
            listing: false,
        };
        config.validate("test");
    }

    #[test]
    fn test_channel_config() {
        assert!(serde_yaml::from_str::<ChannelConfig>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(
                r#"
                name: x
                type: GR
                channel: y
            "#
            )
            .unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(
                r#"
                name: x
                type: GR
                channel: y
                extra-args: "--extra args"
            "#
            )
            .unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "--extra args".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(
                r#"
                name: x
                type: GR
                channel: y
                disabled: true
            "#
            )
            .unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: true,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(
                r#"
                name: x
                type: GR
                channel: y
                excluded-services: [100]
            "#
            )
            .unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![100.into()],
                disabled: false,
            }
        );

        assert!(serde_yaml::from_str::<ChannelConfig>(
            r#"
                name: x
                type: WOWOW
                channel: y
            "#
        )
        .is_err());

        let result = serde_yaml::from_str::<ChannelConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_config_normalize() {
        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                - name: ch
                  type: GR
                  channel: 2
            "#
                )
                .unwrap()
            ),
            vec![
                ChannelConfig {
                    name: "ch".to_string(),
                    channel_type: ChannelType::GR,
                    channel: "1".to_string(),
                    extra_args: "".to_string(),
                    services: vec![],
                    excluded_services: vec![],
                    disabled: false,
                },
                ChannelConfig {
                    name: "ch".to_string(),
                    channel_type: ChannelType::GR,
                    channel: "2".to_string(),
                    extra_args: "".to_string(),
                    services: vec![],
                    excluded_services: vec![],
                    disabled: false,
                },
            ],
        );

        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                  services: [1]
                  excluded-services: [3]
                - name: ch
                  type: GR
                  channel: 1
                  services: [2]
                  excluded-services: [4]
            "#
                )
                .unwrap()
            ),
            vec![ChannelConfig {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "1".to_string(),
                extra_args: "".to_string(),
                services: vec![1.into(), 2.into()],
                excluded_services: vec![3.into(), 4.into()],
                disabled: false,
            },],
        );

        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                  services: [2]
                  excluded-services: [4]
                - name: ch
                  type: GR
                  channel: 1
                  services: [1]
                  excluded-services: [3]
            "#
                )
                .unwrap()
            ),
            vec![ChannelConfig {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "1".to_string(),
                extra_args: "".to_string(),
                services: vec![1.into(), 2.into()],
                excluded_services: vec![3.into(), 4.into()],
                disabled: false,
            },],
        );

        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                  services: [1]
                  excluded-services: [3]
                - name: ch
                  type: GR
                  channel: 1
                  services: [1, 2]
                  excluded-services: [3, 4]
            "#
                )
                .unwrap()
            ),
            vec![ChannelConfig {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "1".to_string(),
                extra_args: "".to_string(),
                services: vec![1.into(), 2.into()],
                excluded_services: vec![3.into(), 4.into()],
                disabled: false,
            },],
        );

        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                - name: ch
                  type: GR
                  channel: 1
                  services: [2]
            "#
                )
                .unwrap()
            ),
            vec![ChannelConfig {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "1".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            },],
        );

        assert_eq!(
            ChannelConfig::normalize(
                serde_yaml::from_str::<Vec<ChannelConfig>>(
                    r#"
                - name: ch
                  type: GR
                  channel: 1
                  services: [1]
                - name: ch
                  type: GR
                  channel: 1
            "#
                )
                .unwrap()
            ),
            vec![ChannelConfig {
                name: "ch".to_string(),
                channel_type: ChannelType::GR,
                channel: "1".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            },],
        );
    }

    #[test]
    fn test_channel_config_validate() {
        let config = ChannelConfig {
            name: "test".to_string(),
            channel_type: ChannelType::GR,
            channel: "test".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
            disabled: false,
        };
        config.validate(0);
    }

    #[test]
    #[should_panic]
    fn test_channel_config_validate_empty_name() {
        let config = ChannelConfig {
            name: "".to_string(),
            channel_type: ChannelType::GR,
            channel: "test".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
            disabled: false,
        };
        config.validate(0);
    }

    #[test]
    #[should_panic]
    fn test_channel_config_validate_empty_channel() {
        let config = ChannelConfig {
            name: "test".to_string(),
            channel_type: ChannelType::GR,
            channel: "".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
            disabled: false,
        };
        config.validate(0);
    }

    #[test]
    fn test_tuner_config() {
        assert!(serde_yaml::from_str::<TunerConfig>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(
                r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
            "#
            )
            .unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![
                    ChannelType::GR,
                    ChannelType::BS,
                    ChannelType::CS,
                    ChannelType::SKY
                ],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: false,
                decoded: false,
                dedicated_for: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(
                r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                time-limit: 1
            "#
            )
            .unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![
                    ChannelType::GR,
                    ChannelType::BS,
                    ChannelType::CS,
                    ChannelType::SKY
                ],
                command: "open tuner".to_string(),
                time_limit: 1,
                disabled: false,
                decoded: false,
                dedicated_for: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(
                r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                disabled: true
            "#
            )
            .unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![
                    ChannelType::GR,
                    ChannelType::BS,
                    ChannelType::CS,
                    ChannelType::SKY
                ],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: true,
                decoded: false,
                dedicated_for: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(
                r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                decoded: true
            "#
            )
            .unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![
                    ChannelType::GR,
                    ChannelType::BS,
                    ChannelType::CS,
                    ChannelType::SKY
                ],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: false,
                decoded: true,
                dedicated_for: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(
                r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                dedicated-for: user
            "#
            )
            .unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![
                    ChannelType::GR,
                    ChannelType::BS,
                    ChannelType::CS,
                    ChannelType::SKY
                ],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: false,
                decoded: false,
                dedicated_for: Some("user".to_string()),
            }
        );

        assert!(serde_yaml::from_str::<TunerConfig>(
            r#"
                name: x
                types: [WOWOW]
                command: open tuner
            "#
        )
        .is_err());

        let result = serde_yaml::from_str::<TunerConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_tuner_config_validate() {
        let config = TunerConfig {
            name: "test".to_string(),
            channel_types: vec![ChannelType::GR],
            command: "test".to_string(),
            time_limit: TunerConfig::default_time_limit(),
            disabled: false,
            decoded: false,
            dedicated_for: None,
        };
        config.validate(0);
    }

    #[test]
    #[should_panic]
    fn test_tuner_config_validate_empty_name() {
        let config = TunerConfig {
            name: "".to_string(),
            channel_types: vec![ChannelType::GR],
            command: "test".to_string(),
            time_limit: TunerConfig::default_time_limit(),
            disabled: false,
            decoded: false,
            dedicated_for: None,
        };
        config.validate(0);
    }

    #[test]
    #[should_panic]
    fn test_tuner_config_validate_empty_types() {
        let config = TunerConfig {
            name: "test".to_string(),
            channel_types: vec![],
            command: "test".to_string(),
            time_limit: TunerConfig::default_time_limit(),
            disabled: false,
            decoded: false,
            dedicated_for: None,
        };
        config.validate(0);
    }

    #[test]
    #[should_panic]
    fn test_tuner_config_validate_empty_command() {
        let config = TunerConfig {
            name: "test".to_string(),
            channel_types: vec![ChannelType::GR],
            command: "".to_string(),
            time_limit: TunerConfig::default_time_limit(),
            disabled: false,
            decoded: false,
            dedicated_for: None,
        };
        config.validate(0);
    }

    #[test]
    fn test_tuner_config_validate_disabled() {
        let config = TunerConfig {
            name: "".to_string(),
            channel_types: vec![],
            command: "".to_string(),
            time_limit: TunerConfig::default_time_limit(),
            disabled: true,
            decoded: false,
            dedicated_for: None,
        };
        config.validate(0);
    }

    #[test]
    fn test_filters_config() {
        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(
                r#"
                tuner-filter:
                  command: filter
            "#
            )
            .unwrap(),
            FiltersConfig {
                tuner_filter: FilterConfig {
                    command: "filter".to_string()
                },
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: Default::default(),
                program_filter: FiltersConfig::default_program_filter(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(
                r#"
                service-filter:
                  command: filter
            "#
            )
            .unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FilterConfig {
                    command: "filter".to_string()
                },
                decode_filter: Default::default(),
                program_filter: FiltersConfig::default_program_filter(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(
                r#"
                decode-filter:
                  command: filter
            "#
            )
            .unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: FilterConfig {
                    command: "filter".to_string()
                },
                program_filter: FiltersConfig::default_program_filter(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(
                r#"
                program-filter:
                  command: filter
            "#
            )
            .unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: Default::default(),
                program_filter: FilterConfig {
                    command: "filter".to_string()
                },
            }
        );

        let result = serde_yaml::from_str::<FiltersConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_config() {
        assert_eq!(
            serde_yaml::from_str::<FilterConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<FilterConfig>(
                r#"
                command: filter
            "#
            )
            .unwrap(),
            FilterConfig {
                command: "filter".to_string(),
            }
        );

        let result = serde_yaml::from_str::<FilterConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_config_validate() {
        let config = FilterConfig {
            command: "test".to_string(),
        };
        config.validate("filters", "test");
    }

    #[test]
    #[should_panic]
    fn test_filter_config_validate_empty_command() {
        let config = FilterConfig {
            command: "".to_string(),
        };
        config.validate("filters", "test");
    }

    #[test]
    fn test_post_filter_config() {
        assert_eq!(
            serde_yaml::from_str::<PostFilterConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<PostFilterConfig>(
                r#"
                command: filter
            "#
            )
            .unwrap(),
            PostFilterConfig {
                command: "filter".to_string(),
                content_type: None,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<PostFilterConfig>(
                r#"
                command: filter
                content-type: video/mp4
            "#
            )
            .unwrap(),
            PostFilterConfig {
                command: "filter".to_string(),
                content_type: Some("video/mp4".to_string()),
            }
        );

        let result = serde_yaml::from_str::<PostFilterConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_post_filter_config_validate() {
        let config = PostFilterConfig {
            command: "test".to_string(),
            content_type: None,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_post_filter_config_validate_empty_command() {
        let config = PostFilterConfig {
            command: "".to_string(),
            content_type: None,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_post_filter_config_validate_empty_content_type() {
        let config = PostFilterConfig {
            command: "test".to_string(),
            content_type: Some("".to_string()),
        };
        config.validate("test");
    }

    #[test]
    fn test_jobs_config() {
        assert_eq!(
            serde_yaml::from_str::<JobsConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(
                r#"
                scan-services:
                  command: job
                  schedule: '*'
            "#
            )
            .unwrap(),
            JobsConfig {
                scan_services: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
                sync_clocks: JobsConfig::default_sync_clocks(),
                update_schedules: JobsConfig::default_update_schedules(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(
                r#"
                sync-clocks:
                  command: job
                  schedule: '*'
            "#
            )
            .unwrap(),
            JobsConfig {
                scan_services: JobsConfig::default_scan_services(),
                sync_clocks: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
                update_schedules: JobsConfig::default_update_schedules(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(
                r#"
                update-schedules:
                  command: job
                  schedule: '*'
            "#
            )
            .unwrap(),
            JobsConfig {
                scan_services: JobsConfig::default_scan_services(),
                sync_clocks: JobsConfig::default_sync_clocks(),
                update_schedules: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
            }
        );

        let result = serde_yaml::from_str::<JobsConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_jobs_config_normalize() {
        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(
                r#"
                scan-services:
                  command: ''
                  schedule: ''
                sync-clocks:
                  command: ''
                  schedule: ''
                update-schedules:
                  command: ''
                  schedule: ''
            "#
            )
            .unwrap()
            .normalize(),
            JobsConfig::default()
        );
    }

    #[test]
    fn test_job_config() {
        assert_eq!(
            serde_yaml::from_str::<JobConfig>("{}").unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "".to_string(),
                disabled: false,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(
                r#"
                command: test
            "#
            )
            .unwrap(),
            JobConfig {
                command: "test".to_string(),
                schedule: "".to_string(),
                disabled: false,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(
                r#"
                schedule: '*'
            "#
            )
            .unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "*".to_string(),
                disabled: false,
            }
        );

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(
                r#"
                disabled: true
            "#
            )
            .unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "".to_string(),
                disabled: true,
            }
        );

        assert!(serde_yaml::from_str::<JobConfig>(
            r#"
            unknown:
              property: value
        "#
        )
        .is_err());
    }

    #[test]
    fn test_job_config_validate() {
        let config = JobConfig {
            command: "test".to_string(),
            schedule: "0 30 9,12,15 1,15 May-Aug Mon,Wed,Fri 2018/2".to_string(),
            disabled: false,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_job_config_validate_command() {
        let config = JobConfig {
            command: "".to_string(),
            schedule: "0 30 9,12,15 1,15 May-Aug Mon,Wed,Fri 2018/2".to_string(),
            disabled: false,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_job_config_validate_schedule() {
        let config = JobConfig {
            command: "test".to_string(),
            schedule: "".to_string(),
            disabled: false,
        };
        config.validate("test");
    }

    #[test]
    fn test_job_config_validate_disabled() {
        let config = JobConfig {
            command: "".to_string(),
            schedule: "".to_string(),
            disabled: true,
        };
        config.validate("test");
    }

    #[test]
    fn test_recording_config() {
        assert_eq!(
            serde_yaml::from_str::<RecordingConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<RecordingConfig>(
                r#"
                basedir: /tmp
            "#
            )
            .unwrap(),
            RecordingConfig {
                basedir: Some("/tmp".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn test_recording_is_enabled() {
        assert!(!RecordingConfig::default().is_enabled());

        assert!(serde_yaml::from_str::<RecordingConfig>(
            r#"
                basedir: /tmp
            "#
        )
        .unwrap()
        .is_enabled());
    }

    #[test]
    fn test_recording_config_validate_basedir() {
        let config = serde_yaml::from_str::<RecordingConfig>(
            r#"
            basedir: /tmp
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_recording_config_validate_basedir_relative() {
        let config = serde_yaml::from_str::<RecordingConfig>(
            r#"
            basedir: relative/dir
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_recording_config_validate_basedir_not_existing() {
        let config = serde_yaml::from_str::<RecordingConfig>(
            r#"
            basedir: /no/such/dir
        "#,
        )
        .unwrap();
        config.validate();
    }

    #[test]
    fn test_timeshift_config() {
        assert_eq!(
            serde_yaml::from_str::<TimeshiftConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<TimeshiftConfig>(
                r#"
                command: command
                recorders:
                  test:
                    service-triple: [1, 2, 3]
                    ts-file: /path/to/timeshift.m2ts
                    data-file: /path/to/timeshift.json
                    num-chunks: 100
            "#
            )
            .unwrap(),
            TimeshiftConfig {
                command: "command".to_string(),
                recorders: indexmap! {
                    "test".to_string() => TimeshiftRecorderConfig {
                        service_triple: (1.into(), 2.into(), 3.into()),
                        ts_file: "/path/to/timeshift.m2ts".to_string(),
                        data_file: "/path/to/timeshift.json".to_string(),
                        chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
                        num_chunks: 100,
                        num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
                        priority: TimeshiftRecorderConfig::default_priority(),
                    },
                },
            },
        );

        assert!(serde_yaml::from_str::<TimeshiftConfig>(
            r#"
            unknown: property
        "#
        )
        .is_err());
    }

    #[test]
    fn test_timeshift_is_enabled() {
        assert!(!TimeshiftConfig::default().is_enabled());

        assert!(serde_yaml::from_str::<TimeshiftConfig>(
            r#"
                command: command
                recorders:
                  test:
                    service-triple: [1, 2, 3]
                    ts-file: /path/to/timeshift.m2ts
                    data-file: /path/to/timeshift.json
                    num-chunks: 100
            "#
        )
        .unwrap()
        .is_enabled());

        assert!(!serde_yaml::from_str::<TimeshiftConfig>(
            r#"
                command: ''
                recorders:
                  test:
                    service-triple: [1, 2, 3]
                    ts-file: /path/to/timeshift.m2ts
                    data-file: /path/to/timeshift.json
                    num-chunks: 100
            "#
        )
        .unwrap()
        .is_enabled());
    }

    #[test]
    fn test_timeshift_config_validate() {
        let config = TimeshiftConfig {
            command: "test".to_string(),
            recorders: indexmap! {},
        };
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_timeshift_config_validate_empty_command() {
        let config = TimeshiftConfig {
            command: "".to_string(),
            recorders: indexmap! {},
        };
        config.validate();
    }

    #[test]
    fn test_timeshift_recorder_config() {
        assert!(serde_yaml::from_str::<TimeshiftRecorderConfig>("{}").is_err());

        assert!(serde_yaml::from_str::<TimeshiftRecorderConfig>(
            r#"
            service-triple: [1, 2, 3]
            ts-file: /path/to/timeshift.m2ts
            data-file: /path/to/timeshift.data
            num-chunks: 100
            unknown: property
        "#
        )
        .is_err());

        assert_eq!(
            serde_yaml::from_str::<TimeshiftRecorderConfig>(
                r#"
                service-triple: [1, 2, 3]
                ts-file: /path/to/timeshift.m2ts
                data-file: /path/to/timeshift.data
                num-chunks: 100
            "#
            )
            .unwrap(),
            TimeshiftRecorderConfig {
                service_triple: (1.into(), 2.into(), 3.into()),
                ts_file: "/path/to/timeshift.m2ts".to_string(),
                data_file: "/path/to/timeshift.data".to_string(),
                chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
                num_chunks: 100,
                num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
                priority: TimeshiftRecorderConfig::default_priority(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<TimeshiftRecorderConfig>(
                r#"
                service-triple: [1, 2, 3]
                ts-file: /path/to/timeshift.m2ts
                data-file: /path/to/timeshift.data
                chunk-size: 8192
                num-chunks: 100
                num-reserves: 2
                priority: 2
            "#
            )
            .unwrap(),
            TimeshiftRecorderConfig {
                service_triple: (1.into(), 2.into(), 3.into()),
                ts_file: "/path/to/timeshift.m2ts".to_string(),
                data_file: "/path/to/timeshift.data".to_string(),
                chunk_size: 8192,
                num_chunks: 100,
                num_reserves: 2,
                priority: 2.into(),
            }
        );
    }

    #[test]
    fn test_timeshift_recorder_config_validate() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_empty_ts_file() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_ts_file_parent_dir() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/path/to/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_ts_file_is_file() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_empty_data_file() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_empty_data_file_parent_dir() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/path/to/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_empty_data_file_is_file() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_data_bincode() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.bincode".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_chunk_size_0() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: 0,
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_chunk_size_1() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: 1,
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_chunk_size_8192() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: 8192,
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    fn test_timeshift_recorder_config_validate_chunk_size_1540096() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: 1540096,
            num_chunks: 10,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_num_chunks_1() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 1,
            num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_num_reserves_0() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: 0,
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_timeshift_recorder_config_validate_available_chunks_1() {
        let config = TimeshiftRecorderConfig {
            service_triple: (1.into(), 2.into(), 3.into()),
            ts_file: "/ts.m2ts".to_string(),
            data_file: "/data.json".to_string(),
            chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
            num_chunks: 10,
            num_reserves: 9,
            priority: TimeshiftRecorderConfig::default_priority(),
        };
        config.validate("test");
    }

    #[test]
    fn test_events_config() {
        assert_eq!(
            serde_yaml::from_str::<EventsConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<EventsConfig>(
                r#"
                concurrency: !number 2
            "#
            )
            .unwrap(),
            EventsConfig {
                concurrency: Concurrency::Number(2),
                ..Default::default()
            }
        );

        assert_eq!(
            serde_yaml::from_str::<EventsConfig>(
                r#"
                concurrency: !num-cpus 0.5
            "#
            )
            .unwrap(),
            EventsConfig {
                concurrency: Concurrency::NumCpus(0.5),
                ..Default::default()
            }
        );

        assert_eq!(
            serde_yaml::from_str::<EventsConfig>(
                r#"
                concurrency: !unlimited
            "#
            )
            .unwrap(),
            EventsConfig {
                concurrency: Concurrency::Unlimited,
                ..Default::default()
            }
        );
    }

    #[test]
    fn test_local_onair_program_tracker_config() {
        assert_eq!(
            serde_yaml::from_str::<LocalOnairProgramTrackerConfig>(
                r#"
                channel-types: [GR]
            "#
            )
            .unwrap(),
            LocalOnairProgramTrackerConfig {
                channel_types: hashset![ChannelType::GR],
                services: Default::default(),
                excluded_services: Default::default(),
                command: LocalOnairProgramTrackerConfig::default_command(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<LocalOnairProgramTrackerConfig>(
                r#"
                channel-types: [GR]
                services: [1]
                excluded-services: [2]
                command: "true"
            "#
            )
            .unwrap(),
            LocalOnairProgramTrackerConfig {
                channel_types: hashset![ChannelType::GR],
                services: hashset![1.into()],
                excluded_services: hashset![2.into()],
                command: "true".to_string(),
            }
        );
    }

    #[test]
    fn test_local_onair_program_tracker_config_validate() {
        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![],
            excluded_services: hashset![],
            command: LocalOnairProgramTrackerConfig::default_command(),
        };
        config.validate("test");

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![1.into()],
            excluded_services: hashset![2.into()],
            command: "true".to_string(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_local_onair_program_tracker_config_validate_channel_types() {
        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![],
            services: hashset![],
            excluded_services: hashset![],
            command: LocalOnairProgramTrackerConfig::default_command(),
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_local_onair_program_tracker_config_validate_command() {
        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![],
            excluded_services: hashset![],
            command: "".to_string(),
        };
        config.validate("test");
    }

    #[test]
    fn test_resource_config() {
        assert_eq!(
            serde_yaml::from_str::<ResourceConfig>("{}").unwrap(),
            Default::default()
        );

        assert_eq!(
            serde_yaml::from_str::<ResourceConfig>(
                r#"
                strings-yaml: /path/to/strings.yml
            "#
            )
            .unwrap(),
            ResourceConfig {
                strings_yaml: "/path/to/strings.yml".to_string(),
                logos: Default::default(),
            }
        );

        assert_eq!(
            serde_yaml::from_str::<ResourceConfig>(
                r#"
                logos:
                  - service-triple: [1, 2, 3]
                    image: /path/to/logo.png
            "#
            )
            .unwrap(),
            ResourceConfig {
                strings_yaml: ResourceConfig::default_strings_yaml(),
                logos: hashmap! {
                    ServiceTriple::new(1.into(), 2.into(), 3.into())
                        => "/path/to/logo.png".to_string(),
                },
            }
        );

        let result = serde_yaml::from_str::<ResourceConfig>(
            r#"
            unknown:
              property: value
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_resource_config_validate_existing_strings_yaml() {
        let mut config = ResourceConfig::default();
        config.strings_yaml = "/bin/sh".to_string();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_resource_config_validate_non_existing_strings_yaml() {
        let mut config = ResourceConfig::default();
        config.strings_yaml = "/path/to/non-existing".to_string();
        config.validate();
    }

    #[test]
    fn test_resource_config_validate_existing_logos() {
        let config = ResourceConfig {
            strings_yaml: "/bin/sh".to_string(),
            logos: hashmap! {
                ServiceTriple::new(1.into(), 2.into(), 3.into())
                    => "/bin/sh".to_string(),
            },
        };
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_epg_config_validate_non_existing_logos() {
        let config = ResourceConfig {
            strings_yaml: "/bin/sh".to_string(),
            logos: hashmap! {
                ServiceTriple::new(1.into(), 2.into(), 3.into())
                    => "/path/to/non-existing".to_string(),
            },
        };
        config.validate();
    }
}
// </coverage:exclude>
