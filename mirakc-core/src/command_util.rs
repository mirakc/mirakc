use std::convert::TryInto;
use std::env;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitStatus;
use std::process::Stdio;
use std::task::Context;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::fs::File;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::ReadBuf;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tracing::Instrument;
use tracing::Span;

use actlet::Spawn;

const COMMAND_PIPELINE_TERMINATION_WAIT_NANOS_DEFAULT: Duration = Duration::from_nanos(100_000); // 100us

static COMMAND_PIPELINE_TERMINATION_WAIT_NANOS: Lazy<Duration> = Lazy::new(|| {
    let nanos = env::var("MIRAKC_COMMAND_PIPELINE_TERMINATION_WAIT_NANOS")
        .ok()
        .map(|s| {
            s.parse::<u64>().expect(
                "MIRAKC_COMMAND_PIPELINE_TERMINATION_WAIT_NANOS \
                      must be a u64 value",
            )
        })
        .map(Duration::from_nanos)
        .unwrap_or(COMMAND_PIPELINE_TERMINATION_WAIT_NANOS_DEFAULT);
    tracing::debug!(
        COMMAND_PIPELINE_TERMINATION_WAIT_NANOS = %humantime::format_duration(nanos),
    );
    nanos
});

// Spawn processes for input commands and build a pipeline, then returns it.
// Input and output endpoints can be took from the pipeline only once
// respectively.
pub fn spawn_pipeline<T, C>(
    commands: Vec<String>,
    id: T,
    label: &'static str,
    ctx: &C,
) -> Result<CommandPipeline<T>, Error>
where
    T: std::fmt::Display + Clone + Unpin,
    C: Spawn,
{
    CommandPipelineBuilder::new(commands, id, label).build(ctx)
}

// errors

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse: {0}")]
    UnableToParse(String),
    #[error("Unable to spawn: {0}: {1}")]
    UnableToSpawn(String, io::Error),
    #[error(transparent)]
    IoError(#[from] io::Error),
}

// pipeline builder

