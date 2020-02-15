use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use actix::prelude::*;
use chrono::DateTime;
use cron;
use humantime;
use log;
use tokio::sync::Semaphore;

use crate::clock_synchronizer::ClockSynchronizer;
use crate::config::Config;
use crate::datetime_ext::*;
use crate::eit_feeder;
use crate::epg::{self, *};
use crate::error::Error;
use crate::service_scanner::ServiceScanner;

// TODO: Refactoring
//
// It's better to implement each job as an actor.  However, it's NOT easy to use
// async/await in the Actor implementation.  See Handler<StartStreamingMessage>
// in tuner.rs.

pub fn start(config: Arc<Config>) {
    let addr = JobManager::new(config).start();
    actix::registry::SystemRegistry::set(addr);
}

pub fn invoke_scan_services() {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
        } else {
            JobManager::from_registry().do_send(InvokeScanServicesMessage);
        }
    }
}

pub fn invoke_sync_clocks() {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
        } else {
            JobManager::from_registry().do_send(InvokeSyncClocksMessage);
        }
    }
}

pub fn invoke_update_schedules() {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
        } else {
            JobManager::from_registry().do_send(InvokeUpdateSchedulesMessage);
        }
    }
}

struct Job {
    kind: JobKind,
    semaphore: Arc<Semaphore>,
}

impl Job {
    fn new(kind: JobKind, semaphore: Arc<Semaphore>) -> Self {
        Job { kind, semaphore }
    }

    async fn perform<T, F>(self, fut: F) -> Result<T, Error>
    where
        F: Future<Output = Result<T, Error>>,
    {
        log::debug!("{}: acquiring semaphore...", self.kind);
        let _permit = self.semaphore.acquire().await;
        log::info!("{}: performing...", self.kind);
        let now = Instant::now();
        let result = fut.await;
        let elapsed = now.elapsed();
        match result {
            Ok(_) => log::info!("{}: Done successfully, {} elapsed",
                                self.kind, humantime::format_duration(elapsed)),
            Err(ref err) => log::error!("{}: Failed: {}", self.kind, err),
        }
        result
    }
}

enum JobKind {
    ScanServices,
    SyncClocks,
    UpdateSchedules,
}

impl JobKind {
    fn create(self, semaphore: Arc<Semaphore>) -> Job {
        Job::new(self, semaphore)
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use JobKind::*;
        match *self {
            ScanServices => write!(f, "scan-services"),
            SyncClocks => write!(f, "sync-clocks"),
            UpdateSchedules => write!(f, "update-schedules"),
        }
    }
}

struct JobManager {
    config: Arc<Config>,
    semaphore: Arc<Semaphore>,  // job concurrency
    scanning_services: bool,
    synchronizing_clocks: bool,
    updating_schedules: bool,
}

impl JobManager {
    fn new(config: Arc<Config>) -> Self {
        JobManager {
            config,
            semaphore: Arc::new(Semaphore::new(1)),
            scanning_services: false,
            synchronizing_clocks: false,
            updating_schedules: false,
        }
    }

    fn calc_next_scheduled_datetime(&self, schedule: &str) -> DateTime<Jst> {
        cron::Schedule::from_str(schedule)
            .unwrap()
            .upcoming(Jst)
            .take(1)
            .nth(0)
            .unwrap()
    }

    fn scan_services(&mut self, ctx: &mut Context<Self>) {
        self.invoke_scan_services(ctx);
        self.schedule_scan_services(ctx);
    }

    fn invoke_scan_services(&mut self, ctx: &mut Context<Self>) {
        if self.scanning_services {
            log::warn!("scan-services: Already running, skip");
            return;
        }

        self.scanning_services = true;

        let scanner = ServiceScanner::new(
            self.config.jobs.scan_services.command.clone(),
            self.collect_enabled_channels());

        let job = JobKind::ScanServices.create(self.semaphore.clone())
            .perform(scanner.scan_services());

        actix::fut::wrap_future::<_, Self>(job)
            .map(|result, act, _| {
                if let Ok(services) = result {
                    epg::update_services(services);
                }
                act.scanning_services = false;
            })
            .spawn(ctx);
    }

    fn schedule_scan_services(&self, ctx: &mut Context<Self>) {
        let datetime = self.calc_next_scheduled_datetime(
            &self.config.jobs.scan_services.schedule);
        log::info!("scan-services: Scheduled for {}", datetime);
        let interval = (datetime - Jst::now()).to_std().unwrap();
        ctx.run_later(interval, Self::scan_services);
    }

