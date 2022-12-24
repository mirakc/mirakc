use std::future::Future;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use serde::Serialize;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::Semaphore;
use tracing::Instrument;

use crate::command_util::CommandBuilder;
use crate::config::Concurrency;
use crate::config::Config;
use crate::epg;
use crate::error::Error;
use crate::models::events::*;
use crate::recording;

pub struct ScriptRunner<E, R> {
    config: Arc<Config>,
    epg: E,
    recording_manager: R,
    semaphore: Arc<Semaphore>,
}

impl<E, R> ScriptRunner<E, R> {
    pub fn new(config: Arc<Config>, epg: E, recording_manager: R) -> Self {
        let concurrency = match config.scripts.concurrency {
            Concurrency::Unlimited => Semaphore::MAX_PERMITS,
            Concurrency::Number(n) => n.max(1),
            Concurrency::NumCpus(r) => (num_cpus::get() as f32 * r).max(1.0) as usize,
        };
        ScriptRunner {
            config,
            epg,
            recording_manager,
            semaphore: Arc::new(Semaphore::new(concurrency)),
        }
    }

    fn has_epg_programs_updated_script(&self) -> bool {
        !self.config.scripts.epg.programs_updated.is_empty()
    }

    fn has_recording_started_script(&self) -> bool {
        !self.config.scripts.recording.started.is_empty()
    }

    fn has_recording_stopped_script(&self) -> bool {
        !self.config.scripts.recording.stopped.is_empty()
    }

    fn has_recording_failed_script(&self) -> bool {
        !self.config.scripts.recording.failed.is_empty()
    }

    fn has_recording_retried_script(&self) -> bool {
        !self.config.scripts.recording.retried.is_empty()
    }

    fn has_recording_rescheduled_script(&self) -> bool {
        !self.config.scripts.recording.rescheduled.is_empty()
    }
}