pub struct CommandPipelineBuilder<T>
where
    T: std::fmt::Display + Clone + Unpin,
{
    commands: Vec<String>,
    envs: Vec<(&'static str, String)>,
    id: T,
    label: &'static str,
    log_file: Option<PathBuf>,
    logging: CommandLogging,
}

impl<T> CommandPipelineBuilder<T>
where
    T: std::fmt::Display + Clone + Unpin,
{
    pub fn new(commands: Vec<String>, id: T, label: &'static str) -> Self {
        Self {
            commands,
            envs: vec![],
            id,
            label,
            log_file: None,
            logging: CommandLogging::Default,
        }
    }

    pub fn set_logging(&mut self, logging: bool) {
        self.logging = if logging {
            CommandLogging::Enabled
        } else {
            CommandLogging::Disabled
        };
    }

    pub fn set_log_filter(&mut self, log_filter: &str) {
        self.envs.push(("MIRAKC_ARIB_LOG", log_filter.to_string()));
    }

    pub fn set_log_file<P: AsRef<Path>>(&mut self, log_file: P) {
        self.envs
            .push(("MIRAKC_ARIB_LOG_NO_TIMESTAMP", "0".to_string())); // enable timestamp
        self.log_file = Some(log_file.as_ref().to_owned());
    }

    pub fn build<C: Spawn>(self, ctx: &C) -> Result<CommandPipeline<T>, Error> {
        let span = tracing::debug_span!(parent: None, "pipeline", %self.id, self.label);
        // We can safely use Span::enter() in non-async functions.
        let _enter = span.enter();
        let mut pipeline = CommandPipeline::new(self.id, span.clone());
        for command in self.commands.into_iter() {
            pipeline.spawn(command, &self.envs, self.logging)?;
        }
        match self.log_file.as_ref() {
            Some(log_file) => pipeline.log_to_file(log_file, ctx),
            None => pipeline.log_to_console(ctx),
        }
        Ok(pipeline)
    }
}

// pipeline

pub struct CommandPipeline<T>
where
    T: Clone + Unpin,
{
    id: T,
    sender: broadcast::Sender<()>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    commands: Vec<CommandData>,
    span: Span,
}

struct CommandData {
    command: String,
    process: Child,
    pid: u32,
}

impl<T> CommandPipeline<T>
where
    T: Clone + Unpin,
{
    fn new(id: T, span: Span) -> Self {
        let (sender, _) = broadcast::channel(1);
        Self {
            id,
            sender,
            stdin: None,
            stdout: None,
            commands: Vec::new(),
            span,
        }
    }

    fn spawn(
        &mut self,
        command: String,
        envs: &[(&str, String)],
        logging: CommandLogging,
    ) -> Result<(), Error> {
        let input = if self.stdout.is_none() {
            Stdio::piped()
        } else {
            self.stdout.take().unwrap().try_into()?
        };

        let mut process = CommandBuilder::new(&command)?
            .stdin(input)
            .stdout(Stdio::piped())
            .envs(envs.iter().cloned())
            .logging(logging)
            .spawn()?;
        let pid = process.id().unwrap();

        if self.stdin.is_none() {
            self.stdin = process.stdin.take();
        }
        self.stdout = process.stdout.take();
        self.commands.push(CommandData {
            command,
            process,
            pid,
        });

        Ok(())
    }

    pub fn get_model(&self) -> Vec<CommandPipelineProcessModel> {
        self.commands
            .iter()
            .map(|data| CommandPipelineProcessModel {
                command: data.command.clone(),
                pid: data.process.id(),
            })
            .collect()
    }

    pub fn get_command(&self, i: usize) -> Option<&str> {
        self.commands.get(i).map(|data| data.command.as_str())
    }

    pub fn id(&self) -> &T {
        &self.id
    }

    pub fn pids(&self) -> Vec<Option<u32>> {
        self.commands.iter().map(|data| data.process.id()).collect()
    }

    pub fn take_endpoints(&mut self) -> (CommandPipelineInput<T>, CommandPipelineOutput<T>) {
        let _enter = self.span.enter();
        let input = CommandPipelineInput::new(
            self.stdin.take().unwrap(),
            self.id.clone(),
            self.sender.subscribe(),
        );
        let output = CommandPipelineOutput::new(
            self.stdout.take().unwrap(),
            self.id.clone(),
            self.sender.subscribe(),
        );
        (input, output)
    }

    pub fn kill(&mut self) {
        // We can safely use Span::enter() in non-async functions.
        let _enter = self.span.enter();
        if self.commands.is_empty() {
            // Already terminated.
            return;
        }
        for mut data in std::mem::take(&mut self.commands).into_iter() {
            match data.process.id() {
                Some(_) => {
                    // Always send a SIGKILL to the process.
                    tracing::debug!(pid = data.pid, "Kill");
                    let _ = data.process.start_kill();

                    // It's necessary to wait for the process termination because
                    // the process may  exclusively use resources like a tuner
                    // device.
                    //
                    // However, we cannot wait for any async task here, so we wait
                    // for the process termination in a busy loop.
                    while let Ok(None) = data.process.try_wait() {
                        sleep(*COMMAND_PIPELINE_TERMINATION_WAIT_NANOS);
                    }
                }
                None => tracing::debug!(pid = data.pid, "Already terminated"),
            }
        }
        tracing::debug!("Terminated");
    }

    // Don't use Span::enter() in async functions.
    pub async fn wait(&mut self) -> Vec<io::Result<ExitStatus>> {
        let mut result = Vec::with_capacity(self.commands.len());
        self.span
            .in_scope(|| tracing::debug!("Wait for termination..."));
        let commands = std::mem::take(&mut self.commands);
        for mut data in commands.into_iter() {
            self.span
                .in_scope(|| tracing::debug!("Wait for termination..."));
            result.push(data.process.wait().instrument(self.span.clone()).await);
        }
        self.span.in_scope(|| tracing::debug!("Terminated"));
        result
    }

    fn log_to_console<C: Spawn>(&mut self, ctx: &C) {
        let streams =
            self.commands
                .iter_mut()
                .filter_map(|data| match data.process.stderr.take() {
                    Some(stderr) => Some(logging::new_stream(stderr, data.pid)),
                    None => None,
                });
        let mut stream = futures::stream::select_all(streams);
        let fut = async move {
            while let Some(Ok((raw, pid))) = stream.next().await {
                // data may contain non-utf8 sequence.
                let log = String::from_utf8_lossy(&raw);
                tracing::debug!(pid, "{}", log.trim_end());
            }
        };
        let _enter = self.span.enter();
        ctx.spawn_task(fut);
    }

    fn log_to_file<C: Spawn>(&mut self, log_file: &Path, ctx: &C) {
        let streams =
            self.commands
                .iter_mut()
                .filter_map(|data| match data.process.stderr.take() {
                    Some(stderr) => Some(logging::new_stream(stderr, data.pid)),
                    None => None,
                });
        let mut stream = futures::stream::select_all(streams);
        let log_file = log_file.to_owned();
        let fut = async move {
            let mut file = File::create(&log_file).await.ok();
            if file.is_none() {
                tracing::error!(?log_file, "Failed to create, logs will be output to STDOUT");
            }
            while let Some(Ok((raw, pid))) = stream.next().await {
                match file {
                    Some(ref mut file) => {
                        if let Err(err) = file.write_all(&raw).await {
                            tracing::error!(?err, ?log_file, "Failed to write, the log is lost");
                        }
                        if let Err(err) = file.write_all(b"\n").await {
                            tracing::error!(?err, ?log_file, "Failed to write, the log is lost");
                        }
                    }
                    None => {
                        // data may contain non-utf8 sequence.
                        let log = String::from_utf8_lossy(&raw);
                        tracing::debug!(pid, "{}", log.trim_end())
                    }
                }
            }
        };
        let _enter = self.span.enter();
        ctx.spawn_task(fut);
    }
}

impl<T> Drop for CommandPipeline<T>
where
    T: Clone + Unpin,
{
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Debug)]
pub struct CommandPipelineProcessModel {
    pub command: String,
    pub pid: Option<u32>,
}

