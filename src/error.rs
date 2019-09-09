use std::env;
use std::io;

use actix;
use failure::Fail;
use mustache;
use serde_json;
use serde_yaml;
use shell_words;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Unavailable")]
    Unavailable,
    #[fail(display = "Service not found")]
    ServiceNotFound,
    #[fail(display = "Program not found")]
    ProgramNotFound,
    #[fail(display = "Tuner already used")]
    TunerAlreadyUsed,
    #[fail(display = "Invalid session")]
    InvalidSession,
    #[fail(display = "IO error: {}", 0)]
    IoError(io::Error),
    #[fail(display = "JSON error: {}", 0)]
    JsonError(serde_json::Error),
    #[fail(display = "YAML error: {}", 0)]
    YamlError(serde_yaml::Error),
    #[fail(display = "Mailbox error: {}", 0)]
    MailboxError(actix::MailboxError),
    #[fail(display = "Mustache error: {}", 0)]
    MustacheError(mustache::Error),
    #[fail(display = "Command parse error: {}", 0)]
    CommandParseError(shell_words::ParseError),
    #[fail(display = "std::env error: {}", 0)]
    EnvVarError(env::VarError),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::IoError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::JsonError(err)
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(err: serde_yaml::Error) -> Self {
        Self::YamlError(err)
    }
}

impl From<actix::MailboxError> for Error {
    fn from(err: actix::MailboxError) -> Self {
        Self::MailboxError(err)
    }
}

impl From<mustache::Error> for Error {
    fn from(err: mustache::Error) -> Self {
        Self::MustacheError(err)
    }
}

impl From<mustache::EncoderError> for Error {
    fn from(err: mustache::EncoderError) -> Self {
        Self::MustacheError(mustache::Error::from(err))
    }
}

impl From<shell_words::ParseError> for Error {
    fn from(err: shell_words::ParseError) -> Self {
        Self::CommandParseError(err)
    }
}

impl From<env::VarError> for Error {
    fn from(err: env::VarError) -> Self {
        Self::EnvVarError(err)
    }
}
