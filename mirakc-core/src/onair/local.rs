use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use actlet::prelude::*;
use chrono_jst::Jst;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tracing::Instrument;

use crate::config::LocalOnairProgramTrackerConfig;
use crate::epg::EitSection;
use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::epg::QueryServices;
use crate::error::Error;
use crate::models::ProgramId;
use crate::models::ServiceId;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;

use super::OnairProgramChanged;
use super::TrackerStopped;

pub struct LocalTracker<T, E> {
    name: String,
    config: Arc<LocalOnairProgramTrackerConfig>,
    tuner_manager: T,
    epg: E,
    changed_emitter: Emitter<OnairProgramChanged>,
    stopped_emitter: Option<Emitter<TrackerStopped>>,
    entries: HashMap<ServiceId, Entry>,
}

impl<T, E> LocalTracker<T, E> {
    pub fn new(
        name: String,
        config: Arc<LocalOnairProgramTrackerConfig>,
        tuner_manager: T,
        epg: E,
        changed_emitter: Emitter<OnairProgramChanged>,
        stopped_emitter: Option<Emitter<TrackerStopped>>,
    ) -> Self {
        LocalTracker {
            name,
            config,
            tuner_manager,
            epg,
            changed_emitter,
            stopped_emitter,
            entries: Default::default(),
        }
    }
}

// actor

#[async_trait]
impl<T, E> Actor for LocalTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!(tracker.name = self.name, "Started");
        self.set_timer(ctx);
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(tracker.name = self.name, "Stopping...");
        if let Some(ref stopped) = self.stopped_emitter {
            let msg = TrackerStopped {
                tracker: self.name.clone(),
            };
            stopped.emit(msg).await;
        }
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(tracker.name = self.name, "Stopped");
    }
}

impl<T, E> LocalTracker<T, E> {
    fn set_timer<C>(&mut self, ctx: &C)
    where
        C: Spawn + EmitterFactory<UpdateOnairPrograms>,
    {
        // Get the current time before getting `datetime`
        // so that `datetime - now` always returns a non-negative duration.
        let now = Jst::now();
        let datetime = cron::Schedule::from_str("1 * * * * * *")
            .unwrap()
            .upcoming(Jst)
            .take(1)
            .nth(0)
            .unwrap();
        let interval = (datetime - now).to_std().unwrap();
        let emitter = ctx.emitter();
        ctx.spawn_task(async move {
            tokio::time::sleep(interval).await;
            emitter.emit(UpdateOnairPrograms).await;
        });
    }
}

// update on-air programs

#[derive(Message)]
struct UpdateOnairPrograms;

#[async_trait]
impl<T, E> Handler<UpdateOnairPrograms> for LocalTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
{
    async fn handle(&mut self, _msg: UpdateOnairPrograms, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateOnairPrograms");
        let now = std::time::Instant::now();
        if self.update_onair_programs().await {
            let elapsed = now.elapsed();
            if elapsed >= std::time::Duration::from_secs(60) {
                tracing::warn!(
                    tracker.name = self.name,
                    elapsed = %humantime::format_duration(elapsed),
                    "Didn't finish within 60s",
                );
            }
            self.set_timer(ctx);
        } else {
            ctx.stop();
        }
    }
}

impl<T, E> LocalTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: TriggerFactory<StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryServices>,
{
    async fn update_onair_programs(&mut self) -> bool {
        // Clone the config in order to avoid compile errors caused by the borrow checker.
        let config = self.config.clone();

        let services = match self.epg.call(QueryServices).await {
            Ok(services) => services,
            Err(err) => {
                tracing::error!(%err, tracker.name = self.name, "Failed to get services, Epg dead?");
                return true; // continue
            }
        };

        let iter = services
            .iter()
            .filter(|(_, service)| config.matches(service));
        for (service_id, service) in iter {
            let result = self.update_onair_program(service).await;
            match (result, self.config.stream_id.is_some()) {
                (Ok(_), _) => {
                    // Finished successfully, process next.
                }
                (Err(Error::TunerUnavailable), true) => {
                    tracing::error!(tracker.name = self.name, "Stop tracking");
                    return false; // stop
                }
                (Err(err), _) => {
                    tracing::error!(
                        %err,
                        tracker.name = self.name,
                        service.id = %service_id,
                        "Failed to update on-air program"
                    );
                    // Ignore the error, process next.
                }
            }
        }

        true // continue
    }