// input-side endpoint

pub struct CommandPipelineInput<T>
where
    T: Unpin,
{
    inner: ChildStdin,
    _pipeline_id: T,
    receiver: broadcast::Receiver<()>,
}

impl<T> CommandPipelineInput<T>
where
    T: Unpin,
{
    fn new(inner: ChildStdin, pipeline_id: T, receiver: broadcast::Receiver<()>) -> Self {
        Self {
            inner,
            _pipeline_id: pipeline_id,
            receiver,
        }
    }

    fn has_pipeline_broken(&mut self) -> bool {
        !matches!(
            self.receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        )
    }
}

impl<T> AsyncWrite for CommandPipelineInput<T>
where
    T: Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.has_pipeline_broken() {
            Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "")))
        } else {
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        if self.has_pipeline_broken() {
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut self.inner).poll_flush(cx)
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        if self.has_pipeline_broken() {
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }
}

// output-side endpoint

pub struct CommandPipelineOutput<T>
where
    T: Unpin,
{
    inner: ChildStdout,
    _pipeline_id: T,
    receiver: broadcast::Receiver<()>,
}

impl<T> CommandPipelineOutput<T>
where
    T: Unpin,
{
    fn new(inner: ChildStdout, pipeline_id: T, receiver: broadcast::Receiver<()>) -> Self {
        Self {
            inner,
            _pipeline_id: pipeline_id,
            receiver,
        }
    }

    fn has_pipeline_broken(&mut self) -> bool {
        !matches!(
            self.receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        )
    }
}

impl<T> AsyncRead for CommandPipelineOutput<T>
where
    T: Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        if self.has_pipeline_broken() {
            Poll::Ready(Ok(())) // EOF
        } else {
            Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }
}

pub struct CommandBuilder<'a> {
    inner: Command,
    command: &'a str,
    logging: CommandLogging,
}

impl<'a> CommandBuilder<'a> {
    pub fn new(command: &'a str) -> Result<Self, Error> {
        let words = match shell_words::split(command) {
            Ok(words) => words,
            Err(_) => return Err(Error::UnableToParse(command.to_string())),
        };

        let (prog, args) = words.split_first().unwrap();

        let mut inner = Command::new(prog);
        inner.args(args);

        Ok(CommandBuilder {
            inner,
            command,
            logging: CommandLogging::Default,
        })
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, io: T) -> &mut Self {
        self.inner.stdin(io);
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, io: T) -> &mut Self {
        self.inner.stdout(io);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    pub fn logging(&mut self, logging: CommandLogging) -> &mut Self {
        self.logging = logging;
        self
    }

    pub fn spawn(&mut self) -> Result<Child, Error> {
        if self.logging.is_enabled() {
            self.inner.stderr(Stdio::piped());
        } else {
            self.inner.stderr(Stdio::null());
        }

        let child = self
            .inner
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| Error::UnableToSpawn(self.command.to_string(), err))?;

        let pid = child.id().unwrap();

        // At this point, tracing_subscriber::filter::EnvFilter doesn't support
        // expressions for filtering nested spans.  For example, there are two
        // spans `parent` and `child`, and `parent` contains `child`.  In this
        // case, `RUST_LOG='[parent],[child]'` shows logs from the both.
        //
        // See https://github.com/tokio-rs/tracing/issues/722 for more details.
        //
        // Therefore, we don't create a `cmd` span for each process spawned in
        // this function.  Instead, put the process metadata to each event.
        //
        //let span = tracing::debug_span!("cmd", pid);
        //let _enter = span.enter();

        tracing::debug!(command = self.command, pid, "Spawned");
        Ok(child)
    }
}

