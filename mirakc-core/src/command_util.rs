use std::convert::TryInto;
use std::env;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::process::Stdio;
use std::task::Context;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::BufReader;
use tokio::io::ReadBuf;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::broadcast;

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
        "MIRAKC_COMMAND_PIPELINE_TERMINATION_WAIT_NANOS: {}",
        humantime::format_duration(nanos)
    );
    nanos
});

pub fn spawn_process<I, O>(command: &str, input: I, output: O) -> Result<Child, Error>
where
    I: Into<Stdio>,
    O: Into<Stdio>,
{
    let words = match shell_words::split(command) {
        Ok(words) => words,
        Err(_) => return Err(Error::UnableToParse(command.to_string())),
    };
    let words: Vec<&str> = words.iter().map(|word| &word[..]).collect();
    let (prog, args) = words.split_first().unwrap();
    let debug_child_process = env::var_os("MIRAKC_DEBUG_CHILD_PROCESS").is_some();
    let stderr = if debug_child_process {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(prog)
        .args(args)
        .stdin(input)
        .stdout(output)
        .stderr(stderr)
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| Error::UnableToSpawn(command.to_string(), err))?;
    if cfg!(not(test)) {
        if let Some(stderr) = child.stderr.take() {
            let prog = prog.to_string();
            let child_id = child.id().unwrap();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut data = Vec::with_capacity(4096);
                // BufReader::read_line() gets stuck if an invalid character
                // sequence is found.  Therefore, we use BufReader::read_until()
                // here in order to avoid such a situation.
                while let Ok(n) = reader.read_until(0x0A, &mut data).await {
                    if n == 0 {
                        // EOF
                        break;
                    }
                    tracing::debug!(
                        "{}#{}: {}",
                        prog,
                        child_id,
                        // data may contain non-utf8 sequence.
                        String::from_utf8_lossy(&data).trim_end()
                    );
                    data.clear();
                }
            });
        }
    }
    Ok(child)
}

// Spawn processes for input commands and build a pipeline, then returns it.
// Input and output endpoints can be took from the pipeline only once
// respectively.
pub fn spawn_pipeline<T>(commands: Vec<String>, id: T) -> Result<CommandPipeline<T>, Error>
where
    T: fmt::Display + Clone + Unpin,
{
    let mut pipeline = CommandPipeline::new(id);
    for command in commands.into_iter() {
        pipeline.spawn(command)?;
    }
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
    T: fmt::Display + Clone + Unpin,
{
    id: T,
    sender: broadcast::Sender<()>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    commands: Vec<CommandData>,
}

struct CommandData {
    command: String,
    process: Child,
}

impl<T> CommandPipeline<T>
where
    T: fmt::Display + Clone + Unpin,
{
    fn new(id: T) -> Self {
        let (sender, _) = broadcast::channel(1);
        Self {
            id,
            sender,
            stdin: None,
            stdout: None,
            commands: Vec::new(),
        }
    }

    fn spawn(&mut self, command: String) -> Result<(), Error> {
        let input = if self.stdout.is_none() {
            Stdio::piped()
        } else {
            self.stdout.take().unwrap().try_into()?
        };

        let mut process = spawn_process(&command, input, Stdio::piped())?;
        tracing::debug!(
            "{}: Spawned {}: `{}`",
            self.id,
            process.id().unwrap(),
            command
        );

        if self.stdin.is_none() {
            self.stdin = process.stdin.take();
        }
        self.stdout = process.stdout.take();
        self.commands.push(CommandData { command, process });

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

    pub fn id(&self) -> &T {
        &self.id
    }

    pub fn pids(&self) -> Vec<Option<u32>> {
        self.commands.iter().map(|data| data.process.id()).collect()
    }

    pub fn take_endpoints(
        &mut self,
    ) -> Result<(CommandPipelineInput<T>, CommandPipelineOutput<T>), Error> {
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
        Ok((input, output))
    }
}

impl<T> Drop for CommandPipeline<T>
where
    T: fmt::Display + Clone + Unpin,
{
    fn drop(&mut self) {
        for data in self.commands.iter() {
            match data.process.id() {
                Some(pid) => tracing::debug!("{}: Kill {}: `{}`", self.id, pid, data.command),
                None => tracing::debug!("{}: Already terminated: {}", self.id, data.command),
            }
        }
        tracing::debug!("{}: Wait for the command pipeline termination...", self.id);
        for data in self.commands.iter_mut() {
            // Always send a SIGKILL to the process.
            let _ = data.process.start_kill();

            // It's necessary to wait for the process termination because the process may
            // exclusively use resources like a tuner device.
            //
            // However, we cannot wait for any async task here, so we wait for the process
            // termination in a busy loop.
            loop {
                match data.process.try_wait() {
                    Ok(None) => (),
                    _ => break,
                }
                sleep(*COMMAND_PIPELINE_TERMINATION_WAIT_NANOS);
            }
        }
        tracing::debug!("{}: The command pipeline has terminated", self.id);
    }
}

pub struct CommandPipelineProcessModel {
    pub command: String,
    pub pid: Option<u32>,
}

// input-side endpoint

pub struct CommandPipelineInput<T>
where
    T: fmt::Display + Clone + Unpin,
{
    inner: ChildStdin,
    _pipeline_id: T,
    receiver: broadcast::Receiver<()>,
}

impl<T> CommandPipelineInput<T>
where
    T: fmt::Display + Clone + Unpin,
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
    T: fmt::Display + Clone + Unpin,
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
    T: fmt::Display + Clone + Unpin,
{
    inner: ChildStdout,
    _pipeline_id: T,
    receiver: broadcast::Receiver<()>,
}

impl<T> CommandPipelineOutput<T>
where
    T: fmt::Display + Clone + Unpin,
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
    T: fmt::Display + Clone + Unpin,
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

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_spawn_process() {
        let result = spawn_process("sh -c 'exit 0;'", Stdio::null(), Stdio::piped());
        assert!(result.is_ok());
        let _ = result.unwrap().wait().await;

        let result = spawn_process("'", Stdio::null(), Stdio::piped());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Unable to parse: '");

        let result = spawn_process("command-not-found", Stdio::null(), Stdio::piped());
        assert!(result.is_err());
        assert_matches!(
            result.unwrap_err(),
            Error::UnableToSpawn(_, io::Error { .. })
        );
    }

    #[tokio::test]
    async fn test_pipeline_io() {
        use futures::task::noop_waker;

        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8).unwrap();
        let (mut input, mut output) = pipeline.take_endpoints().unwrap();

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
        )
        .unwrap();
        let (mut input, mut output) = pipeline.take_endpoints().unwrap();

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
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8).unwrap();
        let (mut input, mut output) = pipeline.take_endpoints().unwrap();

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
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8).unwrap();
        let (mut input, mut output) = pipeline.take_endpoints().unwrap();

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
        let mut pipeline = spawn_pipeline(vec!["cat".to_string()], 0u8).unwrap();
        let (mut input, output) = pipeline.take_endpoints().unwrap();

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

        let mut pipeline = spawn_pipeline(vec!["sh -c 'exec 0<&-;'".to_string()], 0u8).unwrap();
        let (input, _) = pipeline.take_endpoints().unwrap();

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
