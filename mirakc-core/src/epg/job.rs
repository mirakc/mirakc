use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use actlet::prelude::*;
use chrono::DateTime;
use chrono_jst::Jst;

use crate::config::Config;
use crate::epg::clock_synchronizer::ClockSynchronizer;
use crate::epg::eit_feeder::FeedEitSections;
use crate::epg::service_scanner::ServiceScanner;
use crate::epg::*;

pub struct JobManager<T, E, F> {
    config: Arc<Config>,
    scanning_services: bool,
    synchronizing_clocks: bool,
    updating_schedules: bool,
    tuner_manager: T,
    epg: E,
    eit_feeder: F,
}

impl<T, E, F> JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    pub fn new(config: Arc<Config>, tuner_manager: T, epg: E, eit_feeder: F) -> Self {
        JobManager {
            config,
            scanning_services: false,
            synchronizing_clocks: false,
            updating_schedules: false,
            tuner_manager,
            epg,
            eit_feeder,
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

    async fn scan_services(&mut self, ctx: &mut Context<Self>) {
        if Self::is_job_disabled_for_debug("scan-services") {
            tracing::debug!(job = "scan-services", "Disabled for debug");
            return;
        }
        self.invoke_scan_services(ctx).await;
        self.schedule_scan_services(ctx);
    }

    async fn invoke_scan_services(&mut self, _ctx: &mut Context<Self>) {
        if self.scanning_services {
            tracing::warn!(job = "scan-services", "Already running, skip");
            return;
        }

        tracing::info!(job = "scan-services", "Performing...");
        let now = Instant::now();
        self.scanning_services = true;
        let scanner = ServiceScanner::new(self.config.clone(), self.tuner_manager.clone());
        let results = scanner.scan_services().await;
        self.epg.emit(UpdateServices { results }).await;
        self.scanning_services = false;
        let elapsed = now.elapsed();
        tracing::info!(job = "scan-services", elapsed = %humantime::format_duration(elapsed), "Done");
    }

    fn schedule_scan_services(&self, ctx: &mut Context<Self>) {
        let datetime = self.calc_next_scheduled_datetime(&self.config.jobs.scan_services.schedule);
        tracing::info!(
            job = "scan-services",
            datetime = datetime.to_rfc3339(),
            "Scheduled"
        );
        let interval = (datetime - Jst::now()).to_std().unwrap();
        let addr = ctx.address().clone();
        ctx.spawn_task(async move {
            tokio::time::sleep(interval).await;
            addr.emit(ScanServices).await;
        });
    }

    async fn sync_clocks(&mut self, ctx: &mut Context<Self>) {
        if Self::is_job_disabled_for_debug("sync-clocks") {
            tracing::debug!(job = "sync-clocks", "Disabled for debug");
            return;
        }
        self.invoke_sync_clocks(ctx).await;
        self.schedule_sync_clocks(ctx);
    }

    async fn invoke_sync_clocks(&mut self, _ctx: &mut Context<Self>) {
        if self.synchronizing_clocks {
            tracing::warn!(job = "sync-clocks", "Already running, skip");
            return;
        }

        tracing::info!(job = "sync-clocks", "Performing...");
        self.synchronizing_clocks = true;
        let now = Instant::now();
        let sync = ClockSynchronizer::new(self.config.clone(), self.tuner_manager.clone());
        let results = sync.sync_clocks().await;
        self.epg.emit(UpdateClocks { results }).await;
        self.synchronizing_clocks = false;
        let elapsed = now.elapsed();
        tracing::info!(job = "sync-clocks", elapsed = %humantime::format_duration(elapsed), "Done");
    }

    fn schedule_sync_clocks(&self, ctx: &mut Context<Self>) {
        let datetime = self.calc_next_scheduled_datetime(&self.config.jobs.sync_clocks.schedule);
        tracing::info!(
            job = "sync-clocks",
            datetime = datetime.to_rfc3339(),
            "Scheduled"
        );
        let interval = (datetime - Jst::now()).to_std().unwrap();
        let addr = ctx.address().clone();
        ctx.spawn_task(async move {
            tokio::time::sleep(interval).await;
            addr.emit(SyncClocks).await;
        });
    }

    async fn update_schedules(&mut self, ctx: &mut Context<Self>) {
        if Self::is_job_disabled_for_debug("update-schedules") {
            tracing::debug!(job = "update-schedules", "Disabled for debug");
            return;
        }
        self.invoke_update_schedules(ctx).await;
        self.schedule_update_schedules(ctx);
    }

    async fn invoke_update_schedules(&mut self, _ctx: &mut Context<Self>) {
        if self.updating_schedules {
            tracing::warn!(job = "update-schedules", "Already running, skip");
            return;
        }

        tracing::info!(job = "update-schedules", "Performing...");
        let now = Instant::now();
        self.updating_schedules = true;
        let eit_feeder = self.eit_feeder.clone();
        match eit_feeder.call(FeedEitSections).await {
            Ok(_) => self.epg.emit(SaveSchedules).await,
            Err(err) => tracing::error!(%err, job = "update-schedules"),
        }
        self.updating_schedules = false;
        let elapsed = now.elapsed();
        tracing::info!(job = "update-schedules", elapsed = %humantime::format_duration(elapsed), "Done");
    }

    fn schedule_update_schedules(&mut self, ctx: &mut Context<Self>) {
        let datetime =
            self.calc_next_scheduled_datetime(&self.config.jobs.update_schedules.schedule);
        tracing::info!(
            job = "update-schedules",
            datetime = datetime.to_rfc3339(),
            "Scheduled"
        );
        let interval = (datetime - Jst::now()).to_std().unwrap();
        let addr = ctx.address().clone();
        ctx.spawn_task(async move {
            tokio::time::sleep(interval).await;
            addr.emit(UpdateSchedules).await;
        });
    }

    fn is_job_disabled_for_debug(job: &str) -> bool {
        env::var("MIRAKC_DEBUG_DISABLE_JOBS")
            .ok()
            .map(|var| var.split_whitespace().position(|s| s == job))
            .flatten()
            .is_some()
    }
}

#[async_trait]
impl<T, E, F> Actor for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        // It's guaranteed that no response is sent before initial jobs are invoked.
        tracing::debug!("Started");
        if self.config.jobs.scan_services.disabled {
            tracing::warn!(job = "scan-services", "Disabled");
        } else if is_fresh(&self.config, "services.json") {
            tracing::debug!(job = "scan-services", "Skip initial scan");
            self.schedule_scan_services(ctx);
        } else {
            self.scan_services(ctx).await;
        }
        if self.config.jobs.sync_clocks.disabled {
            tracing::warn!(job = "sync-clocks", "Disabled");
        } else if is_fresh(&self.config, "clocks.json") {
            tracing::debug!(job = "sync-clocks", "Skip initial scan");
            self.schedule_sync_clocks(ctx);
        } else {
            self.sync_clocks(ctx).await;
        }
        if self.config.jobs.update_schedules.disabled {
            tracing::warn!(job = "update-schedules", "Disabled");
        } else if is_fresh(&self.config, "schedules.json") {
            tracing::debug!(job = "update-schedules", "Skip initial scan");
            self.schedule_update_schedules(ctx);
        } else {
            self.update_schedules(ctx).await;
        }
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// scan services

#[derive(Message)]
struct ScanServices;

#[async_trait]
impl<T, E, F> Handler<ScanServices> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: ScanServices, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ScanServices");
        self.scan_services(ctx).await;
    }
}