    fn sync_clocks(&mut self, ctx: &mut Context<Self>) {
        self.invoke_sync_clocks(ctx);
        self.schedule_sync_clocks(ctx);
    }

    fn invoke_sync_clocks(&mut self, ctx: &mut Context<Self>) {
        if self.synchronizing_clocks {
            log::warn!("sync-clocks: Already running, skip");
            return;
        }

        self.synchronizing_clocks = true;

        let sync = ClockSynchronizer::new(
            self.config.jobs.sync_clocks.command.clone(),
            self.collect_enabled_channels());

        let job = JobKind::SyncClocks.create(self.semaphore.clone())
            .perform(sync.sync_clocks());

        actix::fut::wrap_future::<_, Self>(job)
            .map(|result, act, _| {
                if let Ok(clocks) = result {
                    epg::update_clocks(clocks);
                }
                act.synchronizing_clocks = false;
            })
            .spawn(ctx);
    }

    fn schedule_sync_clocks(&self, ctx: &mut Context<Self>) {
        let datetime = self.calc_next_scheduled_datetime(
            &self.config.jobs.sync_clocks.schedule);
        log::info!("sync-clocks: Scheduled for {}", datetime);
        let interval = (datetime - Jst::now()).to_std().unwrap();
        ctx.run_later(interval, Self::sync_clocks);
    }

    fn update_schedules(&mut self, ctx: &mut Context<Self>) {
        self.invoke_update_schedules(ctx);
        self.schedule_update_schedules(ctx);
    }

    fn invoke_update_schedules(&mut self, ctx: &mut Context<Self>) {
        if self.updating_schedules {
            log::warn!("update-schedules: Already running, skip");
            return;
        }

        self.updating_schedules = true;

        let job = JobKind::UpdateSchedules.create(self.semaphore.clone())
            .perform(eit_feeder::feed_eit_sections());

        actix::fut::wrap_future::<_, Self>(job)
            .map(|_, act, _| {
                epg::save_schedules();
                act.updating_schedules = false;
            })
            .spawn(ctx);
    }

    fn schedule_update_schedules(&mut self, ctx: &mut Context<Self>) {
        let datetime = self.calc_next_scheduled_datetime(
            &self.config.jobs.update_schedules.schedule);
        log::info!("update-schedules: Scheduled for {}", datetime);
        let interval = (datetime - Jst::now()).to_std().unwrap();
        ctx.run_later(interval, Self::update_schedules);
    }

    fn collect_enabled_channels(&self) -> Vec<EpgChannel> {
        self.config
            .channels
            .iter()
            .filter(|config| !config.disabled)
            .cloned()
            .map(EpgChannel::from)
            .collect()
    }
}

impl Actor for JobManager {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::debug!("Started");
        self.schedule_scan_services(ctx);
        self.schedule_sync_clocks(ctx);
        self.schedule_update_schedules(ctx);
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::debug!("Stopped");
    }
}

impl Supervised for JobManager {}
impl SystemService for JobManager {}

impl Default for JobManager {
    fn default() -> Self {
        unreachable!();
    }
}

// invoke scan services

struct InvokeScanServicesMessage;

impl fmt::Display for InvokeScanServicesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvokeScanServices")
    }
}

impl Message for InvokeScanServicesMessage {
    type Result = ();
}

impl Handler<InvokeScanServicesMessage> for JobManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: InvokeScanServicesMessage,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.invoke_scan_services(ctx);
    }
}

// invoke sync clocks

struct InvokeSyncClocksMessage;

impl fmt::Display for InvokeSyncClocksMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvokeSyncClocks")
    }
}

impl Message for InvokeSyncClocksMessage {
    type Result = ();
}

impl Handler<InvokeSyncClocksMessage> for JobManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: InvokeSyncClocksMessage,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.invoke_sync_clocks(ctx);
    }
}

// invoke update schedules

struct InvokeUpdateSchedulesMessage;

impl fmt::Display for InvokeUpdateSchedulesMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvokeUpdateSchedules")
    }
}

impl Message for InvokeUpdateSchedulesMessage {
    type Result = ();
}

impl Handler<InvokeUpdateSchedulesMessage> for JobManager {
    type Result = ();

    fn handle(
        &mut self,
        msg: InvokeUpdateSchedulesMessage,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        log::debug!("{}", msg);
        self.invoke_update_schedules(ctx);
    }
}