#[derive(Clone, Copy)]
pub enum CommandLogging {
    Default,
    Enabled,
    Disabled,
}

impl CommandLogging {
    const ENV_NAME: &'static str = if cfg!(test) {
        "MIRAKC_DEBUG_CHILD_PROCESS_FOR_TEST"
    } else {
        "MIRAKC_DEBUG_CHILD_PROCESS"
    };

    fn is_enabled(&self) -> bool {
        match self {
            Self::Default => env::var_os(Self::ENV_NAME).is_some(),
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }
}

mod logging {
    use tokio::io::AsyncRead;
    use tokio_util::codec::AnyDelimiterCodec;
    use tokio_util::codec::Decoder;
    use tokio_util::codec::FramedRead;

    pub const DELIM: u8 = b'\n';
    const BUFSIZE: usize = 4096;

    // BufReader::read_line() gets stuck if an invalid character sequence is
    // found.  Therefore, we use AnyDelimiterCodec here in order to avoid
    // such a situation.
    fn new_codec(pid: u32) -> LogCodec {
        LogCodec::new(pid)
    }

    pub fn new_stream<R>(read: R, pid: u32) -> FramedRead<R, LogCodec>
    where
        R: AsyncRead,
    {
        FramedRead::new(read, new_codec(pid))
    }

    pub struct LogCodec {
        inner: AnyDelimiterCodec,
        pid: u32,
    }

    impl LogCodec {
        fn new(pid: u32) -> Self {
            let inner = AnyDelimiterCodec::new_with_max_length(vec![DELIM], vec![DELIM], BUFSIZE);
            LogCodec { inner, pid }
        }
    }

    impl Decoder for LogCodec {
        type Item = (bytes::Bytes, u32);
        type Error = <AnyDelimiterCodec as Decoder>::Error;

        fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            let log = self.inner.decode(src)?;
            Ok(log.map(|raw| (raw, self.pid)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use test_log::test;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test(tokio::test)]
    async fn test_command_pipeline_builder_defaul_envs() {
        let mut commands = vec![];

        let expected = std::env::var("MIRAKC_ARIB_LOG").unwrap_or_default();
        commands.push(format!(r#"sh -c 'test "$MIRAKC_ARIB_LOG" = "{expected}"'"#));

        let expected = std::env::var("MIRAKC_ARIB_LOG_NO_TIMESTAMP").unwrap_or_default();
        commands.push(format!(
            r#"sh -c 'test "$MIRAKC_ARIB_LOG_NO_TIMESTAMP" = "{expected}"'"#
        ));

        let builder = CommandPipelineBuilder::new(commands, 0u8, "test");
        let mut pipeline = builder.build(&actlet::stubs::Context::default()).unwrap();

        let status = pipeline.wait().await;
        assert_matches!(status[0], Ok(status) => {
            assert!(status.success());
        });

        assert_matches!(status[1], Ok(status) => {
            assert!(status.success());
        });
    }

    #[test(tokio::test)]
    async fn test_command_pipeline_bilder_set_log_filter() {
        let mut builder = CommandPipelineBuilder::new(
            vec![r#"sh -c 'test "$MIRAKC_ARIB_LOG" = off'"#.to_string()],
            0u8,
            "test",
        );
        builder.set_log_filter("off");
        let mut pipeline = builder.build(&actlet::stubs::Context::default()).unwrap();
        let status = pipeline.wait().await;
        assert_matches!(status[0], Ok(status) => {
            assert!(status.success());
        });
    }

    #[test(tokio::test)]
    async fn test_command_pipeline_bilder_set_log_file() {
        let mut builder = CommandPipelineBuilder::new(
            vec![r#"sh -c 'test "$MIRAKC_ARIB_LOG_NO_TIMESTAMP" = 0'"#.to_string()],
            0u8,
            "test",
        );
        builder.set_log_file("/dev/null");
        let mut pipeline = builder.build(&actlet::stubs::Context::default()).unwrap();
        let status = pipeline.wait().await;
        assert_matches!(status[0], Ok(status) => {
            assert!(status.success());
        });
    }

    #[test(tokio::test)]
    async fn test_command_builder() {
        let result = CommandBuilder::new("true")
            .and_then(|mut builder| builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn());
        assert!(result.is_ok());
        let _ = result.unwrap().wait().await;

        let result = CommandBuilder::new("'")
            .and_then(|mut builder| builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Unable to parse: '");

        // FIXME(ci): The following test randomly fails in the `cross-build` workflow for an
        // unclear reason.  We have not succeeded reproducing the failure locally, but the command
        // that doesn't exist can be spawned successfully for some mysterious reason...
        let skip = match std::env::var_os("MIRAKC_CI_CROSS_BUILD_WORKAROUND") {
            Some(v) => v == "true",
            None => false,
        };
        if !skip {
            let result = CommandBuilder::new("command-not-found").and_then(|mut builder| {
                builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn()
            });
            assert!(result.is_err());
            assert_matches!(
                result.unwrap_err(),
                Error::UnableToSpawn(_, io::Error { .. })
            );
        }
    }

    #[test(tokio::test)]
    async fn test_pipeline_io() {
        use futures::task::noop_waker;

        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (mut input, mut output) = pipeline.take_endpoints();

        let result = input.write_all(b"hello").await;
        assert!(result.is_ok());

        let result = input.flush().await;
        assert!(result.is_ok());

        let result = input.shutdown().await;
        assert!(result.is_ok());

        let mut buf = [0u8; 6];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello\0", &buf);

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let mut rbuf = ReadBuf::new(&mut buf);
        let poll = Pin::new(&mut output).poll_read(&mut cx, &mut rbuf);
        assert!(poll.is_pending());

        drop(input);

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(0, result.unwrap());
    }

    #[test(tokio::test)]
    async fn test_pipeline_mulpile_commands() {
        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string(), "cat".to_string(), "cat".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (mut input, mut output) = pipeline.take_endpoints();

        let result = input.write_all(b"hello").await;
        assert!(result.is_ok());

        let result = input.flush().await;
        assert!(result.is_ok());

        let result = input.shutdown().await;
        assert!(result.is_ok());

        let mut buf = [0u8; 6];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello\0", &buf);
    }

    #[test(tokio::test)]
    async fn test_pipeline_write_1mb() {
        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (mut input, mut output) = pipeline.take_endpoints();

        let handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            let result = output.read_to_end(&mut buf).await;
            assert!(result.is_ok());
            assert_eq!(1_000_000, result.unwrap());
        });

        let data = vec![0u8; 1_000_000];
        let result = input.write_all(&data).await;
        assert!(result.is_ok());

        let result = input.flush().await;
        assert!(result.is_ok());

        let result = input.shutdown().await;
        assert!(result.is_ok());

        drop(input);

        handle.await.unwrap();
    }

    #[test(tokio::test)]
    async fn test_pipeline_input_dropped() {
        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (mut input, mut output) = pipeline.take_endpoints();

        let _ = input.write_all(b"hello").await;

        drop(input);

        let mut buf = [0u8; 5];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello", &buf);
    }

    #[test(tokio::test)]
    async fn test_pipeline_output_dropped() {
        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (mut input, output) = pipeline.take_endpoints();

        drop(output);

        let result = input.write_all(b"hello").await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn test_pipeline_broken() {
        use tokio::sync::oneshot;

        struct Wrapper<W> {
            inner: W,
            sender: Option<oneshot::Sender<()>>,
        }

        impl<W> Wrapper<W> {
            fn new(inner: W, sender: Option<oneshot::Sender<()>>) -> Self {
                Wrapper { inner, sender }
            }
        }

        impl<W: AsyncWrite + Unpin> AsyncWrite for Wrapper<W> {
            fn poll_write(
                mut self: Pin<&mut Self>,
                cx: &mut Context,
                buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
                if poll.is_pending() {
                    if let Some(sender) = self.sender.take() {
                        let _ = sender.send(());
                    }
                }
                poll
            }

            fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
                Pin::new(&mut self.inner).poll_flush(cx)
            }

            fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
                Pin::new(&mut self.inner).poll_shutdown(cx)
            }
        }

        let mut pipeline = spawn_pipeline(
            vec!["sh -c 'exec 0<&-;'".to_string()],
            0u8,
            "test",
            &actlet::stubs::Context::default(),
        )
        .unwrap();
        let (input, _) = pipeline.take_endpoints();

        let (tx, rx) = oneshot::channel();
        let mut wrapper = Wrapper::new(input, Some(tx));

        let handle = tokio::spawn(async move {
            let data = [0u8; 8192];
            loop {
                if wrapper.write_all(&data).await.is_err() {
                    return;
                }
            }
        });

        // Wait until pending in `wrapper.write_all()`.
        let _ = rx.await;

        // Kill the spawned process while pending in `wrapper.write_all()`.
        drop(pipeline);

        // Will block until the task fails if the line above is removed.
        let result = handle.await;
        assert!(result.is_ok());
    }
}
