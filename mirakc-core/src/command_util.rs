use std::convert::TryInto;
use std::env;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::process::ExitStatus;
use std::process::Stdio;
use std::task::Context;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tracing::Instrument;
use tracing::Span;

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
        .map(|nanos| Duration::from_nanos(nanos))
        .unwrap_or(COMMAND_PIPELINE_TERMINATION_WAIT_NANOS_DEFAULT);
    tracing::debug!(
        COMMAND_PIPELINE_TERMINATION_WAIT_NANOS = %humantime::format_duration(nanos),
    );
    nanos
});

// Spawn processes for input commands and build a pipeline, then returns it.
// Input and output endpoints can be took from the pipeline only once
// respectively.
pub fn spawn_pipeline<T>(
    commands: Vec<String>,
    id: T,
    label: &'static str,
) -> Result<CommandPipeline<T>, Error>
where
    T: fmt::Display + Clone + Unpin,
{
    let span = tracing::debug_span!(parent: None, "pipeline", %id, label);
    // We can safely use Span::enter() in non-async functions.
    let _enter = span.enter();
    let mut pipeline = CommandPipeline::new(id, span.clone());
    for command in commands.into_iter() {
        pipeline.spawn(command)?;
    }
    pipeline.log_to_console();
    Ok(pipeline)
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

    fn spawn(&mut self, command: String) -> Result<(), Error> {
        let input = if self.stdout.is_none() {
            Stdio::piped()
        } else {
            self.stdout.take().unwrap().try_into()?
        };

        let mut process = CommandBuilder::new(&command)?
            .stdin(input)
            .stdout(Stdio::piped())
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
            self.stdin.take().unwrap().try_into().unwrap(),
            self.id.clone(),
            self.sender.subscribe(),
        );
        let output = CommandPipelineOutput::new(
            self.stdout.take().unwrap().try_into().unwrap(),
            self.id.clone(),
            self.sender.subscribe(),
        );
        (input, output)
    }

    // Don't use Span::enter() in async functions.
    pub async fn wait(&mut self) -> Vec<io::Result<ExitStatus>> {
        let mut result = Vec::with_capacity(self.commands.len());
        self.span
            .in_scope(|| tracing::debug!("Wait for termination..."));
        let commands = std::mem::replace(&mut self.commands, vec![]);
        for mut data in commands.into_iter() {
            self.span
                .in_scope(|| tracing::debug!("Wait for termination..."));
            result.push(data.process.wait().instrument(self.span.clone()).await);
        }
        self.span.in_scope(|| tracing::debug!("Terminated"));
        result
    }

    fn log_to_console(&mut self) {
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
        tokio::spawn(fut.instrument(self.span.clone()));
    }
}

