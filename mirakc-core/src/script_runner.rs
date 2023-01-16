use std::future::Future;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;

use actlet::prelude::*;
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
use crate::onair;
use crate::recording;

pub struct ScriptRunner<E, R, O> {
    config: Arc<Config>,
    epg: E,
    recording_manager: R,
    onair_manager: O,
    semaphore: Arc<Semaphore>,
}

impl<E, R, O> ScriptRunner<E, R, O> {
    pub fn new(config: Arc<Config>, epg: E, recording_manager: R, onair_manager: O) -> Self {
        let concurrency = match config.events.concurrency {
            Concurrency::Unlimited => Semaphore::MAX_PERMITS,
            Concurrency::Number(n) => n.max(1),
            Concurrency::NumCpus(r) => (num_cpus::get() as f32 * r).max(1.0) as usize,
        };
        ScriptRunner {
            config,
            epg,
            recording_manager,
            onair_manager,
            semaphore: Arc::new(Semaphore::new(concurrency)),
        }
    }

    fn has_epg_programs_updated_script(&self) -> bool {
        !self.config.events.epg.programs_updated.is_empty()
    }

    fn has_recording_started_script(&self) -> bool {
        !self.config.events.recording.started.is_empty()
    }

    fn has_recording_stopped_script(&self) -> bool {
        !self.config.events.recording.stopped.is_empty()
    }

    fn has_recording_failed_script(&self) -> bool {
        !self.config.events.recording.failed.is_empty()
    }

    fn has_recording_rescheduled_script(&self) -> bool {
        !self.config.events.recording.rescheduled.is_empty()
    }

    fn has_onair_program_changed_script(&self) -> bool {
        !self.config.events.onair.program_changed.is_empty()
    }
}

#[async_trait]
impl<E, R, O> Actor for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
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
        if self.has_recording_rescheduled_script() {
            self.recording_manager
                .call(recording::RegisterEmitter::RecordingRescheduled(
                    ctx.address().clone().into(),
                ))
                .await
                .expect("Failed to register emitter for recording::RecordingRescheduled");
        }
        if self.has_onair_program_changed_script() {
            self.onair_manager
                .call(onair::RegisterEmitter(ctx.address().clone().into()))
                .await
                .expect("Failed to register emitter for onair::OnairProgramChanged");
        }
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

// epg::ProgramsUpdated

#[async_trait]
impl<E, R, O> Handler<epg::ProgramsUpdated> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: epg::ProgramsUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ProgramsUpdated", %msg.service_id);
        ctx.spawn_task(self.create_epg_programs_updated_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_epg_programs_updated_task(
        &self,
        event: EpgProgramsUpdated,
    ) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(EpgProgramsUpdated::name(), %event.service_id);
        let fut = Self::run_epg_programs_updated_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_epg_programs_updated_script(
        config: Arc<Config>,
        event: EpgProgramsUpdated,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.epg.programs_updated)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingStarted

#[async_trait]
impl<E, R, O> Handler<recording::RecordingStarted> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingStarted, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_id);
        ctx.spawn_task(self.create_recording_started_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_recording_started_task(&self, event: RecordingStarted) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(RecordingStarted::name(), %event.program_id);
        let fut = Self::run_recording_started_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_started_script(
        config: Arc<Config>,
        event: RecordingStarted,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.recording.started)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingStopped

#[async_trait]
impl<E, R, O> Handler<recording::RecordingStopped> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingStopped, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_id);
        ctx.spawn_task(self.create_recording_stopped_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_recording_stopped_task(&self, event: RecordingStopped) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(RecordingStopped::name(), %event.program_id);
        let fut = Self::run_recording_stopped_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_stopped_script(
        config: Arc<Config>,
        event: RecordingStopped,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.recording.stopped)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingFailed

#[async_trait]
impl<E, R, O> Handler<recording::RecordingFailed> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingFailed, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingFailed", %msg.program_id, ?msg.reason);
        ctx.spawn_task(self.create_recording_failed_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_recording_failed_task(&self, event: RecordingFailed) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(RecordingFailed::name(), %event.program_id);
        let fut = Self::run_recording_failed_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_failed_script(
        config: Arc<Config>,
        event: RecordingFailed,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.recording.failed)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// recording::RecordingRescheduled

#[async_trait]
impl<E, R, O> Handler<recording::RecordingRescheduled> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: recording::RecordingRescheduled, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "RecordingRescheduled", %msg.program_id);
        ctx.spawn_task(self.create_recording_rescheduled_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_recording_rescheduled_task(
        &self,
        event: RecordingRescheduled,
    ) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(RecordingRescheduled::name(), %event.program_id);
        let fut = Self::run_recording_rescheduled_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_recording_rescheduled_script(
        config: Arc<Config>,
        event: RecordingRescheduled,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.recording.rescheduled)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &event).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

// onair::OnairProgramChanged

#[async_trait]
impl<E, R, O> Handler<onair::OnairProgramChanged> for ScriptRunner<E, R, O>
where
    E: Send + Sync + 'static,
    E: Call<epg::RegisterEmitter>,
    R: Send + Sync + 'static,
    R: Call<recording::RegisterEmitter>,
    O: Send + Sync + 'static,
    O: Call<onair::RegisterEmitter>,
{
    async fn handle(&mut self, msg: onair::OnairProgramChanged, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "OnairProgramChanged", %msg.service_id);
        ctx.spawn_task(self.create_onair_program_changed_task(msg.into()));
    }
}

