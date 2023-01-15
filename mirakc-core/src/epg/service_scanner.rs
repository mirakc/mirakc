use std::sync::Arc;

use actlet::prelude::*;
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

#[cfg(test)]
use serde::Serialize;

use crate::command_util;
use crate::config::ChannelConfig;
use crate::config::Config;
use crate::epg::*;
use crate::models::*;
use crate::tuner::*;

pub struct ServiceScanner<T> {
    config: Arc<Config>,
    tuner_manager: T,
}

// TODO: The following implementation has code clones similar to
//       ClockSynchronizer and EitCollector.

impl<T> ServiceScanner<T>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    const LABEL: &'static str = "service-scanner";

    pub fn new(config: Arc<Config>, tuner_manager: T) -> Self {
        ServiceScanner {
            config,
            tuner_manager,
        }
    }

    pub async fn scan_services(self) -> Vec<(EpgChannel, Option<IndexMap<ServiceId, EpgService>>)> {
        tracing::debug!("Scanning services...");

        let command = &self.config.jobs.scan_services.command;
        let mut results = Vec::new();

        for channel in self.config.channels.iter() {
            let result =
                match Self::scan_services_in_channel(channel, command, &self.tuner_manager).await {
                    Ok(services) => {
                        let mut map = IndexMap::new();
                        for service in services.into_iter() {
                            map.insert(service.id, service.clone());
                        }
                        Some(map)
                    }
                    Err(err) => {
                        tracing::warn!("Failed to scan services in {}: {}", channel.name, err);
                        None
                    }
                };
            results.push((channel.clone().into(), result));
        }

        tracing::debug!("Scanned {} channels", self.config.channels.len());

        results
    }

    async fn scan_services_in_channel(
        channel: &ChannelConfig,
        command: &str,
        tuner_manager: &T,
    ) -> anyhow::Result<Vec<EpgService>> {
        tracing::debug!("Scanning services in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job {
                name: Self::LABEL.to_string(),
            },
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .call(StartStreaming {
                channel: channel.clone().into(),
                user,
                stream_id: None,
            })
            .await??;

        let stop_trigger = TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

        let template = mustache::compile_str(command)?;
        let data = mustache::MapBuilder::new()
            .insert("sids", &channel.services)?
            .insert("xsids", &channel.excluded_services)?
            .build();
        let cmd = template.render_data_to_string(&data)?;

        let mut pipeline = command_util::spawn_pipeline(vec![cmd], stream.id(), "epg.service")?;

        let (input, mut output) = pipeline.take_endpoints().unwrap();

        let handle = tokio::spawn(stream.pipe(input));

        let mut buf = Vec::new();
        output.read_to_end(&mut buf).await?;

        drop(stop_trigger);

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(pipeline);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        anyhow::ensure!(buf.len() > 0, "No service, maybe out of service");

        let services: Vec<TsService> = serde_json::from_slice(&buf)?;
        tracing::debug!("Found {} services in {}", services.len(), channel.name);

        Ok(services
            .into_iter()
            .map(|sv| EpgService::from((channel, &sv)))
            .collect())
    }
}

#[derive(Clone, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(rename_all = "camelCase")]
struct TsService {
    nid: Nid,
    #[allow(dead_code)]
    tsid: Tsid,
    sid: Sid,
    #[serde(rename = "type")]
    service_type: u16,
    #[serde(default)]
    logo_id: i16,
    #[serde(default)]
    remote_control_key_id: u16,
    name: String,
}

impl From<(&ChannelConfig, &TsService)> for EpgService {
    fn from((ch, sv): (&ChannelConfig, &TsService)) -> Self {
        EpgService {
            id: ServiceId::new(sv.nid, sv.sid),
            service_type: sv.service_type,
            logo_id: sv.logo_id,
            remote_control_key_id: sv.remote_control_key_id,
            name: sv.name.clone(),
            channel: ch.clone().into(),
        }
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuner::stub::TunerManagerStub;

    #[tokio::test]
    async fn test_scan_services_in_channel() {
        let stub = TunerManagerStub;

        let expected = vec![TsService {
            nid: 1.into(),
            tsid: 2.into(),
            sid: 3.into(),
            service_type: 4,
            logo_id: 0,
            remote_control_key_id: 1,
            name: "service".to_string(),
        }];

        let config_yml = format!(
            r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              scan-services:
                command: echo '{}'
        "#,
            serde_json::to_string(&expected).unwrap()
        );

        let config = Arc::new(serde_yaml::from_str::<Config>(&config_yml).unwrap());

        let scan = ServiceScanner::new(config, stub.clone());
        let results = scan.scan_services().await;
        assert!(results[0].1.is_some());
        assert_eq!(results[0].1.as_ref().unwrap().len(), 1);

        // Emulate out of services by using `false`
        let config = Arc::new(
            serde_yaml::from_str::<Config>(
                r#"
            channels:
              - name: channel
                type: GR
                channel: '0'
            jobs:
              scan-services:
                command: false
        "#,
            )
            .unwrap(),
        );
        let scan = ServiceScanner::new(config, stub.clone());
        let results = scan.scan_services().await;
        assert!(results[0].1.is_none());
    }
}
// </coverage:exclude>
