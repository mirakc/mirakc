use std::collections::HashMap;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::epg::EitEvent;
use crate::epg::EitSection;
use crate::epg::EpgProgram;
use crate::error::Error;
use crate::models::EventId;
use crate::models::ProgramQuad;
use crate::models::ServiceTriple;
use crate::models::TunerUser;
use crate::models::TunerUserInfo;
use crate::tuner::TunerStreamStopTrigger;

const INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub struct OnairTracker<T, E> {
    config: Arc<Config>,
    tuner_manager: T,
    epg: E,
    entries: HashMap<ServiceTriple, Entry>,
    timer_token: Option<CancellationToken>,
}

impl<T, E> OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E) -> Self {
        OnairTracker {
            config,
            tuner_manager,
            epg,
            entries: Default::default(),
            timer_token: None,
        }
    }
}

impl<T, E> OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryService>,
{
    fn set_timer(&mut self, ctx: &Context<Self>) {
        if let Some(token) = self.timer_token.take() {
            token.cancel();
        }
        let addr = ctx.address().clone();
        let token = ctx.spawn_task(async move {
            tokio::time::sleep(INTERVAL).await;
            addr.emit(TimerExpired).await;
        });
        self.timer_token = Some(token);
    }
}

// actor

#[async_trait]
impl<T, E> Actor for OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Started");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// add observer

#[derive(Message)]
#[reply("()")]
pub struct AddObserver {
    pub service_triple: ServiceTriple,
    pub name: &'static str,
    pub emitter: Emitter<OnairProgramChanged>,
}

#[async_trait]
impl<T, E> Handler<AddObserver> for OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryService>,
{
    async fn handle(&mut self, msg: AddObserver, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "AddObserver", %msg.service_triple);
        self.add_observer(msg.service_triple, msg.name, msg.emitter)
            .await;
        if self.timer_token.is_none() {
            self.set_timer(ctx);
        }
    }
}

impl<T, E> OnairTracker<T, E> {
    async fn add_observer(
        &mut self,
        service_triple: ServiceTriple,
        name: &'static str,
        emitter: Emitter<OnairProgramChanged>,
    ) {
        match self.entries.get_mut(&service_triple) {
            Some(entry) => {
                if entry.present.is_some() || entry.following.is_some() {
                    let msg = OnairProgramChanged::from((
                        service_triple,
                        entry.present.as_ref(),
                        entry.following.as_ref(),
                    ));
                    emitter.emit(msg).await;
                }
                if let Some(_) = entry.emitters.insert(name, emitter) {
                    tracing::error!(%service_triple, name, "Already added");
                    panic!();
                }
            }
            None => {
                let mut entry = Entry::default();
                entry.emitters.insert(name, emitter);
                self.entries.insert(service_triple, entry);
            }
        }
        tracing::info!(%service_triple, name, "On-air program observer added");
    }
}

// remove observer

#[derive(Message)]
#[reply("()")]
pub struct RemoveObserver {
    pub service_triple: ServiceTriple,
    pub name: &'static str,
}

#[async_trait]
impl<T, E> Handler<RemoveObserver> for OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    async fn handle(&mut self, msg: RemoveObserver, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RemoveObserver", %msg.service_triple, msg.name);
        self.remove_observer(msg.service_triple, msg.name);
        // We don't need stopping the timer.  It will stop if there is no
        // observer added.
    }
}

impl<T, E> OnairTracker<T, E> {
    fn remove_observer(&mut self, service_triple: ServiceTriple, name: &'static str) {
        match self.entries.get_mut(&service_triple) {
            Some(entry) => {
                if let Some(_) = entry.emitters.remove(&name) {
                    tracing::info!(%service_triple, name, "On-air program observer removed");
                } else {
                    tracing::warn!(%service_triple, name, "No observer added");
                }
                // Keep the entry even if there is no observer in it so that we
                // can reuse it if a new observer is added to it just after this
                // call.
            }
            None => {
                tracing::warn!(%service_triple, "No entry created");
            }
        }
    }
}

// timer expired

#[derive(Message)]
struct TimerExpired;

#[async_trait]
impl<T, E> Handler<TimerExpired> for OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryService>,
{
    async fn handle(&mut self, _msg: TimerExpired, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "TimerExpired");
        self.update_onair_programs().await;
        if !self.entries.is_empty() {
            self.set_timer(ctx);
        }
    }
}