impl<T> Drop for CommandPipeline<T>
where
    T: Clone + Unpin,
{
    fn drop(&mut self) {
        // We can safely use Span::enter() in non-async functions.
        let _enter = self.span.enter();
        if self.commands.is_empty() {
            // Already terminated.
            return;
        }
        for mut data in std::mem::replace(&mut self.commands, vec![]).into_iter() {
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
                    loop {
                        match data.process.try_wait() {
                            Ok(None) => (),
                            _ => break,
                        }
                        sleep(*COMMAND_PIPELINE_TERMINATION_WAIT_NANOS);
                    }
                }
                None => tracing::debug!(pid = data.pid, "Already terminated"),
            }
        }
        tracing::debug!("Terminated");
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
        match self.receiver.try_recv() {
            Err(err) if err == broadcast::error::TryRecvError::Empty => false,
            _ => true,
        }
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
        match self.receiver.try_recv() {
            Err(err) if err == broadcast::error::TryRecvError::Empty => false,
            _ => true,
        }
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

    pub fn enable_logging(&mut self) -> &mut Self {
        self.logging = CommandLogging::Enabled;
        self
    }

    pub fn spawn(&mut self) -> Result<Child, Error> {
        if self.logging.is_enabled() {
            self.inner.stderr(Stdio::piped());
        } else {
            self.inner.stderr(Stdio::null());
        }

        let mut child = self
            .inner
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| Error::UnableToSpawn(self.command.to_string(), err))?;

        let pid = child.id().unwrap();

        // At this point, tracing_subscriber::filter::EnvFilter doesn't support
        // expressions for filtering nested spans.  For example, there are two
        // spans `parent` and `child` and `parent` contains `child`.  In this
        // case, `RUST_LOG='[parent],[child]'` shows logs from the both.
        //
        // See https://github.com/tokio-rs/tracing/issues/722 for more details.
        //
        // Therefore, we don't stop creating a `cmd` span for each process
        // spawned in this function.  Instead, put the process metadata to each
        // event.
        //
        //let span = tracing::debug_span!("cmd", pid);
        //let _enter = span.enter();

        if self.logging.is_default() {
            if let Some(stderr) = child.stderr.take() {
                let fut = async move {
                    let mut stream = logging::new_stream(stderr, pid);
                    while let Some(Ok((raw, pid))) = stream.next().await {
                        // data may contain non-utf8 sequence.
                        let log = String::from_utf8_lossy(&raw);
                        tracing::debug!(pid, "{}", log.trim_end());
                    }
                };
                tokio::spawn(fut.in_current_span());
            }
        }

        tracing::debug!(command = self.command, pid, "Spawned");
        Ok(child)
    }
}

enum CommandLogging {
    Default,
    Enabled,
}

impl CommandLogging {
    const ENV_NAME: &'static str = if cfg!(test) {
        "MIRAKC_DEBUG_CHILD_PROCESS_FOR_TEST"
    } else {
        "MIRAKC_DEBUG_CHILD_PROCESS"
    };

    fn is_default(&self) -> bool {
        match self {
            Self::Default => true,
            _ => false,
        }
    }

    fn is_enabled(&self) -> bool {
        match self {
            Self::Default => env::var_os(Self::ENV_NAME).is_some(),
            Self::Enabled => true,
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

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_command_builder() {
        let result = CommandBuilder::new("true")
            .and_then(|mut builder| builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn());
        assert!(result.is_ok());
        let _ = result.unwrap().wait().await;

        let result = CommandBuilder::new("'")
            .and_then(|mut builder| builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Unable to parse: '");

        let result = CommandBuilder::new("command-not-found")
            .and_then(|mut builder| builder.stdin(Stdio::null()).stdout(Stdio::piped()).spawn());
        assert!(result.is_err());
        assert_matches!(
            result.unwrap_err(),
            Error::UnableToSpawn(_, io::Error { .. })
        );
    }

    #[tokio::test]
    async fn test_pipeline_io() {
        use futures::task::noop_waker;

        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8, "test").unwrap();
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

    #[tokio::test]
    async fn test_pipeline_mulpile_commands() {
        let mut pipeline = spawn_pipeline(
            vec!["cat".to_string(), "cat".to_string(), "cat".to_string()],
            0u8,
            "test",
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

    #[tokio::test]
    async fn test_pipeline_write_1mb() {
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8, "test").unwrap();
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

        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_pipeline_input_dropped() {
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8, "test").unwrap();
        let (mut input, mut output) = pipeline.take_endpoints();

        let _ = input.write_all(b"hello").await;

        drop(input);

        let mut buf = [0u8; 5];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello", &buf);
    }

    #[tokio::test]
    async fn test_pipeline_output_dropped() {
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8, "test").unwrap();
        let (mut input, output) = pipeline.take_endpoints();

        drop(output);

        let result = input.write_all(b"hello").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
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

        let mut pipeline =
            spawn_pipeline(vec!["sh -c 'exec 0<&-;'".to_string()], 0u8, "test").unwrap();
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
// </coverage:exclude>
