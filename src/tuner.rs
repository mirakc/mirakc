use std::env;
use std::fmt;
use std::io;
use std::process::{Child, ChildStdout, Command, Stdio};

use bytes::Bytes;
use chrono::Duration;
use futures::Stream;
use log;
use mustache;
use tokio::prelude::Async;
use tokio::codec::{BytesCodec, Decoder};

use crate::config::TunerConfig;
use crate::error::Error;
use crate::messages::CloseTunerMessage;
use crate::models::{ChannelType, TunerModel, TunerUserModel};

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use tests::resource_manager_mock as resource_manager;
    } else {
        use crate::resource_manager;
    }
}

// tuner

pub struct Tuner {
    index: usize,
    name: String,
    channel_types: Vec<ChannelType>,
    detail: TunerDetail,
    session: Option<TunerSession>,
    session_id: u64,
}

impl Tuner {
    pub fn new(
        index: usize,
        config: &TunerConfig,
    ) -> Self {
        Tuner {
            index,
            name: config.name.clone(),
            channel_types: config.channel_types.clone(),
            detail: TunerDetail::Device { command: config.command.clone() },
            session: None,
            session_id: 0,
        }
    }

    #[inline]
    pub fn is_available(&self) -> bool {
        self.session.is_none()
    }

    #[inline]
    pub fn is_supported_type(&self, channel_type: ChannelType) -> bool {
        self.channel_types.contains(&channel_type)
    }

    #[inline]
    pub fn is_available_for(&self, channel_type: ChannelType) -> bool {
        self.is_available() && self.is_supported_type(channel_type)
    }

    pub fn can_take_over(&self, user: &TunerUser) -> bool {
        match self.session {
            Some(ref session) => user.priority > session.user.priority,
            None => true,
        }
    }

    pub fn take_over(
        &mut self,
        channel_type: ChannelType,
        channel: String,
        user: TunerUser,
        duration: Option<Duration>,
    ) -> Result<TunerOutput, Error> {
        log::info!("{} dispossesses users of tuner#{}", user, self.index);
        let id = self.session_id().unwrap();
        self.close(id)?;
        self.open(channel_type, channel, user, duration)
    }

    pub fn get_model(&self) -> TunerModel {
        let (command, pid, users) = match self.session {
            Some(ref session) => session.get_models(),
            None => (None, None, Vec::new()),
        };

        TunerModel {
            index: self.index,
            name: self.name.clone(),
            channel_types: self.channel_types.clone(),
            command,
            pid,
            users,
            is_available: true,
            is_remote: false,
            is_free: self.is_available(),
            is_using: !self.is_available(),
            is_fault: false,
        }
    }

    pub fn open(
        &mut self,
        channel_type: ChannelType,
        channel: String,
        user: TunerUser,
        duration: Option<Duration>,
    ) -> Result<TunerOutput, Error> {
        if self.session.is_some() {
            return Err(Error::TunerAlreadyUsed);
        }

        let command = self.get_command(channel_type, &channel, duration)?;

        let mut process = spawn(&command, Stdio::null())?;
        log::debug!("tuner#{}: Process#{} has been spawned by `{}`",
                    self.index, process.id(), command);
        let stdout = process.stdout.take();

        let id = self.session_id;
        self.session_id += 1;

        log::info!("tuner#{}: Started streaming to {}", self.index, user);
        self.session = Some(TunerSession { id, command, process, user });

        Ok(TunerOutput::new(self.index, id, stdout))
    }

    pub fn close(&mut self, session_id: u64) -> Result<(), Error> {
        match self.session {
            Some(ref session) if session.id == session_id => (),
            _ => return Err(Error::InvalidSession),
        }

        let mut session = self.session.take().unwrap();
        // Always kill the process and ignore the error.  Because there is no
        // method to check whether the process is alive or dead.
        session.process.kill().unwrap_or(());
        session.process.wait().unwrap();
        log::debug!("tuner#{}: Process#{} has terminated",
                    self.index, session.process.id());
        log::info!("tuner#{}: Stopped streaming to {}",
                   self.index, session.user);

        Ok(())
    }

