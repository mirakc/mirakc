use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use actix::prelude::*;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;

use crate::command_util;
use crate::config::Config;
use crate::epg::*;
use crate::error::Error;
use crate::models::*;
use crate::tuner::*;

pub fn start(
    config: Arc<Config>,
    tuner_manager: Addr<TunerManager>,
    epg: Addr<Epg>,
) -> Addr<EitFeeder> {
    EitFeeder::new(config, tuner_manager, epg).start()
}

pub struct EitFeeder {
    config: Arc<Config>,
    tuner_manager: Addr<TunerManager>,
    epg: Addr<Epg>,
}

impl EitFeeder {
    pub fn new(config: Arc<Config>, tuner_manager: Addr<TunerManager>, epg: Addr<Epg>) -> Self {
        EitFeeder {
            config,
            tuner_manager,
            epg,
        }
    }

    async fn feed_eit_sections(
        command: String,
        tuner_manager: Addr<TunerManager>,
        epg: Addr<Epg>,
    ) -> Result<(), Error> {
        let services = epg.send(QueryServicesMessage).await?;

        let mut map: HashMap<String, EpgChannel> = HashMap::new();
        for sv in services.values() {
            let chid = format!("{}/{}", sv.channel.channel_type, sv.channel.channel);
            map.entry(chid)
                .and_modify(|ch| ch.services.push(sv.sid))
                .or_insert(EpgChannel {
                    name: sv.channel.name.clone(),
                    channel_type: sv.channel.channel_type,
                    channel: sv.channel.channel.clone(),
                    extra_args: sv.channel.extra_args.clone(),
                    services: vec![sv.sid],
                    excluded_services: vec![],
                });
        }
        let channels = map.values().cloned().collect();

        EitCollector::new(command, channels, tuner_manager, epg)
            .collect_schedules()
            .await
    }
}

impl Actor for EitFeeder {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        tracing::debug!("Started");
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        tracing::debug!("Stopped");
    }
}

// feed eit sections

#[derive(Message)]
#[rtype(result = "Result<(), Error>")]
pub struct FeedEitSectionsMessage;

impl fmt::Display for FeedEitSectionsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeedEitSections")
    }
}

impl Handler<FeedEitSectionsMessage> for EitFeeder {
    type Result = ResponseFuture<Result<(), Error>>;

    fn handle(&mut self, msg: FeedEitSectionsMessage, _: &mut Self::Context) -> Self::Result {
        tracing::debug!("{}", msg);
        Box::pin(Self::feed_eit_sections(
            self.config.jobs.update_schedules.command.clone(),
            self.tuner_manager.clone(),
            self.epg.clone(),
        ))
    }
}

// collector

pub struct EitCollector {
    command: String,
    channels: Vec<EpgChannel>,
    tuner_manager: Addr<TunerManager>,
    epg: Addr<Epg>,
}

// TODO: The following implementation has code clones similar to
//       ClockSynchronizer and ServiceScanner.

impl EitCollector {
    const LABEL: &'static str = "eit-collector";

    pub fn new(
        command: String,
        channels: Vec<EpgChannel>,
        tuner_manager: Addr<TunerManager>,
        epg: Addr<Epg>,
    ) -> Self {
        EitCollector {
            command,
            channels,
            tuner_manager,
            epg,
        }
    }

    pub async fn collect_schedules(self) -> Result<(), Error> {
        tracing::info!("Collecting EIT sections...");
        let mut num_sections = 0;
        for channel in self.channels.iter() {
            num_sections += Self::collect_eits_in_channel(
                &channel,
                &self.command,
                &self.tuner_manager,
                &self.epg,
            )
            .await?;
        }
        tracing::info!("Collected {} EIT sections", num_sections);
        Ok(())
    }

    async fn collect_eits_in_channel(
        channel: &EpgChannel,
        command: &str,
        tuner_manager: &Addr<TunerManager>,
        epg: &Addr<Epg>,
    ) -> Result<usize, Error> {
        tracing::debug!("Collecting EIT sections in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job {
                name: Self::LABEL.to_string(),
            },
            priority: (-1).into(),
        };

        let stream = tuner_manager
            .send(StartStreamingMessage {
                channel: channel.clone(),
                user,
            })
            .await??;

        let stop_trigger =
            TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().recipient());

        let template = mustache::compile_str(command)?;
        let data = mustache::MapBuilder::new()
            .insert("sids", &channel.services)?
            .insert("xsids", &channel.excluded_services)?
            .build();
        let cmd = template.render_data_to_string(&data)?;

        let mut pipeline = command_util::spawn_pipeline(vec![cmd], stream.id())?;

        let (input, output) = pipeline.take_endpoints().unwrap();

        let handle = tokio::spawn(stream.pipe(input));

        let mut reader = BufReader::new(output);
        let mut json = String::new();
        let mut num_sections = 0;
        let mut triples = HashSet::new();
        while reader.read_line(&mut json).await? > 0 {
            let section = serde_json::from_str::<EitSection>(&json)?;
            if section.is_valid() {
                let triple = section.service_triple();
                if !triples.contains(&triple) {
                    triples.insert(triple);
                    epg.do_send(PrepareScheduleMessage {
                        service_triple: triple,
                    });
                }
                epg.do_send(UpdateScheduleMessage { section });
                json.clear();
                num_sections += 1;
            } else {
                tracing::warn!(section.table_id, "Invalid table_id");
            }
        }

        drop(stop_trigger);

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(pipeline);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        for triple in triples.iter() {
            epg.do_send(FlushScheduleMessage {
                service_triple: triple.clone(),
            });
        }

        tracing::debug!(
            "Collected {} EIT sections in {}",
            num_sections,
            channel.name
        );

        Ok(num_sections)
    }
}