impl<E, R, O> ScriptRunner<E, R, O> {
    fn create_onair_program_changed_task(
        &self,
        event: OnairProgramChanged,
    ) -> impl Future<Output = ()> {
        let span = tracing::debug_span!(OnairProgramChanged::name(), %event.service_id);
        let fut = Self::run_onair_program_changed_script(self.config.clone(), event);
        wrap(self.semaphore.clone(), fut).instrument(span)
    }

    async fn run_onair_program_changed_script(
        config: Arc<Config>,
        event: OnairProgramChanged,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.events.onair.program_changed)?;
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
        tracing::debug!("Start");
        match fut.await {
            Ok(status) => {
                if status.success() {
                    tracing::debug!("Done successfully");
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
    use crate::onair::stub::OnairProgramManagerStub;
    use crate::recording::stub::RecordingManagerStub;
    use assert_matches::assert_matches;

    type TestTarget = ScriptRunner<EpgStub, RecordingManagerStub, OnairProgramManagerStub>;

    #[tokio::test]
    async fn test_run_epg_programs_updated_script() {
        let event = EpgProgramsUpdated {
            service_id: (1, 2).into(),
        };

        let mut config = Config::default();
        config.events.epg.programs_updated = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.epg.programs_updated = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.epg.programs_updated = "command-not-found".to_string();
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
        config.events.recording.started = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.recording.started = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.recording.started = "command-not-found".to_string();
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
        config.events.recording.stopped = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.recording.stopped = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.recording.stopped = "command-not-found".to_string();
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
        config.events.recording.failed = format!(
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
        config.events.recording.failed = format!(
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
            reason: recording::RecordingFailedReason::ScheduleExpired,
        };

        let mut config = Config::default();
        config.events.recording.failed = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.recording.failed = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.recording.failed = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_failed_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_rescheduled_script() {
        let event = RecordingRescheduled {
            program_id: (1, 2, 3).into(),
        };

        let mut config = Config::default();
        config.events.recording.rescheduled = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.recording.rescheduled = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.recording.rescheduled = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_rescheduled_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_onair_program_changed_script() {
        let event = OnairProgramChanged {
            service_id: (1, 2).into(),
        };

        let mut config = Config::default();
        config.events.onair.program_changed = format!(
            r#"env EXPECTED='{}' sh -c 'test "$(cat)" = "$EXPECTED"'"#,
            serde_json::to_string(&event).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_onair_program_changed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.events.onair.program_changed = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_onair_program_changed_script(config, event.clone()).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.events.onair.program_changed = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_onair_program_changed_script(config, event.clone()).await;
        assert_matches!(result, Err(_));
    }
}
// </coverage:exclude>
