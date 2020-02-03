use std::collections::HashMap;
use std::sync::Arc;

use actix::prelude::*;
use log;
use serde_json;
use tokio::prelude::*;
use tokio::io::BufReader;

use crate::config::Config;
use crate::error::Error;
use crate::epg::{self, *};
use crate::models::*;
use crate::tuner;
use crate::command_util;

pub fn start(config: Arc<Config>) {
    let addr = EitFeeder::new(config).start();
    actix::registry::SystemRegistry::set(addr);
}

pub async fn update_schedules() -> Result<(), Error> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Ok(())
        } else {
            EitFeeder::from_registry().send(UpdateSchedulesMessage).await?
        }
    }
}

struct EitFeeder {
    config: Arc<Config>,
}

impl EitFeeder {
    fn new(config: Arc<Config>) -> Self {
        EitFeeder { config }
    }

    async fn update_schedules(
        command: String
    ) -> Result<(), Error> {
        let services = epg::query_services().await?;

        let mut map: HashMap<NetworkId, EpgChannel> = HashMap::new();
        for sv in services.iter() {
            map.entry(sv.nid).and_modify(|ch| {
                ch.excluded_services.extend(&sv.channel.excluded_services);
            }).or_insert(sv.channel.clone());
        }
        let channels = map.values().cloned().collect();

        EitCollector::new(command, channels)
            .collect_schedules().await
    }
}

impl Actor for EitFeeder {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        log::debug!("Started");
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::debug!("Stopped");
    }
}

impl Supervised for EitFeeder {}
impl SystemService for EitFeeder {}

impl Default for EitFeeder {
    fn default() -> Self {
        unreachable!();
    }
}

// update schedules

struct UpdateSchedulesMessage;

impl Message for UpdateSchedulesMessage {
    type Result = Result<(), Error>;
}

impl Handler<UpdateSchedulesMessage> for EitFeeder {
    type Result = Response<(), Error>;

    fn handle(
        &mut self,
        _: UpdateSchedulesMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("Handle UpdateSchedulesMessage");
        let fut = Box::pin(Self::update_schedules(
            self.config.jobs.update_schedules.command.clone()));
        Response::fut(fut)
    }
}

// collector

pub struct EitCollector {
    command: String,
    channels: Vec<EpgChannel>,
}

// TODO: The following implementation has code clones similar to
//       ClockSynchronizer and ServiceScanner.

impl EitCollector {
    const LABEL: &'static str = "eit-collector";
    const UPDATE_CHUNK_SIZE: usize = 32;

    pub fn new(
        command: String,
        channels: Vec<EpgChannel>
    ) -> Self {
        EitCollector { command, channels }
    }

    pub async fn collect_schedules(
        self
    ) -> Result<(), Error> {
        log::info!("Collecting EIT sections...");
        let mut num_sections = 0;
        for channel in self.channels.iter() {
            num_sections +=
                Self::collect_eits_in_channel(&channel, &self.command).await?;
        }
        epg::flush_schedules();
        log::info!("Collected {} EIT sections", num_sections);
        Ok(())
    }

    async fn collect_eits_in_channel(
        channel: &EpgChannel,
        command: &str,
    ) -> Result<usize, Error> {
        log::debug!("Collecting EIT sections in {}...", channel.name);

        let user = TunerUser {
            info: TunerUserInfo::Job { name: Self::LABEL.to_string() },
            priority: -1,
        };

        let stream = tuner::start_streaming(
            channel.channel_type, channel.channel.clone(), user).await?;

        let (input, output) = command_util::spawn_pipeline(
            vec![command.to_string()], stream.id())?;

        let handle = tokio::spawn(stream.pipe(input));

        let mut reader = BufReader::new(output);
        let mut json = String::new();
        let mut num_sections = 0;
        let mut sections = Vec::with_capacity(Self::UPDATE_CHUNK_SIZE);
        while reader.read_line(&mut json).await? > 0 {
            let eit = serde_json::from_str::<EitSection>(&json)?;
            sections.push(eit);
            if sections.len() == Self::UPDATE_CHUNK_SIZE {
                epg::update_schedules(sections);
                sections = Vec::with_capacity(32);
            }
            json.clear();
            num_sections += 1;
        }
        if !sections.is_empty() {
            epg::update_schedules(sections);
        }

        // Explicitly dropping the output of the pipeline is needed.  The output
        // holds the child processes and it kills them when dropped.
        drop(reader);

        // Wait for the task so that the tuner is released before a request for
        // streaming in the next iteration.
        let _ = handle.await;

        log::debug!("Collected {} EIT sections in {}",
                    num_sections, channel.name);

        Ok(num_sections)
    }
}