#[async_trait]
impl<E, R> Actor for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");
        if self.has_epg_programs_updated_script() {
            self.epg
                .call(epg::RegisterEmitter::ProgramsUpdated(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for epg::ProgramsUpdated");
        }
        if self.has_recording_started_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingStarted(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingStarted");
        }
        if self.has_recording_stopped_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingStopped(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingStopped");
        }
        if self.has_recording_failed_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingFailed(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingFailed");
        }
        if self.has_recording_retried_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingRetried(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingRetried");
        }
        if self.has_recording_rescheduled_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingRescheduled(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingRescheduled");
        }
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// epg::ProgramsUpdated

#[async_trait]
impl<E, R> Handler<epg::ProgramsUpdated> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ProgramsUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ProgramsUpdated", %msg.service_triple);
        ctx.spawn_task(self.create_epg_programs_updated_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_epg_programs_updated_task(
        &self,
        event: EpgProgramsUpdated,
    ) -> impl Future<Output = ()> {
        let span = tracing::info_span!(EpgProgramsUpdated::name(), %event.service_id);
        let fut = Self::run_epg_programs_updated_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_epg_programs_updated_script(
        config: Arc<Config>,
        event: EpgProgramsUpdated,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.epg.programs_updated)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingStarted

#[async_trait]
impl<E, R> Handler<recording::RecordingStarted> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingStarted, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_quad);
        ctx.spawn_task(self.create_recording_started_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_recording_started_task(&self, event: RecordingStarted) -> impl Future<Output = ()> {
        let span = tracing::info_span!(RecordingStarted::name(), %event.program_id);
        let fut = Self::run_recording_started_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_started_script(
        config: Arc<Config>,
        event: RecordingStarted,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording.started)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingStopped

#[async_trait]
impl<E, R> Handler<recording::RecordingStopped> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingStopped, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_quad);
        ctx.spawn_task(self.create_recording_stopped_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_recording_stopped_task(&self, event: RecordingStopped) -> impl Future<Output = ()> {
        let span = tracing::info_span!(RecordingStopped::name(), %event.program_id);
        let fut = Self::run_recording_stopped_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_stopped_script(
        config: Arc<Config>,
        event: RecordingStopped,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording.stopped)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingFailed

#[async_trait]
impl<E, R> Handler<recording::RecordingFailed> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingFailed, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingFailed", %msg.program_quad, ?msg.reason);
        ctx.spawn_task(self.create_recording_failed_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_recording_failed_task(&self, event: RecordingFailed) -> impl Future<Output = ()> {
        let span = tracing::info_span!(RecordingFailed::name(), %event.program_id);
        let fut = Self::run_recording_failed_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_failed_script(
        config: Arc<Config>,
        event: RecordingFailed,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording.failed)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingRetried

#[async_trait]
impl<E, R> Handler<recording::RecordingRetried> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingRetried, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingRetried", %msg.program_quad);
        ctx.spawn_task(self.create_recording_retried_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_recording_retried_task(&self, event: RecordingRetried) -> impl Future<Output = ()> {
        let span = tracing::info_span!(RecordingRetried::name(), %event.program_id);
        let fut = Self::run_recording_retried_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_retried_script(
        config: Arc<Config>,
        event: RecordingRetried,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording.retried)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingRescheduled

#[async_trait]
impl<E, R> Handler<recording::RecordingRescheduled> for ScriptRunner<E, R>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingRescheduled, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingRescheduled", %msg.program_quad);
        ctx.spawn_task(self.create_recording_rescheduled_task(msg.into()));
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn create_recording_rescheduled_task(
        &self,
        event: RecordingRescheduled,
    ) -> impl Future<Output = ()> {
        let span = tracing::info_span!(RecordingRescheduled::name(), %event.program_id);
        let fut = Self::run_recording_rescheduled_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_rescheduled_script(
        config: Arc<Config>,
        event: RecordingRescheduled,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording.rescheduled)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// helpers

fn wrap(
    semaphore: Arc<Semaphore>,
    fut: impl Future<Output = Result<ExitStatus, Error>>,
) -> impl Future<Output = ()> {
    async move {
        let _permit = semaphore.acquire().await;
        tracing::info!("Start");
        match fut.await {
            Ok(status) => {
                if status.success() {
                    tracing::info!("Done successfully");
                } else {
                    tracing::error!(%status);
                }
            }
            Err(err) => tracing::error!(%err),
        }
    }
}

// Use stderr for logging from a script.  Data from stdout of the script will be
// thrown away at this point.
//
// TODO
// ----
// There is no "safe" way to redirect stdout to stderr of tokio::process::Child
// (and also std::process::Child) at this point.
// https://users.rust-lang.org/t/double-redirection-stdout-stderr/13554
//
// FrowRawFd::from_raw_fd() is an unsafe function.  In addition, the
// RawFd may be closed twice on drop.
fn spawn_command(command: &str) -> Result<Child, Error> {
    Ok(CommandBuilder::new(command)?
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?)
}

async fn write_line<W, T>(write: &mut W, data: &T) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let json = serde_json::to_vec(data)?;
    write.write_all(&json).await?;
    write.write_all(b"\n").await?;
    Ok(())
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::epg::stub::EpgStub;
    use crate::recording::stub::RecordingManagerStub;
    use assert_matches::assert_matches;

    type TestTarget = ScriptRunner<EpgStub, RecordingManagerStub>;

    #[tokio::test]
    async fn test_run_epg_programs_updated_script() {
        let event = EpgProgramsUpdated {
            service_id: (1, 2).into(),
        };

        let mut config = Config::default();
        config.scripts.epg.programs_updated = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.epg.programs_updated = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.epg.programs_updated = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_started_script() {
        let event = RecordingStarted {
            program_id: (1, 2, 3).into(),
        };

        let mut config = Config::default();
        config.scripts.recording.started = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording.started = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording.started = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_stopped_script() {
        let event = RecordingStopped {
            program_id: (1, 2, 3).into(),
        };

        let mut config = Config::default();
        config.scripts.recording.stopped = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording.stopped = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording.stopped = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_failed_script() {
        let event = RecordingFailed {
            program_id: (1, 2, 3).into(),
            reason: recording::RecordingFailedReason::IoError {
                message: "message".to_string(),
                os_error: None,
            },
        };
        let mut config = Config::default();
        config.scripts.recording.failed = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let event = RecordingFailed {
            program_id: (1, 2, 3).into(),
            reason: recording::RecordingFailedReason::PipelineError { exit_code: 1 },
        };
        let mut config = Config::default();
        config.scripts.recording.failed = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let event = RecordingFailed {
            program_id: (1, 2, 3).into(),
            reason: recording::RecordingFailedReason::RetryFailed,
        };

        let mut config = Config::default();
        config.scripts.recording.failed = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording.failed = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording.failed = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_retried_script() {
        let event = RecordingRetried {
            program_id: (1, 2, 3).into(),
        };

        let mut config = Config::default();
        config.scripts.recording.retried = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_retried_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording.retried = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_retried_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording.retried = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_retried_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_rescheduled_script() {
        let event = RecordingRescheduled {
            program_id: (1, 2, 3).into(),
        };

        let mut config = Config::default();
        config.scripts.recording.rescheduled = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording.rescheduled = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording.rescheduled = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }
}
// </coverage:exclude>
