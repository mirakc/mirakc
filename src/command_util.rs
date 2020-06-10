use std::env;
use std::io;
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::process::{Command, Child, ChildStdin, ChildStdout, Stdio};
use std::task::{Poll, Context};

use failure::Fail;
use tokio::prelude::*;
use tokio::io::BufReader;

use crate::tokio_snippet;
use crate::tuner::TunerSubscriptionId as CommandPipelineId;

pub fn spawn_process(
    command: &str,
    input: Stdio,
) -> Result<Child, Error> {
    let words = match shell_words::split(command) {
        Ok(words) => words,
        Err(_) => return Err(Error::UnableToParse(command.to_string())),
    };
    let words: Vec<&str> = words.iter().map(|word| &word[..]).collect();
    let (prog, args) = words.split_first().unwrap();
    let debug_child_process =
        env::var_os("MIRAKC_DEBUG_CHILD_PROCESS").is_some();
    let stderr = if debug_child_process {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(prog)
        .args(args)
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|err| Error::UnableToSpawn(command.to_string(), err))?;
    if cfg!(not(test)) {
        if debug_child_process {
            let child_id = child.id();
            let child_stderr = tokio_snippet::stdio(child.stderr.take())
                .map_err(|err| Error::AsyncIoRegistrationFailure(err))?.unwrap();
            tokio::spawn(async move {
                let mut reader = BufReader::new(child_stderr);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line).await {
                    if n == 0 {  // EOF
                        break;
                    }
                    log::debug!("stderr#{}: {}", child_id, line.trim());
                    line.clear();
                }
            });
        }
    }
    Ok(child)
}

// Spawn processes for input commands and build a pipeline, then returns
// endpoints of the pipeline.
pub fn spawn_pipeline(
    commands: Vec<String>,
    id: CommandPipelineId,
) -> Result<CommandPipelineIoPair, Error> {
    let mut builder = CommandPipelineBuilder::new(id);
    for command in commands.into_iter() {
        builder.spawn(command)?;
    }
    builder.build()
}

// errors

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Unable to parse: {}", 0)]
    UnableToParse(String),
    #[fail(display = "Unable to spawn: {}: {}", 0, 1)]
    UnableToSpawn(String, io::Error),
    #[fail(display = "Async I/O registration failure: {}", 0)]
    AsyncIoRegistrationFailure(io::Error),
}

// Alias for making developer's life easier.
type CommandPipelineIoPair = (CommandPipelineInput, CommandPipelineOutput);

// pipeline builder

struct CommandPipelineBuilder {
    id: CommandPipelineId,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    commands: Vec<CommandData>,
}

impl CommandPipelineBuilder {
    pub fn new(id: CommandPipelineId) -> Self {
        CommandPipelineBuilder {
            id,
            stdin: None,
            stdout: None,
            commands: Vec::new(),
        }
    }

    fn spawn(&mut self, command: String) -> Result<(), Error> {
        let input = if self.stdout.is_none() {
            Stdio::piped()
        } else {
            Stdio::from(self.stdout.take().unwrap())
        };

        let mut process = spawn_process(&command, input)?;
        log::debug!("{}: Spawned {}: `{}`",
                    self.id, process.id(), command);

        if self.stdin.is_none() {
            self.stdin = process.stdin.take();
        }
        self.stdout = process.stdout.take();
        self.commands.push(CommandData{ command, process });

        Ok(())
    }

    fn build(mut self) -> Result<CommandPipelineIoPair, Error> {
        let pipeline = CommandPipeline::new(self.commands, self.id);
        let input = CommandPipelineInput::new(
            Self::make_childio_async(self.stdin.take())?, pipeline.id);
        let output = CommandPipelineOutput::new(
            Self::make_childio_async(self.stdout.take())?, pipeline);
        Ok((input, output))
    }

    fn make_childio_async<T: AsRawFd>(
        childio: Option<T>
    ) -> Result<tokio_snippet::ChildIo<T>, Error> {
        match tokio_snippet::stdio(childio) {
            Ok(childio) => Ok(childio.expect("Should not be None")),
            Err(err) => Err(Error::AsyncIoRegistrationFailure(err)),
        }
    }
}

// pipeline

struct CommandPipeline {
    commands: Vec<CommandData>,
    id: CommandPipelineId,
}

struct CommandData {
    command: String,
    process: Child,
}

impl CommandPipeline {
    fn new(commands: Vec<CommandData>, id: CommandPipelineId) -> Self {
        Self { commands, id }
    }
}