    async fn update_onair_program(&mut self, service: &EpgService) -> Result<(), Error> {
        let service_id = service.id;

        let user = TunerUser {
            info: TunerUserInfo::OnairProgramTracker(self.name.clone()),
            priority: (-1).into(),
        };

        let stream = self
            .tuner_manager
            .call(StartStreaming {
                channel: service.channel.clone(),
                user,
                stream_id: self.config.stream_id,
            })
            .await??;

        let msg = StopStreaming { id: stream.id() };
        let stop_trigger = self.tuner_manager.trigger(msg);

        let template = mustache::compile_str(&self.config.command)?;
        let data = mustache::MapBuilder::new()
            .insert("sid", &service.sid())?
            .build();
        let cmd = template.render_data_to_string(&data)?;
        let mut pipeline = crate::command_util::spawn_pipeline(vec![cmd], stream.id(), "onair")?;
        let (input, output) = pipeline.take_endpoints();
        let handle = tokio::spawn(stream.pipe(input).in_current_span());

        let mut changed = false;
        let mut reader = BufReader::new(output);
        let mut json = String::new();
        while reader.read_line(&mut json).await? > 0 {
            let section: EitSection = serde_json::from_str(&json)?;
            match section.section_number {
                0 => {
                    let entry = self.entries.entry(service_id).or_default();
                    if section.is_updated(&entry.current) {
                        tracing::debug!(tracker.name = self.name, service.id = %service_id, "Update current program");
                        entry.current = Some(section);
                        changed = true;
                    }
                }
                1 => {
                    let entry = self.entries.entry(service_id).or_default();
                    if section.is_updated(&entry.next) {
                        tracing::debug!(tracker.name = self.name, service.id = %service_id, "Update next program");
                        entry.next = Some(section);
                        changed = true;
                    }
                }
                _ => (),
            }
            json.clear();
        }

        drop(stop_trigger);

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(pipeline);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        if changed {
            let entry = self.entries.get(&service_id).unwrap();
            let msg = OnairProgramChanged {
                service_id,
                current: entry
                    .current
                    .as_ref()
                    .and_then(|section| section.extract_program()),
                next: entry
                    .next
                    .as_ref()
                    .and_then(|section| section.extract_program()),
            };
            self.changed_emitter.emit(msg).await;
        }

        Ok(())
    }
}

// helpers

impl LocalOnairProgramTrackerConfig {
    pub fn matches(&self, service: &EpgService) -> bool {
        if !self.channel_types.contains(&service.channel.channel_type) {
            return false;
        }
        let service_id = service.id;
        if !self.services.is_empty() && !self.services.contains(&service_id) {
            return false;
        }
        if !self.excluded_services.is_empty() && self.excluded_services.contains(&service_id) {
            return false;
        }
        true
    }
}

impl EitSection {
    fn is_updated(&self, section: &Option<Self>) -> bool {
        !matches!(section, Some(section) if self.version_number == section.version_number)
    }

    fn extract_program(&self) -> Option<Arc<EpgProgram>> {
        self.events.first().map(|event| {
            let id = ProgramId::new(self.original_network_id, self.service_id, event.event_id);
            let mut program = EpgProgram::new(id);
            program.update(event);
            Arc::new(program)
        })
    }
}

// models

#[derive(Default)]
struct Entry {
    current: Option<EitSection>,
    next: Option<EitSection>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LocalOnairProgramTrackerUses;
    use crate::epg::stub::EpgStub;
    use crate::epg::EitEvent;
    use crate::epg::EpgChannel;
    use crate::models::ChannelType;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use maplit::hashset;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use test_log::test;

    macro_rules! service {
        ($id:expr, $channel_type:expr) => {
            EpgService {
                id: $id.into(),
                service_type: 0,
                logo_id: 0,
                remote_control_key_id: 0,
                name: Default::default(),
                channel: EpgChannel {
                    name: Default::default(),
                    channel_type: $channel_type,
                    channel: Default::default(),
                    extra_args: Default::default(),
                    services: Default::default(),
                    excluded_services: Default::default(),
                },
            }
        };
    }

    #[test(tokio::test)]
    async fn test_update_onair_program() {
        let script_file = make_script();
        let command = format!("sh {}", script_file.path().display());
        let config = Arc::new(LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![],
            excluded_services: hashset![],
            command,
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        });
        let mut changed_mock = MockChangedEmitter::new();
        changed_mock.expect_emit().times(1).returning(|msg| {
            assert_matches!(msg.current, Some(program) => {
                assert_eq!(program.id, (0, 1, 4).into());
            });
            assert_matches!(msg.next, Some(program) => {
                assert_eq!(program.id, (0, 1, 4).into());
            });
        });
        let mut tracker = LocalTracker::new(
            "".to_string(),
            config,
            TunerManagerStub::default(),
            EpgStub,
            Emitter::new(changed_mock),
            None,
        );

