use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;
use std::time::SystemTime;

use num_cpus;
use serde::Deserialize;
use serde_yaml;

use crate::models::{ChannelType, ServiceId};

pub fn load(config_path: &str) -> Arc<Config> {
    const SERVER_STREAM_TIME_LIMIT_MIN: u64 = 15_000;

    let reader = File::open(config_path)
        .unwrap_or_else(|err| {
            panic!("Failed to open {}: {}", config_path, err);
        });
    let mut config: Config = serde_yaml::from_reader(reader)
        .unwrap_or_else(|err| {
            panic!("Failed to parse {}: {}", config_path, err);
        });
    if config.server.stream_time_limit < SERVER_STREAM_TIME_LIMIT_MIN {
        log::warn!("server.stream_time_limit must be larger than {0}, \
                    reset it to {0}", SERVER_STREAM_TIME_LIMIT_MIN);
        config.server.stream_time_limit = SERVER_STREAM_TIME_LIMIT_MIN;
    }

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
    pub mirakurun: MirakurunConfig,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct EpgConfig {
    #[serde(default)]
    pub cache_dir: Option<String>,
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            addrs: Self::default_addrs(),
            workers: Self::default_workers(),
            stream_max_chunks: Self::default_stream_max_chunks(),
            stream_chunk_size: Self::default_stream_chunk_size(),
            stream_time_limit: Self::default_stream_time_limit(),
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
}

impl TunerConfig {
    fn default_time_limit() -> u64 {
        30 * 1000  // 30s
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
            command: "mirakc-arib filter-service --sid={{sid}}".to_string()
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
            command: "mirakc-arib filter-program --sid={{sid}} --eid={{eid}} \
                      --clock-pcr={{clock_pcr}} --clock-time={{clock_time}} \
                      --end-margin=2000".to_string()
        }
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

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PostFilterConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub content_type: Option<String>,
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
                      {{#sids}} --sids={{.}}{{/sids}}\
                      {{#xsids}} --xsids={{.}}{{/xsids}}".to_string(),
            schedule: "0 31 5 * * * *".to_string(),
        }
    }

    fn default_sync_clocks() -> JobConfig {
        JobConfig {
            command: "mirakc-arib sync-clocks\
                      {{#sids}} --sids={{.}}{{/sids}}\
                      {{#xsids}} --xsids={{.}}{{/xsids}}".to_string(),
            schedule: "0 3 12 * * * *".to_string(),
        }
    }

    fn default_update_schedules() -> JobConfig {
        JobConfig {
            command: "mirakc-arib collect-eits\
                      {{#sids}} --sids={{.}}{{/sids}}\
                      {{#xsids}} --xsids={{.}}{{/xsids}}".to_string(),
            schedule: "0 7,37 * * * * *".to_string(),
        }
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
    pub command: String,
    pub schedule: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct RecorderConfig {
    pub track_airtime_command: String,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        RecorderConfig {
            track_airtime_command: "mirakc-arib track-airtime \
                                    --sid={{sid}} --eid={{eid}}".to_string(),
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
            });

        let result = serde_yaml::from_str::<ServerConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
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
        assert!(serde_yaml::from_str::<JobConfig>("{}").is_err());
        assert!(
            serde_yaml::from_str::<JobConfig>(r#"{"command":""}"#).is_err());
        assert!(
            serde_yaml::from_str::<JobConfig>(r#"{"schedule":""}"#).is_err());

        let result = serde_yaml::from_str::<JobConfig>(r#"
            unknown:
              property: value
        "#);
        assert!(result.is_err());
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
}