impl Drop for CommandPipeline {
    fn drop(&mut self) {
        for data in self.commands.iter_mut() {
            let _ = data.process.kill();
            let _ = data.process.wait();
            log::debug!("{}: Killed {}: `{}`",
                       self.id, data.process.id(), data.command);
        }
    }
}

// input-side endpoint

pub struct CommandPipelineInput {
    inner: tokio_snippet::ChildIo<ChildStdin>,
    _pipeline_id: CommandPipelineId,
}

impl CommandPipelineInput {
    fn new(
        inner: tokio_snippet::ChildIo<ChildStdin>,
        _pipeline_id: CommandPipelineId,
    ) -> Self {
        Self { inner, _pipeline_id }
    }
}

impl AsyncWrite for CommandPipelineInput {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8]
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// output-side endpoint

pub struct CommandPipelineOutput {
    inner: tokio_snippet::ChildIo<ChildStdout>,
    _pipeline: CommandPipeline,
}

impl CommandPipelineOutput {
    fn new(
        inner: tokio_snippet::ChildIo<ChildStdout>,
        pipeline: CommandPipeline,
    ) -> Self {
        Self { inner, _pipeline: pipeline }
    }
}

impl AsyncRead for CommandPipelineOutput {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8]
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matches::*;

    #[test]
    fn test_spawn_process() {
        let result = spawn_process("sh -c 'exit 0;'", Stdio::null());
        assert!(result.is_ok());
        let _ = result.unwrap().wait();

        let result = spawn_process("'", Stdio::null());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Unable to parse: '");

        let result = spawn_process("command-not-found", Stdio::null());
        assert!(result.is_err());
        assert_matches!(result.unwrap_err(),
                        Error::UnableToSpawn(_, io::Error {..}));
    }

    #[tokio::test]
    async fn test_pipeline() {
        use futures::task::noop_waker;

        let (mut input, mut output) = spawn_pipeline(vec![
            "cat".to_string()
        ], Default::default()).unwrap();

        let result = input.write_all(b"hello").await;
        assert!(result.is_ok());

        let result = input.flush().await;
        assert!(result.is_ok());

        let result = input.shutdown().await;
        assert!(result.is_ok());

        let mut buf = [0; 6];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello\0", &buf);

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let poll =  Pin::new(&mut output).poll_read(&mut cx, &mut buf);
        assert!(poll.is_pending());

        drop(input);

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(0, result.unwrap());
    }

    #[tokio::test]
    async fn test_pipeline_input_dropped() {
        let (mut input, mut output) = spawn_pipeline(vec![
            "cat".to_string(),
        ], Default::default()).unwrap();

        let _ = input.write_all(b"hello").await;

        drop(input);

        let mut buf = [0; 5];

        let result = output.read(&mut buf).await;
        assert!(result.is_ok());
        assert_eq!(5, result.unwrap());
        assert_eq!(b"hello", &buf);
    }

    #[tokio::test]
    async fn test_pipeline_output_dropped() {
        let (mut input, output) = spawn_pipeline(vec![
            "cat".to_string(),
            "sleep 5".to_string(),
        ], Default::default()).unwrap();

        drop(output);

        let result = input.write_all(b"hello").await;
        assert!(result.is_err());
        assert_eq!(io::ErrorKind::BrokenPipe, result.unwrap_err().kind());
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
                buf: &[u8]
            ) -> Poll<io::Result<usize>> {
                let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
                if poll.is_pending() {
                    if let Some(sender) = self.sender.take() {
                        let _ = sender.send(());
                    }
                }
                poll
            }

            fn poll_flush(
                mut self: Pin<&mut Self>,
                cx: &mut Context
            ) -> Poll<io::Result<()>> {
                Pin::new(&mut self.inner).poll_flush(cx)
            }

            fn poll_shutdown(
                mut self: Pin<&mut Self>,
                cx: &mut Context
            ) -> Poll<io::Result<()>> {
                Pin::new(&mut self.inner).poll_shutdown(cx)
            }
        }

        let (input, output) = spawn_pipeline(vec![
            "sh -c 'exec 0<&-;'".to_string(),
        ], Default::default()).unwrap();

        let (tx, rx) = oneshot::channel();
        let mut wrapper = Wrapper::new(input, Some(tx));

        let handle = tokio::spawn(async move {
            let data = [0; 8192];
            loop {
                if wrapper.write_all(&data).await.is_err() {
                    return;
                }
            }
        });

        // Wait until pending in `wrapper.write_all()`.
        let _ = rx.await;

        // Kill the spawned process while pending in `wrapper.write_all()`.
        drop(output);

        // Will block until the task fails if the line above is removed.
        let result = handle.await;
        assert!(result.is_ok());
    }
}