// invoke scan services

#[derive(Message)]
struct InvokeScanServices;

#[async_trait]
impl<T, E, F> Handler<InvokeScanServices> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks> + 'static,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: InvokeScanServices, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "InvokeScanServices");
        self.invoke_scan_services(ctx).await;
    }
}

// sync clocks

#[derive(Message)]
struct SyncClocks;

#[async_trait]
impl<T, E, F> Handler<SyncClocks> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: SyncClocks, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "SyncClocks");
        self.sync_clocks(ctx).await;
    }
}

// invoke sync clocks

#[derive(Message)]
struct InvokeSyncClocks;

#[async_trait]
impl<T, E, F> Handler<InvokeSyncClocks> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: InvokeSyncClocks, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "InvokeSyncClocks");
        self.invoke_sync_clocks(ctx).await;
    }
}

// update schedules

#[derive(Message)]
struct UpdateSchedules;

#[async_trait]
impl<T, E, F> Handler<UpdateSchedules> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: UpdateSchedules, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "UpdateSchedules");
        self.update_schedules(ctx).await;
    }
}

// invoke update schedules

#[derive(Message)]
struct InvokeUpdateSchedules;

#[async_trait]
impl<T, E, F> Handler<InvokeUpdateSchedules> for JobManager<T, E, F>
where
    T: Clone + Send + Sync + 'static,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Emit<SaveSchedules>,
    E: Emit<UpdateClocks>,
    E: Emit<UpdateServices>,
    F: Clone + Send + Sync + 'static,
    F: Call<FeedEitSections>,
{
    async fn handle(&mut self, _msg: InvokeUpdateSchedules, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "InvokeUpdateSchedules");
        self.invoke_update_schedules(ctx).await;
    }
}

// used for debugging purposes

static EPG_FRESH_PERIOD: Lazy<Option<std::time::Duration>> = Lazy::new(|| {
    let period = std::env::var("MIRAKC_EPG_FRESH_PERIOD")
        .ok()
        .map(|s| humantime::parse_duration(&s).ok())
        .flatten();
    tracing::debug!(MIRAKC_EPG_FRESH_PERIOD = ?period);
    period
});

fn is_fresh(config: &Config, filename: &str) -> bool {
    let cache_dir = if let Some(ref cache_dir) = config.epg.cache_dir {
        cache_dir
    } else {
        return false;
    };
    let path = cache_dir.join(filename);
    match (*EPG_FRESH_PERIOD, path.metadata()) {
        (Some(period), Ok(metadata)) => metadata
            .modified()
            .ok()
            .map(|time| time.elapsed().ok())
            .flatten()
            .map(|elapsed| elapsed <= period)
            .unwrap_or(false),
        _ => false,
    }
}