        let service01 = service!((0, 1), ChannelType::GR);
        let result = tracker.update_onair_program(&service01).await;
        assert_matches!(result, Ok(()));
        let result = tracker.update_onair_program(&service01).await;
        assert_matches!(result, Ok(()));
    }

    #[test]
    fn test_config_matches() {
        let gr12 = service!((1, 2), ChannelType::GR);
        let gr13 = service!((1, 3), ChannelType::GR);
        let bs12 = service!((1, 2), ChannelType::BS);
        let bs13 = service!((1, 3), ChannelType::BS);

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![],
            excluded_services: hashset![],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(config.matches(&gr12));
        assert!(config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(!config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR, ChannelType::BS],
            services: hashset![],
            excluded_services: hashset![],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(config.matches(&gr12));
        assert!(config.matches(&gr13));
        assert!(config.matches(&bs12));
        assert!(config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![(1, 2).into()],
            excluded_services: hashset![],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(config.matches(&gr12));
        assert!(!config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(!config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR, ChannelType::BS],
            services: hashset![(1, 2).into()],
            excluded_services: hashset![],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(config.matches(&gr12));
        assert!(!config.matches(&gr13));
        assert!(config.matches(&bs12));
        assert!(!config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![],
            excluded_services: hashset![(1, 2).into()],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(!config.matches(&gr12));
        assert!(config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(!config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR, ChannelType::BS],
            services: hashset![],
            excluded_services: hashset![(1, 2).into()],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(!config.matches(&gr12));
        assert!(config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR],
            services: hashset![(1, 2).into()],
            excluded_services: hashset![(1, 2).into()],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(!config.matches(&gr12));
        assert!(!config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(!config.matches(&bs13));

        let config = LocalOnairProgramTrackerConfig {
            channel_types: hashset![ChannelType::GR, ChannelType::BS],
            services: hashset![(1, 2).into()],
            excluded_services: hashset![(1, 2).into()],
            command: "".to_string(),
            uses: LocalOnairProgramTrackerUses {
                tuner: "".to_string(),
            },
            stream_id: None,
        };
        assert!(!config.matches(&gr12));
        assert!(!config.matches(&gr13));
        assert!(!config.matches(&bs12));
        assert!(!config.matches(&bs13));
    }

    #[test]
    fn test_section_extract_program() {
        let section = EitSection {
            original_network_id: 1.into(),
            transport_stream_id: 2.into(),
            service_id: 3.into(),
            table_id: 0,
            section_number: 0,
            last_section_number: 1,
            segment_last_section_number: 0,
            version_number: 0,
            events: vec![EitEvent {
                event_id: 4.into(),
                start_time: Some(Jst::now().timestamp_millis()),
                duration: Some(60000),
                scrambled: false,
                descriptors: vec![],
            }],
        };
        assert_matches!(section.extract_program(), Some(program) => {
            assert_eq!(program.id, (1, 3, 4).into());
        });

        let section = EitSection {
            original_network_id: 1.into(),
            transport_stream_id: 2.into(),
            service_id: 3.into(),
            table_id: 0,
            section_number: 0,
            last_section_number: 1,
            segment_last_section_number: 0,
            version_number: 0,
            events: vec![],
        };
        assert_matches!(section.extract_program(), None);
    }

    fn make_script() -> NamedTempFile {
        let mut script = vec![];

        let mut section = EitSection {
            original_network_id: 0.into(),
            transport_stream_id: 0.into(),
            service_id: 1.into(),
            table_id: 0,
            section_number: 0,
            last_section_number: 1,
            segment_last_section_number: 1,
            version_number: 0,
            events: vec![EitEvent {
                event_id: 4.into(),
                start_time: Some(Jst::now().timestamp_millis()),
                duration: Some(60000),
                scrambled: false,
                descriptors: vec![],
            }],
        };

        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));

        section.section_number = 1;
        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));

        let mut tempfile = NamedTempFile::new().unwrap();
        write!(tempfile.as_file_mut(), "{}", script.join("\n")).unwrap();

        tempfile
    }

    mockall::mock! {
        ChangedEmitter {}

        #[async_trait]
        impl Emit<OnairProgramChanged> for ChangedEmitter {
            async fn emit(&self, msg: OnairProgramChanged);
        }
    }
}