impl<T, E> OnairTracker<T, E>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryService>,
{
    async fn update_onair_programs(&mut self) {
        let mut entries = HashMap::with_capacity(self.entries.len());

        for (service_triple, mut entry) in
            std::mem::replace(&mut self.entries, Default::default()).into_iter()
        {
            // Remove entries which have no observer.
            if entry.emitters.is_empty() {
                tracing::info!(%service_triple, "On-air program entry removed");
                continue;
            }

            let updated = Self::update_onair_program(
                &self.config,
                &self.tuner_manager,
                &self.epg,
                service_triple,
                &mut entry,
            )
            .await;

            match updated {
                Ok(true) => {
                    let msg = OnairProgramChanged::from((
                        service_triple,
                        entry.present.as_ref(),
                        entry.following.as_ref(),
                    ));
                    for emitter in entry.emitters.values() {
                        emitter.emit(msg.clone()).await;
                    }
                }
                Ok(false) => {
                    // Nothing to do.
                }
                Err(err) => {
                    tracing::error!(%err, %service_triple, "Failed to update");
                }
            }

            entries.insert(service_triple, entry);
        }

        entries.shrink_to_fit();
        self.entries = entries;
    }

    async fn update_onair_program(
        config: &Config,
        tuner_manager: &T,
        epg: &E,
        service_triple: ServiceTriple,
        entry: &mut Entry,
    ) -> Result<bool, Error> {
        let mut updated = false;

        let service = epg
            .call(crate::epg::QueryService::ByServiceTriple(service_triple))
            .await??;

        let user = TunerUser {
            info: TunerUserInfo::OnairTracker(service_triple),
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .call(crate::tuner::StartStreaming {
                channel: service.channel.clone().into(),
                user,
            })
            .await??;

        let stop_trigger = TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

        let template = mustache::compile_str(&config.onair_trackers.local)?;
        let data = mustache::MapBuilder::new()
            .insert("sid", &service.sid)?
            .build();
        let cmd = template.render_data_to_string(&data)?;
        let mut pipeline = crate::command_util::spawn_pipeline(vec![cmd], stream.id())?;
        let (input, output) = pipeline.take_endpoints().unwrap();
        let handle = tokio::spawn(stream.pipe(input));

        let mut reader = BufReader::new(output);
        let mut json = String::new();
        while reader.read_line(&mut json).await? > 0 {
            let section: EitSection = serde_json::from_str(&json)?;
            match section.section_number {
                0 => {
                    if Self::is_changed(entry.present.as_ref(), &section) {
                        tracing::info!(%service_triple, "EIT[p] updated");
                        updated = true;
                    }
                    entry.present = Some(section);
                }
                1 => {
                    if Self::is_changed(entry.following.as_ref(), &section) {
                        tracing::info!(%service_triple, "EIT[f] updated");
                        updated = true;
                    }
                    entry.following = Some(section);
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

        Ok(updated)
    }

    fn is_changed(old: Option<&EitSection>, new: &EitSection) -> bool {
        match old {
            Some(old) if old.version_number == new.version_number => false,
            _ => true,
        }
    }
}

// on-air program changed

#[derive(Clone, Message)]
pub struct OnairProgramChanged {
    pub service_triple: ServiceTriple,
    pub present: Option<Arc<EpgProgram>>,
    pub following: Option<Arc<EpgProgram>>,
}

// models

#[derive(Debug, Default)]
struct Entry {
    present: Option<EitSection>,
    following: Option<EitSection>,
    emitters: HashMap<&'static str, Emitter<OnairProgramChanged>>,
}

// helpers

impl From<(ServiceTriple, Option<&EitSection>, Option<&EitSection>)> for OnairProgramChanged {
    fn from(data: (ServiceTriple, Option<&EitSection>, Option<&EitSection>)) -> Self {
        OnairProgramChanged {
            service_triple: data.0,
            present: data
                .1
                .map(|section| section.events.get(0))
                .flatten()
                .filter(|event| event.start_time.is_some()) // start_time may be None
                .filter(|event| event.duration.is_some()) // duration may be None
                .map(|event| Arc::new(EpgProgram::from((data.0, event)))),
            following: data
                .2
                .map(|section| section.events.get(0))
                .flatten()
                .filter(|event| event.start_time.is_some()) // start_time may be None
                .filter(|event| event.duration.is_some()) // duration may be None
                .map(|event| Arc::new(EpgProgram::from((data.0, event)))),
        }
    }
}

impl From<(ServiceTriple, &EitEvent)> for EpgProgram {
    fn from(data: (ServiceTriple, &EitEvent)) -> Self {
        let quad = ProgramQuad::from((data.0, EventId::from(data.1.event_id)));
        let mut program = EpgProgram::new(quad);
        program.update(data.1);
        program
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime_ext::Jst;
    use crate::epg::stub::EpgStub;
    use crate::tuner::stub::TunerManagerStub;
    use assert_matches::assert_matches;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_add_observer() {
        let config = config_for_test("");
        let mut tracker = OnairTracker::new(config, TunerManagerStub, EpgStub);
        assert!(tracker.entries.is_empty());

        let triple = (0, 0, 1).into();

        // no entry exists
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test1", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        // entry already exists
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test2", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 2);
        });

        // entry already exists and eit[p] is available
        tracker.entries.entry(triple).and_modify(|entry| {
            entry.present = Some(EitSection {
                original_network_id: 0.into(),
                transport_stream_id: 0.into(),
                service_id: 1.into(),
                table_id: 0,
                section_number: 0,
                last_section_number: 1,
                segment_last_section_number: 0,
                version_number: 0,
                events: vec![],
            });
        });
        let mut observer = MockObserver::new();
        observer.expect_emit().return_once(|_| ());
        tracker
            .add_observer(triple, "test3", Emitter::new(observer))
            .await;

        // entry already exists and eit[f] is available
        tracker.entries.entry(triple).and_modify(|entry| {
            entry.present = None;
            entry.following = Some(EitSection {
                original_network_id: 0.into(),
                transport_stream_id: 0.into(),
                service_id: 1.into(),
                table_id: 0,
                section_number: 0,
                last_section_number: 1,
                segment_last_section_number: 0,
                version_number: 0,
                events: vec![],
            });
        });
        let mut observer = MockObserver::new();
        observer.expect_emit().return_once(|_| ());
        tracker
            .add_observer(triple, "test4", Emitter::new(observer))
            .await;

        // entry already exists and eit[p/f] is available
        tracker.entries.entry(triple).and_modify(|entry| {
            entry.present = Some(EitSection {
                original_network_id: 0.into(),
                transport_stream_id: 0.into(),
                service_id: 1.into(),
                table_id: 0,
                section_number: 0,
                last_section_number: 1,
                segment_last_section_number: 0,
                version_number: 0,
                events: vec![],
            });
        });
        let mut observer = MockObserver::new();
        observer.expect_emit().return_once(|_| ());
        tracker
            .add_observer(triple, "test5", Emitter::new(observer))
            .await;
    }

    #[tokio::test]
    #[should_panic]
    async fn test_add_observer_same_name() {
        let config = config_for_test("");
        let mut tracker = OnairTracker::new(config, TunerManagerStub, EpgStub);
        assert!(tracker.entries.is_empty());

        let triple = (0, 0, 1).into();

        // no entry exists
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);

        // entry already exists
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
    }

    #[tokio::test]
    async fn test_remove_observer() {
        let config = config_for_test("");
        let mut tracker = OnairTracker::new(config, TunerManagerStub, EpgStub);
        let triple = (0, 0, 1).into();
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        tracker.remove_observer(triple, "dummy");
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        tracker.remove_observer((0, 0, 2).into(), "test");
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        tracker.remove_observer(triple, "test");
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert!(entry.emitters.is_empty());
        });

        tracker.remove_observer(triple, "test");
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert!(entry.emitters.is_empty());
        });
    }

    #[tokio::test]
    async fn test_update_onair_programs() {
        let script_file = make_script();
        let cmd = format!("sh {:?}", script_file.path());
        let config = config_for_test(&cmd);
        let mut tracker = OnairTracker::new(config, TunerManagerStub, EpgStub);

        // will fail in QueryService
        let triple = (0, 0, 0).into();
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        // the entry will be updated
        let triple = (0, 0, 1).into();
        let mut observer = MockObserver::new();
        observer.expect_emit().return_once(move |msg| {
            assert_eq!(msg.service_triple, triple);
        });
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 2);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        tracker.update_onair_programs().await;
        assert_eq!(tracker.entries.len(), 2);
        for entry in tracker.entries.values() {
            assert_eq!(entry.emitters.len(), 1);
        }
    }

    #[tokio::test]
    async fn test_update_onair_programs_no_such_service() {
        let script_file = make_script();
        let cmd = format!("sh {:?}", script_file.path());
        let config = config_for_test(&cmd);
        let mut tracker = OnairTracker::new(config, TunerManagerStub, EpgStub);

        let triple = (0, 0, 0).into();
        let mut observer = MockObserver::new();
        observer.expect_emit().never();
        tracker
            .add_observer(triple, "test", Emitter::new(observer))
            .await;
        assert_eq!(tracker.entries.len(), 1);
        assert_matches!(tracker.entries.get(&triple), Some(entry) => {
            assert_eq!(entry.emitters.len(), 1);
        });

        tracker.update_onair_programs().await;
    }

    #[test]
    fn test_from_triple() {
        let triple = (0, 0, 1).into();

        let section = EitSection {
            original_network_id: 0.into(),
            transport_stream_id: 0.into(),
            service_id: 1.into(),
            table_id: 0,
            section_number: 0,
            last_section_number: 1,
            segment_last_section_number: 0,
            version_number: 0,
            events: vec![EitEvent {
                event_id: 0.into(),
                start_time: Some(Jst::now().timestamp_millis()),
                duration: Some(60000),
                scrambled: false,
                descriptors: vec![],
            }],
        };

        let section_no_events = EitSection {
            original_network_id: 0.into(),
            transport_stream_id: 0.into(),
            service_id: 1.into(),
            table_id: 0,
            section_number: 0,
            last_section_number: 1,
            segment_last_section_number: 0,
            version_number: 0,
            events: vec![],
        };

        let msg = OnairProgramChanged::from((triple, None, None));
        assert_eq!(msg.service_triple, triple);
        assert_matches!(msg.present, None);
        assert_matches!(msg.following, None);

        let msg = OnairProgramChanged::from((triple, Some(&section), Some(&section)));
        assert_eq!(msg.service_triple, triple);
        assert_matches!(msg.present, Some(program) => {
            assert_eq!(program.quad, (0, 0, 1, 0).into());
        });
        assert_matches!(msg.following, Some(program) => {
            assert_eq!(program.quad, (0, 0, 1, 0).into());
        });

        let msg =
            OnairProgramChanged::from((triple, Some(&section_no_events), Some(&section_no_events)));
        assert_eq!(msg.service_triple, triple);
        assert_matches!(msg.present, None);
        assert_matches!(msg.following, None);
    }

    fn config_for_test(cmd: &str) -> Arc<Config> {
        let yaml = format!(
            r#"
            onair-trackers:
              local: >-
                {}
            "#,
            cmd,
        );
        Arc::new(serde_yaml::from_str::<Config>(&yaml).unwrap())
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
            segment_last_section_number: 0,
            version_number: 0,
            events: vec![],
        };

        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));
        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));

        section.section_number = 1;
        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));
        script.push(format!(
            "echo '{}'",
            serde_json::to_string(&section).unwrap()
        ));

        let mut tempfile = NamedTempFile::new().unwrap();
        write!(tempfile.as_file_mut(), "{}", script.join("\n")).unwrap();

        tempfile
    }

    mockall::mock! {
        Observer {}

        #[async_trait]
        impl Emit<OnairProgramChanged> for Observer {
            async fn emit(&self, msg: OnairProgramChanged);
        }
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use super::*;

    #[derive(Clone)]
    pub(crate) struct OnairTrackerStub;

    #[async_trait]
    impl Call<AddObserver> for OnairTrackerStub {
        async fn call(
            &self,
            _msg: AddObserver,
        ) -> Result<<AddObserver as Message>::Reply, actlet::Error> {
            Ok(())
        }
    }

    #[async_trait]
    impl Call<RemoveObserver> for OnairTrackerStub {
        async fn call(
            &self,
            _msg: RemoveObserver,
        ) -> Result<<RemoveObserver as Message>::Reply, actlet::Error> {
            Ok(())
        }
    }
}
// </coverage:exclude>