    fn get_command(
        &self,
        channel_type: ChannelType,
        channel: &str,
        duration: Option<Duration>
    ) -> Result<String, Error> {
        let secs = match duration {
            Some(duration) => duration.num_seconds().to_string(),
            None => "-".to_string(),
        };

        match self.detail {
            TunerDetail::Device { ref command } => {
                let template = mustache::compile_str(command)?;
                let data = mustache::MapBuilder::new()
                    .insert("channel_type", &channel_type)?
                    .insert_str("channel", channel)
                    .insert_str("duration", secs)
                    .build();
                Ok(template.render_data_to_string(&data)?)
            }
        }
    }

    fn session_id(&self) -> Option<u64> {
        if let Some(TunerSession { id, .. }) = self.session {
            Some(id)
        } else {
            None
        }
    }
}

enum TunerDetail {
    Device {
        command: String,
    },
}

// session

struct TunerSession {
    id: u64,
    command: String,
    // Used for closing the tuner in order to take over the right to use it.
    process: Child,
    user: TunerUser,
}

impl TunerSession {
    fn get_models(&self) -> (Option<String>, Option<u32>, Vec<TunerUserModel>) {
        (
            Some(self.command.clone()),
            Some(self.process.id()),
            vec![self.user.get_model()],
        )
    }
}

// user

pub struct TunerUser {
    id: String,
    agent: Option<String>,
    priority: i32,
}

impl TunerUser {
    #[inline]
    pub fn new(id: String, agent: Option<String>, priority: i32) -> Self {
        TunerUser { id, agent, priority }
    }

    #[inline]
    pub fn background(id: String) -> Self {
        TunerUser { id, agent: None, priority: -1 }
    }

    fn get_model(&self) -> TunerUserModel {
        TunerUserModel {
            id: self.id.clone(),
            agent: self.agent.clone(),
            priority: self.priority
        }
    }
}

impl fmt::Display for TunerUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (priority={})", self.id, self.priority)
    }
}

// tuner output

pub struct TunerOutput {
    tuner_index: usize,
    session_id: u64,
    stdout: Option<ChildStdout>,
    processes: Vec<Child>,
}

impl TunerOutput {
    #[inline]
    pub fn new(
        tuner_index: usize,
        session_id: u64,
        stdout: Option<ChildStdout>,
    ) -> Self {
        TunerOutput { tuner_index, session_id, stdout, processes: Vec::new() }
    }

    // Assumed that a process spawned with `command` will exit when the input
    // stream is closed or the upstream process exits.
    pub fn pipe(mut self, command: &str) -> Result<Self, Error> {
        match self.stdout.take() {
            Some(stdout) => {
                let mut process = spawn(command, Stdio::from(stdout))?;
                log::debug!("stream#{}:{}: Process#{} has been spawned by `{}`",
                            self.tuner_index, self.session_id, process.id(),
                            command);
                self.stdout = process.stdout.take();
                self.processes.push(process);
                Ok(self)
            }
            None => Err(Error::from(broken_pipe_error())),
        }
    }

    pub fn into_stream(self) -> impl Stream<Item = Bytes, Error = Error> {
        log::debug!("stream#{}:{}: Converted into futures::Stream",
                    self.tuner_index, self.session_id);
        BytesCodec::new()
            .framed(self)
            .map(Bytes::from)
            .map_err(Error::from)
    }
}

impl io::Read for TunerOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.stdout {
            Some(ref mut stdout) => stdout.read(buf),
            None => Err(broken_pipe_error()),
        }
    }
}

impl io::Write for TunerOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        unimplemented!();
    }

    fn flush(&mut self) -> io::Result<()> {
        unimplemented!();
    }
}

impl tokio::io::AsyncRead for TunerOutput {}

impl tokio::io::AsyncWrite for TunerOutput {
    fn shutdown(&mut self) -> Result<Async<()>, tokio::io::Error> {
        unimplemented!();
    }
}

impl Drop for TunerOutput {
    fn drop(&mut self) {
        log::debug!("stream#{}:{}: Dropping...",
                    self.tuner_index, self.session_id);
        let msg = CloseTunerMessage {
            tuner_index: self.tuner_index,
            session_id: self.session_id,
        };
        resource_manager::close_tuner(msg);

        for process in self.processes.iter_mut() {
            process.kill().unwrap_or(());
            process.wait().unwrap();
            log::debug!("stream#{}:{}: Process#{} has terminated",
                        self.tuner_index, self.session_id, process.id());
        }
    }
}

// helpers

