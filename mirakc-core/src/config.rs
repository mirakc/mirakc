use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use cron;
use indexmap::IndexMap;
use itertools::Itertools;
use num_cpus;
use serde::Deserialize;
use serde_yaml;

use crate::models::*;

pub fn load(config_path: &str) -> Arc<Config> {
    let reader = File::open(config_path)
        .unwrap_or_else(|err| {
            panic!("Failed to open {}: {}", config_path, err);
        });
    let mut config: Config = serde_yaml::from_reader(reader)
        .unwrap_or_else(|err| {
            panic!("Failed to parse {}: {}", config_path, err);
        });

    config.validate();

    config.last_modified = std::fs::metadata(config_path)
        .map(|metadata| metadata.modified().ok()).ok().flatten();
    Arc::new(config)
}

// result

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
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
    pub recorder: RecorderConfig,
    #[serde(default)]
    pub timeshift: TimeshiftConfig,
    #[serde(default)]
    pub resource: ResourceConfig,
    #[serde(default)]
    pub mirakurun: MirakurunConfig,
}

impl Config {
    fn validate(&self) {
        self.epg.validate();
        self.server.validate();
        self.channels.iter()
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
        self.tuners.iter()
            .enumerate()
            .for_each(|(i, config)| config.validate(i));
        assert_eq!(self.tuners.len(),
                   self.tuners.iter()
                   .map(|config| &config.name)
                   .unique()
                   .count(),
                   "config.tuners: `name` must be a unique");
        self.filters.validate();
        self.pre_filters.iter()
            .for_each(|(name, config)| config.validate("pre-filters", name));
        self.post_filters.iter()
            .for_each(|(name, config)| config.validate(name));
        self.jobs.validate();
        self.recorder.validate();
        self.timeshift.validate();
        self.resource.validate();
        self.mirakurun.validate();
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct EpgConfig {
    #[serde(default)]
    pub cache_dir: Option<String>,
}

impl EpgConfig {
    fn validate(&self) {
        if let Some(cache_dir) = self.cache_dir.as_ref() {
            assert!(Path::new(cache_dir).is_dir(),
                    "config.epg: `cache_dir` must be a path to an existing directory");
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_addrs")]
    pub addrs: Vec<ServerAddr>,
    #[serde(default = "ServerConfig::default_workers")]
    pub workers: usize,
    #[serde(default = "ServerConfig::default_stream_max_chunks")]
    pub stream_max_chunks: usize,
    #[serde(default = "ServerConfig::default_stream_chunk_size")]
    pub stream_chunk_size: usize,
    #[serde(default = "ServerConfig::default_stream_time_limit")]
    pub stream_time_limit: u64,
    #[serde(default)]
    pub mounts: IndexMap<String, MountConfig>,  // keeps the insertion order
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ServerAddr {
    Http(String),
    Unix(String),
}

impl ServerConfig {
    fn default_addrs() -> Vec<ServerAddr> {
        vec![ServerAddr::Http("localhost:40772".to_string())]
    }

    fn default_workers() -> usize {
        num_cpus::get()
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
        16 * 1000  // 16s
    }

    fn validate(&self) {
        const SERVER_STREAM_TIME_LIMIT_MIN: u64 = 15_000;

        assert!(self.stream_time_limit >= SERVER_STREAM_TIME_LIMIT_MIN,
                "config.server: `stream_time_limit` must be larger than or equal to {}",
                SERVER_STREAM_TIME_LIMIT_MIN);
        self.mounts.iter()
            .for_each(|(mp, config)| config.validate(mp));
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            addrs: Self::default_addrs(),
            workers: Self::default_workers(),
            stream_max_chunks: Self::default_stream_max_chunks(),
            stream_chunk_size: Self::default_stream_chunk_size(),
            stream_time_limit: Self::default_stream_time_limit(),
            mounts: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct MountConfig {
    pub path: String,
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub listing: bool,
}

impl MountConfig {
    fn validate(&self, mount_point: &str) {
        assert!(Path::new(&self.path).exists(),
                "config.server.mounts.{}: \
                 `path` must be a path to an existing entry", mount_point);
        if let Some(index) = self.index.as_ref() {
            assert!(!index.is_empty(),
                    "config.server.mounts.{}: `index` must be a non-empty string", mount_point);
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
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
    fn validate(&self, index: usize) {
        if self.disabled {
            return;
        }
        assert!(!self.name.is_empty(),
                "config.channels[{}]: `name` must be a non-empty string", index);
        assert!(!self.channel.is_empty(),
                "config.channels[{}]: `channel` must be a non-empty string", index);
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
}

impl TunerConfig {
    fn default_time_limit() -> u64 {
        30 * 1000  // 30s
    }

    fn validate(&self, index: usize) {
        if self.disabled {
            return;
        }
        assert!(!self.name.is_empty(),
                "config.tuners[{}]: `name` must be a non-empty string", index);
        assert!(!self.channel_types.is_empty(),
                "config.tuners[{}]: `types` must be a non-empty list", index);
        assert!(!self.command.is_empty(),
                "config.tuners[{}]: `command` must be a non-empty string", index);
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
            command: "mirakc-arib filter-service --sid={{{sid}}}".to_string()
        }
    }

    fn default_program_filter() -> FilterConfig {
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
                      ".to_string()
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
        assert!(!self.command.is_empty(),
                "config.{}.{}: `command` must be a non-empty string", group, name);
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
        assert!(!self.command.is_empty(),
                "config.post-filters.{}: `command` must be a non-empty string", name);
        if let Some(content_type) = self.content_type.as_ref() {
        assert!(!content_type.is_empty(),
                "config.post-filters.{}: `content-type` must be a non-empty string", name);
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
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}".to_string(),
            schedule: "0 31 5 * * * *".to_string(),
            disabled: false,
        }
    }

    fn default_sync_clocks() -> JobConfig {
        JobConfig {
            command: "mirakc-arib sync-clocks\
                      {{#sids}} --sids={{{.}}}{{/sids}}\
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}".to_string(),
            schedule: "0 3 12 * * * *".to_string(),
            disabled: false,
        }
    }

    fn default_update_schedules() -> JobConfig {
        JobConfig {
            command: "mirakc-arib collect-eits\
                      {{#sids}} --sids={{{.}}}{{/sids}}\
                      {{#xsids}} --xsids={{{.}}}{{/xsids}}".to_string(),
            schedule: "0 7,37 * * * * *".to_string(),
            disabled: false,
        }
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
            assert!(!self.command.is_empty(),
                    "config.jobs.{}: `command` must be a non-empty string", name);
            assert!(cron::Schedule::from_str(&self.schedule).is_ok(),
                    "config.jobs.{}: `schedule` is not valid", name);
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
    fn default_command() -> String {
        "mirakc-arib record-service --sid={{{sid}}} --file={{{file}}} \
         --chunk-size={{{chunk_size}}} --num-chunks={{{num_chunks}}} \
         --start-pos={{{start_pos}}}".to_string()
    }

    fn validate(&self) {
        assert!(!self.command.is_empty(),
                "config.timeshift: `command` must be a non-empty string");
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

    pub fn max_file_size(&self) -> u64 {
        (self.chunk_size as u64) * (self.num_chunks as u64)
    }

    pub fn max_chunks(&self) -> usize {
        assert!(self.num_chunks > self.num_reserves);
        self.num_chunks - self.num_reserves
    }

    fn validate(&self, name: &str) {
        assert!(!self.ts_file.is_empty(),
                "config.timeshift.recorders.{}: `ts-file` must be a non-empty path", name);
        if let Some(parent) = Path::new(&self.ts_file).parent() {
            assert!(parent.is_dir(),
                    "config.timeshift.recorders.{}: \
                     The parent directory of `ts-file` must exists", name);
        } else {
            unreachable!(
                "config.timeshift.recorders.{}: `ts-file` must be a path to a file", name);
        }
        assert!(!self.data_file.is_empty(),
                "config.timeshift.recorders.{}: `data-file` must be a non-empty path", name);
        if let Some(parent) = Path::new(&self.data_file).parent() {
            assert!(parent.is_dir(),
                    "config.timeshift.recorders.{}: \
                     The parent directory of `data-file` must exists", name);
        } else {
            unreachable!(
                "config.timeshift.recorders.{}: `data-file` must be a path to a file", name);
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
        assert!(self.data_file.ends_with(".json"),
                "config.timeshift.recorders.{}: `data-file` must be a JSON file", name);
        assert!(self.chunk_size > 0,
                "config.timeshift.recorders.{}: `chunk-size` must be larger than 0", name);
        assert!(self.chunk_size % Self::BUFSIZE == 0,
                "config.timeshift.recorders.{}: `chunk-size` must be a multiple of {}",
                name, Self::BUFSIZE);
        assert!(self.num_chunks > 2,
                "config.timeshift.recorders.{}: `num-chunks` must be larger than 2", name);
        assert!(self.num_reserves > 0,
                "config.timeshift.recorders.{}: `num-reserves` must be larger than 0", name);
        assert!(self.num_chunks - self.num_reserves > 1,
                "config.timeshift.recorders.{}: Maximum number of available chunks \
                 (`num-chunks` - `num-reserves`) must be larger than 1", name);
    }

    fn default_chunk_size() -> usize {
        Self::BUFSIZE * 20000  // 40K Blocks
    }

    fn default_num_reserves() -> usize {
        1
    }

    fn default_priority() -> i32 {
        TunerUserPriority::MAX
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct RecorderConfig {
    pub track_airtime_command: String,
}

impl RecorderConfig {
    fn validate(&self) {
        assert!(!self.track_airtime_command.is_empty(),
                "config.recorder: `track-airtime-command` must be a non-empty string");
    }
}

impl Default for RecorderConfig {
    fn default() -> Self {
        RecorderConfig {
            track_airtime_command: "mirakc-arib track-airtime \
                                    --sid={{{sid}}} --eid={{{eid}}}".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    #[serde(default = "ResourceConfig::default_strings_yaml")]
    pub strings_yaml: String,
}

impl ResourceConfig {
    fn default_strings_yaml() -> String {
        "/etc/mirakc/strings.yml".to_string()
    }

    fn validate(&self) {
        assert!(Path::new(&self.strings_yaml).is_file(),
                "config.resources: `strings-yaml` must be a path to a existing YAML file");
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        ResourceConfig {
            strings_yaml: Self::default_strings_yaml(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct MirakurunConfig {
    #[serde(default = "MirakurunConfig::default_openapi_json")]
    pub openapi_json: String,
}

impl MirakurunConfig {
    fn default_openapi_json() -> String {
        "/etc/mirakurun.openapi.json".to_string()
    }

    fn validate(&self) {
        assert!(Path::new(&self.openapi_json).is_file(),
                "config.resources: `openapi-json` must be a path to a existing JSON file");
    }
}

impl Default for MirakurunConfig {
    fn default() -> Self {
        MirakurunConfig {
            openapi_json: Self::default_openapi_json(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::indexmap;

    #[test]
    fn test_config() {
        assert_eq!(
            serde_yaml::from_str::<Config>("{}").unwrap(),
            Default::default());

        let result = serde_yaml::from_str::<Config>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validate() {
        let mut config = Config::default();
        config.resource.strings_yaml = "/bin/sh".to_string();
        config.mirakurun.openapi_json = "/bin/sh".to_string();
        config.validate();
    }

    #[test]
    fn test_config_validate_channel_names() {
        let config = serde_yaml::from_str::<Config>(r#"
            channels:
              - name: test
                type: GR
                channel: test
              - name: test
                type: GR
                channel: test
            resource:
              strings-yaml: /bin/sh
            mirakurun:
              openapi-json: /bin/sh
        "#).unwrap();
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_config_validate_tuner_names() {
        let config = serde_yaml::from_str::<Config>(r#"
            tuners:
              - name: test
                types: [GR]
                command: test
              - name: test
                types: [GR]
                command: test
            resource:
              strings-yaml: /bin/sh
            mirakurun:
              openapi-json: /bin/sh
        "#).unwrap();
        config.validate();
    }

    #[test]
    fn test_epg_config() {
        assert_eq!(
            serde_yaml::from_str::<EpgConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<EpgConfig>(r#"
                cache-dir: /path/to/epg
            "#).unwrap(),
            EpgConfig {
                cache_dir: Some("/path/to/epg".to_string()),
            });

        let result = serde_yaml::from_str::<EpgConfig>(r#"
            unknown:
              property: value
        "#);
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
        config.cache_dir = Some("/path/to/dir".to_string());
        config.validate();
    }

    #[test]
    fn test_server_config() {
        assert_eq!(
            serde_yaml::from_str::<ServerConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                addrs:
                  - http: '0.0.0.0:40772'
            "#).unwrap(),
            ServerConfig {
                addrs: vec![
                    ServerAddr::Http("0.0.0.0:40772".to_string()),
                ],
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                addrs:
                  - unix: /path/to/sock
            "#).unwrap(),
            ServerConfig {
                addrs: vec![
                    ServerAddr::Unix("/path/to/sock".to_string()),
                ],
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                addrs:
                  - http: '0.0.0.0:40772'
                  - unix: /path/to/sock
            "#).unwrap(),
            ServerConfig {
                addrs: vec![
                    ServerAddr::Http("0.0.0.0:40772".to_string()),
                    ServerAddr::Unix("/path/to/sock".to_string()),
                ],
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                workers: 2
            "#).unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                workers: 2,
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                stream-max-chunks: 1000
            "#).unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                workers: ServerConfig::default_workers(),
                stream_max_chunks: 1000,
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                stream-chunk-size: 10000
            "#).unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: 10000,
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                stream-time-limit: 10000
            "#).unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: 10000,
                mounts: Default::default(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                mounts:
                  /ui:
                    path: /path/to/ui
                  /public:
                    path: /path/to/public
                    listing: true
                  /:
                    path: /path/to/folder
                    index: index.html
            "#).unwrap(),
            ServerConfig {
                addrs: ServerConfig::default_addrs(),
                workers: ServerConfig::default_workers(),
                stream_max_chunks: ServerConfig::default_stream_max_chunks(),
                stream_chunk_size: ServerConfig::default_stream_chunk_size(),
                stream_time_limit: ServerConfig::default_stream_time_limit(),
                mounts: indexmap!{
                    "/ui".to_string() => MountConfig {
                        path: "/path/to/ui".to_string(),
                        index: None,
                        listing: false,
                    },
                    "/public".to_string() => MountConfig {
                        path: "/path/to/public".to_string(),
                        index: None,
                        listing: true,
                    },
                    "/".to_string() => MountConfig {
                        path: "/path/to/folder".to_string(),
                        index: Some("index.html".to_string()),
                        listing: false,
                    },
                }
            });

        let result = serde_yaml::from_str::<ServerConfig>(r#"
            unknown:
              property: value
        "#);
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
    fn test_mount_config_validate() {
        let config = MountConfig {
            path: "/".to_string(),
            index: None,
            listing: false,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_mount_config_validate_path() {
        let config = MountConfig {
            path: "/path/to/dir".to_string(),
            index: None,
            listing: false,
        };
        config.validate("test");
    }

    #[test]
    #[should_panic]
    fn test_mount_config_validate_index() {
        let config = MountConfig {
            path: "/".to_string(),
            index: Some("".to_string()),
            listing: false,
        };
        config.validate("test");
    }

    #[test]
    fn test_channel_config() {
        assert!(serde_yaml::from_str::<ChannelConfig>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: GR
                channel: y
            "#).unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            });

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: GR
                channel: y
                extra-args: "--extra args"
            "#).unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "--extra args".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: false,
            });

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: GR
                channel: y
                disabled: true
            "#).unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![],
                disabled: true,
            });

        assert_eq!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: GR
                channel: y
                excluded-services: [100]
            "#).unwrap(),
            ChannelConfig {
                name: "x".to_string(),
                channel_type: ChannelType::GR,
                channel: "y".to_string(),
                extra_args: "".to_string(),
                services: vec![],
                excluded_services: vec![100.into()],
                disabled: false,
            });

        assert!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: WOWOW
                channel: y
            "#).is_err());

        let result = serde_yaml::from_str::<ChannelConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
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
    fn test_channel_config_validate_disabled() {
        let config = ChannelConfig {
            name: "".to_string(),
            channel_type: ChannelType::GR,
            channel: "".to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
            disabled: true,
        };
        config.validate(0);
    }

    #[test]
    fn test_tuner_config() {
        assert!(serde_yaml::from_str::<TunerConfig>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
            "#).unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![ChannelType::GR,
                                    ChannelType::BS,
                                    ChannelType::CS,
                                    ChannelType::SKY],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: false,
                decoded: false,
            });

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                time-limit: 1
            "#).unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![ChannelType::GR,
                                    ChannelType::BS,
                                    ChannelType::CS,
                                    ChannelType::SKY],
                command: "open tuner".to_string(),
                time_limit: 1,
                disabled: false,
                decoded: false,
            });

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                disabled: true
            "#).unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![ChannelType::GR,
                                    ChannelType::BS,
                                    ChannelType::CS,
                                    ChannelType::SKY],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: true,
                decoded: false,
            });

        assert_eq!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [GR, BS, CS, SKY]
                command: open tuner
                decoded: true
            "#).unwrap(),
            TunerConfig {
                name: "x".to_string(),
                channel_types: vec![ChannelType::GR,
                                    ChannelType::BS,
                                    ChannelType::CS,
                                    ChannelType::SKY],
                command: "open tuner".to_string(),
                time_limit: TunerConfig::default_time_limit(),
                disabled: false,
                decoded: true,
            });

        assert!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [WOWOW]
                command: open tuner
            "#).is_err());

        let result = serde_yaml::from_str::<TunerConfig>(r#"
            unknown:
              property: value
        "#);
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
        };
        config.validate(0);
    }

    #[test]
    fn test_filters_config() {
        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(r#"
                tuner-filter:
                  command: filter
            "#).unwrap(),
            FiltersConfig {
                tuner_filter: FilterConfig { command: "filter".to_string() },
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: Default::default(),
                program_filter: FiltersConfig::default_program_filter(),
            });

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(r#"
                service-filter:
                  command: filter
            "#).unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FilterConfig { command: "filter".to_string() },
                decode_filter: Default::default(),
                program_filter: FiltersConfig::default_program_filter(),
            });

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(r#"
                decode-filter:
                  command: filter
            "#).unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: FilterConfig { command: "filter".to_string() },
                program_filter: FiltersConfig::default_program_filter(),
            });

        assert_eq!(
            serde_yaml::from_str::<FiltersConfig>(r#"
                program-filter:
                  command: filter
            "#).unwrap(),
            FiltersConfig {
                tuner_filter: Default::default(),
                service_filter: FiltersConfig::default_service_filter(),
                decode_filter: Default::default(),
                program_filter: FilterConfig { command: "filter".to_string() },
            });

        let result = serde_yaml::from_str::<FiltersConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_config() {
        assert_eq!(
            serde_yaml::from_str::<FilterConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<FilterConfig>(r#"
                command: filter
            "#).unwrap(),
            FilterConfig {
                command: "filter".to_string(),
            });

        let result = serde_yaml::from_str::<FilterConfig>(r#"
            unknown:
              property: value
        "#);
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
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<PostFilterConfig>(r#"
                command: filter
            "#).unwrap(),
            PostFilterConfig {
                command: "filter".to_string(),
                content_type: None,
            });

        assert_eq!(
            serde_yaml::from_str::<PostFilterConfig>(r#"
                command: filter
                content-type: video/mp4
            "#).unwrap(),
            PostFilterConfig {
                command: "filter".to_string(),
                content_type: Some("video/mp4".to_string()),
            });

        let result = serde_yaml::from_str::<PostFilterConfig>(r#"
            unknown:
              property: value
        "#);
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
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(r#"
                scan-services:
                  command: job
                  schedule: '*'
            "#).unwrap(),
            JobsConfig {
                scan_services: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
                sync_clocks: JobsConfig::default_sync_clocks(),
                update_schedules: JobsConfig::default_update_schedules(),
            });

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(r#"
                sync-clocks:
                  command: job
                  schedule: '*'
            "#).unwrap(),
            JobsConfig {
                scan_services: JobsConfig::default_scan_services(),
                sync_clocks: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
                update_schedules: JobsConfig::default_update_schedules(),
            });

        assert_eq!(
            serde_yaml::from_str::<JobsConfig>(r#"
                update-schedules:
                  command: job
                  schedule: '*'
            "#).unwrap(),
            JobsConfig {
                scan_services: JobsConfig::default_scan_services(),
                sync_clocks: JobsConfig::default_sync_clocks(),
                update_schedules: JobConfig {
                    command: "job".to_string(),
                    schedule: "*".to_string(),
                    disabled: false,
                },
            });

        let result = serde_yaml::from_str::<JobsConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_job_config() {
        assert_eq!(
            serde_yaml::from_str::<JobConfig>("{}").unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "".to_string(),
                disabled: false,
            });

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(r#"
                command: test
            "#).unwrap(),
            JobConfig {
                command: "test".to_string(),
                schedule: "".to_string(),
                disabled: false,
            });

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(r#"
                schedule: '*'
            "#).unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "*".to_string(),
                disabled: false,
            });

        assert_eq!(
            serde_yaml::from_str::<JobConfig>(r#"
                disabled: true
            "#).unwrap(),
            JobConfig {
                command: "".to_string(),
                schedule: "".to_string(),
                disabled: true,
            });

        assert!(serde_yaml::from_str::<JobConfig>(r#"
            unknown:
              property: value
        "#).is_err());
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
    fn test_timeshift_config() {
        assert_eq!(
            serde_yaml::from_str::<TimeshiftConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<TimeshiftConfig>(r#"
                command: command
                recorders:
                  test:
                    service-triple: [1, 2, 3]
                    ts-file: /path/to/timeshift.m2ts
                    data-file: /path/to/timeshift.json
                    num-chunks: 100
            "#).unwrap(),
            TimeshiftConfig {
                command: "command".to_string(),
                recorders: indexmap!{
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

        assert!(serde_yaml::from_str::<TimeshiftConfig>(r#"
            unknown: property
        "#).is_err());
    }

    #[test]
    fn test_timeshift_config_validate() {
        let config = TimeshiftConfig {
            command: "test".to_string(),
            recorders: indexmap!{},
        };
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_timeshift_config_validate_empty_command() {
        let config = TimeshiftConfig {
            command: "".to_string(),
            recorders: indexmap!{},
        };
        config.validate();
    }

    #[test]
    fn test_timeshift_recorder_config() {
        assert!(serde_yaml::from_str::<TimeshiftRecorderConfig>("{}").is_err());

        assert!(serde_yaml::from_str::<TimeshiftRecorderConfig>(r#"
            service-triple: [1, 2, 3]
            ts-file: /path/to/timeshift.m2ts
            data-file: /path/to/timeshift.data
            num-chunks: 100
            unknown: property
        "#).is_err());

        assert_eq!(
            serde_yaml::from_str::<TimeshiftRecorderConfig>(r#"
                service-triple: [1, 2, 3]
                ts-file: /path/to/timeshift.m2ts
                data-file: /path/to/timeshift.data
                num-chunks: 100
            "#).unwrap(),
            TimeshiftRecorderConfig {
                service_triple: (1.into(), 2.into(), 3.into()),
                ts_file: "/path/to/timeshift.m2ts".to_string(),
                data_file: "/path/to/timeshift.data".to_string(),
                chunk_size: TimeshiftRecorderConfig::default_chunk_size(),
                num_chunks: 100,
                num_reserves: TimeshiftRecorderConfig::default_num_reserves(),
                priority: TimeshiftRecorderConfig::default_priority(),
            });

        assert_eq!(
            serde_yaml::from_str::<TimeshiftRecorderConfig>(r#"
                service-triple: [1, 2, 3]
                ts-file: /path/to/timeshift.m2ts
                data-file: /path/to/timeshift.data
                chunk-size: 8192
                num-chunks: 100
                num-reserves: 2
                priority: 2
            "#).unwrap(),
            TimeshiftRecorderConfig {
                service_triple: (1.into(), 2.into(), 3.into()),
                ts_file: "/path/to/timeshift.m2ts".to_string(),
                data_file: "/path/to/timeshift.data".to_string(),
                chunk_size: 8192,
                num_chunks: 100,
                num_reserves: 2,
                priority: 2.into(),
            });
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
    fn test_resource_config() {
        assert_eq!(
            serde_yaml::from_str::<ResourceConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<ResourceConfig>(r#"
                strings-yaml: /path/to/strings.yml
            "#).unwrap(),
            ResourceConfig {
                strings_yaml: "/path/to/strings.yml".to_string(),
            });

        let result = serde_yaml::from_str::<ResourceConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_resource_config_validate_existing_strings_yaml() {
        let config = ResourceConfig {
            strings_yaml: "/bin/sh".to_string(),
        };
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_resource_config_validate_non_existing_strings_yaml() {
        let config = ResourceConfig {
            strings_yaml: "/path/to/non-existing".to_string(),
        };
        config.validate();
    }

    #[test]
    fn test_mirakurun_config() {
        assert_eq!(
            serde_yaml::from_str::<MirakurunConfig>("{}").unwrap(),
            Default::default());

        assert_eq!(
            serde_yaml::from_str::<MirakurunConfig>(r#"
                openapi-json: /path/to/json
            "#).unwrap(),
            MirakurunConfig {
                openapi_json: "/path/to/json".to_string(),
            });

        let result = serde_yaml::from_str::<MirakurunConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
    }

    #[test]
    fn test_mirakurun_config_validate_existing_openapi_json() {
        let config = MirakurunConfig {
            openapi_json: "/bin/sh".to_string(),
        };
        config.validate();
    }

    #[test]
    #[should_panic]
    fn test_mirakurun_config_validate_non_existing_openapi_json() {
        let config = MirakurunConfig {
            openapi_json: "/path/to/non-existing".to_string(),
        };
        config.validate();
    }
}
