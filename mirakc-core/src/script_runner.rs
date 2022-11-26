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
use crate::models::MirakurunProgramId;
use crate::models::MirakurunServiceId;
use crate::models::ServiceTriple;
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
        self.epg
            .call(epg::RegisterEmitter::ProgramsUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for epg::ProgramsUpdated");
        self.recording_manager
            .call(recording::RegisterEmitter::RecordingStarted(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for recording::RecordingStarted");
        self.recording_manager
            .call(recording::RegisterEmitter::RecordingStopped(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for recording::RecordingStopped");
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
        if self.has_epg_programs_updated_script() {
            ctx.spawn_task(self.create_epg_programs_updated_task(msg.service_triple));
        }
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn has_epg_programs_updated_script(&self) -> bool {
        !self.config.scripts.epg_programs_updated.is_empty()
    }

    fn create_epg_programs_updated_task(
        &self,
        service_triple: ServiceTriple,
    ) -> impl Future<Output = ()> {
        let fut = Self::run_epg_programs_updated_script(self.config.clone(), service_triple.into());
        wrap(self.semaphore.clone(), fut)
            .instrument(tracing::info_span!("epg-program-updated-script", %service_triple))
    }

    async fn run_epg_programs_updated_script(
        config: Arc<Config>,
        msid: MirakurunServiceId,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.epg_programs_updated)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &msid).await?;
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
        tracing::debug!(msg.name = "RecordingStarted", %msg.program_id);
        if self.has_recording_started_script() {
            ctx.spawn_task(self.create_recording_started_task(msg.program_id));
        }
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn has_recording_started_script(&self) -> bool {
        !self.config.scripts.recording_started.is_empty()
    }

    fn create_recording_started_task(
        &self,
        program_id: MirakurunProgramId,
    ) -> impl Future<Output = ()> {
        let fut = Self::run_recording_started_script(self.config.clone(), program_id);
        wrap(self.semaphore.clone(), fut)
            .instrument(tracing::info_span!("recording-started-script", %program_id))
    }

    async fn run_recording_started_script(
        config: Arc<Config>,
        program_id: MirakurunProgramId,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording_started)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &program_id).await?;
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
        tracing::debug!(msg.name = "RecordingStopped", %msg.program_id, ?msg.result);
        if self.has_recording_stopped_script() {
            ctx.spawn_task(self.create_recording_stopped_task(msg.program_id, msg.result));
        }
    }
}

impl<E, R> ScriptRunner<E, R> {
    fn has_recording_stopped_script(&self) -> bool {
        !self.config.scripts.recording_stopped.is_empty()
    }

    fn create_recording_stopped_task(
        &self,
        program_id: MirakurunProgramId,
        result: Result<u64, String>,
    ) -> impl Future<Output = ()> {
        let fut = Self::run_recording_stopped_script(self.config.clone(), program_id, result);
        wrap(self.semaphore.clone(), fut)
            .instrument(tracing::info_span!("recording-stopped-script", %program_id))
    }

    async fn run_recording_stopped_script(
        config: Arc<Config>,
        program_id: MirakurunProgramId,
        result: Result<u64, String>,
    ) -> Result<ExitStatus, Error> {
        let mut child = spawn_command(&config.scripts.recording_stopped)?;
        let mut input = child.stdin.take().unwrap();
        write_line(&mut input, &program_id).await?;
        write_line(&mut input, &RecordingStoppedResult::from(result)).await?;
        drop(input);
        Ok(child.wait().await?)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum RecordingStoppedResult {
    Ok(u64),
    Err(String),
}

impl From<Result<u64, String>> for RecordingStoppedResult {
    fn from(result: Result<u64, String>) -> Self {
        match result {
            Ok(v) => RecordingStoppedResult::Ok(v),
            Err(s) => RecordingStoppedResult::Err(s),
        }
    }
}

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
    use std::io::Write;
    use tempfile::NamedTempFile;

    type TestTarget = ScriptRunner<EpgStub, RecordingManagerStub>;

    #[tokio::test]
    async fn test_run_epg_programs_updated_script() {
        let service_id = (1, 2).into();

        let mut script = NamedTempFile::new().unwrap();
        write!(script, "read ID\n").unwrap();
        write!(
            script,
            "test $ID = {}\n",
            serde_json::to_string(&service_id).unwrap()
        )
        .unwrap();

        let mut config = Config::default();
        config.scripts.epg_programs_updated = format!("sh {}", script.path().to_str().unwrap());
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, service_id).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.epg_programs_updated = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, service_id).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.epg_programs_updated = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_epg_programs_updated_script(config, service_id).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_started_script() {
        let program_id = (1, 2, 3).into();

        let mut config = Config::default();
        config.scripts.recording_started = format!(
            r#"sh -c "test $(cat) = {}""#,
            serde_json::to_string(&program_id).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, program_id).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording_started = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, program_id).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording_started = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_started_script(config, program_id).await;
        assert_matches!(result, Err(_));
    }

    #[tokio::test]
    async fn test_run_recording_stopped_script() {
        let program_id = (1, 2, 3).into();

        let mut script = NamedTempFile::new().unwrap();
        write!(script, "read ID\n").unwrap();
        write!(script, "test $ID = $1\n").unwrap();
        write!(script, "read ST\n").unwrap();
        write!(script, "test $ST = $2\n").unwrap();

        let result = Ok(0);
        let mut config = Config::default();
        config.scripts.recording_stopped = format!(
            "sh {} {} '{}'",
            script.path().to_str().unwrap(),
            serde_json::to_string(&program_id).unwrap(),
            serde_json::to_string(&RecordingStoppedResult::from(result.clone())).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, program_id, result).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let result = Err("msg".to_string());
        let mut config = Config::default();
        config.scripts.recording_stopped = format!(
            "sh {} {} '{}'",
            script.path().to_str().unwrap(),
            serde_json::to_string(&program_id).unwrap(),
            serde_json::to_string(&RecordingStoppedResult::from(result.clone())).unwrap(),
        );
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, program_id, result).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(0));
        });

        let mut config = Config::default();
        config.scripts.recording_stopped = "sh -c 'cat; false'".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, program_id, Ok(0)).await;
        assert_matches!(result, Ok(status) => {
            assert_matches!(status.code(), Some(1));
        });

        let mut config = Config::default();
        config.scripts.recording_stopped = "command-not-found".to_string();
        let config = Arc::new(config);
        let result = TestTarget::run_recording_stopped_script(config, program_id, Ok(0)).await;
        assert_matches!(result, Err(_));
    }

    #[test]
    fn test_recording_stopped_result() {
        assert_eq!(
            r#"{"ok":0}"#,
            serde_json::to_string(&RecordingStoppedResult::from(Ok(0))).unwrap()
        );
        assert_eq!(
            r#"{"err":"msg"}"#,
            serde_json::to_string(&RecordingStoppedResult::from(Err("msg".to_string()))).unwrap()
        );
    }
}
// </coverage:exclude>
