use std::fs::File;
use std::io;

use log;
use num_cpus;
use serde::Deserialize;
use serde_yaml;

use crate::error::Error;
use crate::models::{ChannelType, ServiceId};

// result

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
    #[serde(default)]
    pub tuners: Vec<TunerConfig>,
    pub tools: ToolsConfig,
    pub epg_cache_dir: String,
    #[serde(default)]
    pub pcr_time_correction: i64,
}

impl Config {
    #[inline]
    pub fn load(config_yml: &str) -> Result<Self, Error> {
        log::info!("Load {}", config_yml);
        let reader = File::open(config_yml)?;
        Config::from_reader(reader)
    }

    #[inline]
    fn from_reader<T: io::Read>(reader: T) -> Result<Self, Error> {
        let config: Config = serde_yaml::from_reader(reader)?;
        Ok(config)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct ServerConfig {
    #[serde(default = "default_server_address")]
    pub address: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
    #[serde(default = "default_server_workers")]
    pub workers: usize,
}

fn default_server_address() -> String { "localhost".to_string() }
fn default_server_port() -> u16 { 40772 }
fn default_server_workers() -> usize { num_cpus::get() }

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            address: default_server_address(),
            port: default_server_port(),
            workers: default_server_workers(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct ChannelConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    pub channel: String,
    #[serde(default)]
    pub excluded_services: Vec<ServiceId>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TunerConfig {
    pub name: String,
    #[serde(rename = "types")]
    pub channel_types: Vec<ChannelType>,
    pub command: String,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct ToolsConfig {
    pub scan_services: String,
    pub sync_clock: String,
    pub collect_eits: String,
    pub filter_service: String,
    pub filter_program: String,
    #[serde(default)]
    pub preprocess: String,
    #[serde(default)]
    pub postprocess: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config() {
        assert!(serde_yaml::from_str::<Config>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<Config>(r#"
                tools:
                  scan-services: scan-services
                  sync-clock: sync-clock
                  collect-eits: collect-eits
                  filter-service: filter-service
                  filter-program: filter-program
                epg-cache-dir: /path/to/epg
            "#).unwrap(),
            Config {
                server: ServerConfig::default(),
                channels: vec![],
                tuners: vec![],
                tools: ToolsConfig {
                    scan_services: "scan-services".to_string(),
                    sync_clock: "sync-clock".to_string(),
                    collect_eits: "collect-eits".to_string(),
                    filter_service: "filter-service".to_string(),
                    filter_program: "filter-program".to_string(),
                    preprocess: "".to_string(),
                    postprocess: "".to_string(),
                },
                epg_cache_dir: "/path/to/epg".to_string(),
                pcr_time_correction: 0,
            });
    }

    #[test]
    fn test_config_from_reader() {
        let result = Config::from_reader(r#"
            tools:
              scan-services: scan-services
              sync-clock: sync-clock
              collect-eits: collect-eits
              filter-service: filter-service
              filter-program: filter-program
            epg-cache-dir: /path/to/epg
        "#.as_bytes());
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            Config {
                server: ServerConfig::default(),
                channels: vec![],
                tuners: vec![],
                tools: ToolsConfig {
                    scan_services: "scan-services".to_string(),
                    sync_clock: "sync-clock".to_string(),
                    collect_eits: "collect-eits".to_string(),
                    filter_service: "filter-service".to_string(),
                    filter_program: "filter-program".to_string(),
                    preprocess: "".to_string(),
                    postprocess: "".to_string(),
                },
                epg_cache_dir: "/path/to/epg".to_string(),
                pcr_time_correction: 0,
            });
    }

    #[test]
    fn test_server_config() {
        assert_eq!(
            serde_yaml::from_str::<ServerConfig>("{}").unwrap(),
            ServerConfig::default());

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                address: '0.0.0.0'
            "#).unwrap(),
            ServerConfig {
                address: "0.0.0.0".to_string(),
                port: default_server_port(),
                workers: default_server_workers(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                port: 11111
            "#).unwrap(),
            ServerConfig {
                address: default_server_address(),
                port: 11111,
                workers: default_server_workers(),
            });

        assert_eq!(
            serde_yaml::from_str::<ServerConfig>(r#"
                workers: 2
            "#).unwrap(),
            ServerConfig {
                address: default_server_address(),
                port: default_server_port(),
                workers: 2,
            });
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
                excluded_services: vec![100.into()],
                disabled: false,
            });

        assert!(
            serde_yaml::from_str::<ChannelConfig>(r#"
                name: x
                type: WOWOW
                channel: y
            "#).is_err());
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
                disabled: true,
            });

        assert!(
            serde_yaml::from_str::<TunerConfig>(r#"
                name: x
                types: [WOWOW]
                command: open tuner
            "#).is_err());
    }

    #[test]
    fn test_tools_config() {
        assert!(serde_yaml::from_str::<ToolsConfig>("{}").is_err());

        assert_eq!(
            serde_yaml::from_str::<ToolsConfig>(r#"
                scan-services: scan-services
                sync-clock: sync-clock
                collect-eits: collect-eits
                filter-service: filter-service
                filter-program: filter-program
            "#).unwrap(),
            ToolsConfig {
                scan_services: "scan-services".to_string(),
                sync_clock: "sync-clock".to_string(),
                collect_eits: "collect-eits".to_string(),
                filter_service: "filter-service".to_string(),
                filter_program: "filter-program".to_string(),
                preprocess: "".to_string(),
                postprocess: "".to_string(),
            });

        assert_eq!(
            serde_yaml::from_str::<ToolsConfig>(r#"
                scan-services: scan-services
                sync-clock: sync-clock
                collect-eits: collect-eits
                filter-service: filter-service
                filter-program: filter-program
                preprocess: preprocess
                postprocess: postprocess
            "#).unwrap(),
            ToolsConfig {
                scan_services: "scan-services".to_string(),
                sync_clock: "sync-clock".to_string(),
                collect_eits: "collect-eits".to_string(),
                filter_service: "filter-service".to_string(),
                filter_program: "filter-program".to_string(),
                preprocess: "preprocess".to_string(),
                postprocess: "postprocess".to_string(),
            });
    }
}