fn spawn(command: &str, input: Stdio) -> Result<Child, Error> {
    let words = shell_words::split(command)?;
    let words: Vec<&str> = words.iter().map(|word| &word[..]).collect();
    let (prog, args) = words.split_first().unwrap();
    let child = Command::new(prog)
        .args(args)
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(child_stderr())
        .spawn()?;
    Ok(child)
}

fn child_stderr() -> Stdio {
    match env::var_os("MIRAKC_DEBUG_CHILD_PROCESS") {
        Some(_) => Stdio::inherit(),
        None => Stdio::null(),
    }
}

#[inline]
fn broken_pipe_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "No output from upstream process")
}

#[cfg(test)]
mod tests {
    use super::*;
    use matches::assert_matches;

    pub mod resource_manager_mock {
        use super::*;
        pub fn close_tuner(_: CloseTunerMessage) {}
    }

    impl TunerConfig {
        fn dummy(command: String) -> Self {
            TunerConfig {
                name: String::new(),
                channel_types: vec![ChannelType::GR],
                command,
                disabled: false,
            }
        }
    }

    #[test]
    fn test_open() {
        {
            let config = TunerConfig::dummy("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            let user = TunerUser::new(String::new(), None, 0);
            let result = tuner.open(ChannelType::GR, String::new(), user, None);
            assert!(result.is_ok());
            let result = tuner.close(0);
            assert!(result.is_ok());
        }

        {
            let config = TunerConfig::dummy("true".to_string());
            let mut tuner = Tuner::new(0, &config);
            let user = TunerUser::new(String::new(), None, 0);
            let result = tuner.open(ChannelType::GR, String::new(), user, None);
            assert!(result.is_ok());
            let user = TunerUser::new(String::new(), None, 0);
            let result = tuner.open(ChannelType::GR, String::new(), user, None);
            assert_matches!(result.err(), Some(Error::TunerAlreadyUsed));
            let result = tuner.close(0);
            assert!(result.is_ok());
        }

        {
            let config = TunerConfig::dummy("cmd '".to_string());
            let mut tuner = Tuner::new(0, &config);
            let user = TunerUser::new(String::new(), None, 0);
            let result = tuner.open(ChannelType::GR, String::new(), user, None);
            assert_matches!(result.err(), Some(Error::CommandParseError(_)));
        }

        {
            let config = TunerConfig::dummy("no-such-command".to_string());
            let mut tuner = Tuner::new(0, &config);
            let user = TunerUser::new(String::new(), None, 0);
            let result = tuner.open(ChannelType::GR, String::new(), user, None);
            assert_matches!(result.err(), Some(Error::IoError(_)));
        }
    }

    #[test]
    fn test_close() {
        let config = TunerConfig::dummy("true".to_string());
        let mut tuner = Tuner::new(0, &config);
        let result = tuner.close(0);
        assert_matches!(result.err(), Some(Error::InvalidSession));
    }

    #[test]
    fn test_can_dispossess() {
        let config = TunerConfig::dummy("true".to_string());
        let mut tuner = Tuner::new(0, &config);

        let user = TunerUser::new(String::new(), None, 0);
        assert!(tuner.can_take_over(&user));

        tuner.open(ChannelType::GR, String::new(), user, None).unwrap();

        let user = TunerUser::new(String::new(), None, 0);
        assert!(!tuner.can_take_over(&user));

        let user = TunerUser::new(String::new(), None, 1);
        assert!(tuner.can_take_over(&user));

        tuner.close(0).unwrap();
    }

    #[test]
    fn test_dispossess() {
        let config = TunerConfig::dummy("true".to_string());
        let mut tuner = Tuner::new(0, &config);
        let user = TunerUser::new(String::new(), None, 0);
        tuner.open(ChannelType::GR, String::new(), user, None).ok();

        let user = TunerUser::new(String::new(), None, 1);
        let result =
            tuner.take_over(ChannelType::GR, String::new(), user, None);
        assert!(result.is_ok());
        tuner.close(0).ok();
    }

    #[test]
    fn test_session_id() {
        let config = TunerConfig::dummy("true".to_string());
        let mut tuner = Tuner::new(0, &config);
        assert_matches!(tuner.session_id(), None);
        let user = TunerUser::new(String::new(), None, 0);
        tuner.open(ChannelType::GR, String::new(), user, None).ok();
        assert_matches!(tuner.session_id(), Some(0));
        tuner.close(0).ok();
    }
}
