mod openapi;
mod rebuild_timeshift;
mod serve;

use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " ", env!("VERGEN_GIT_SHA"));

#[derive(Parser)]
#[command(author, about, version = VERSION)]
struct CommandLine {
    /// Path to a configuration file.
    ///
    /// The MIRAKC_CONFIG environment variable is used if this option is not
    /// specified.  Its value has to be an absolute path.
    ///
    /// YAML (*.yml or *.yaml) or TOML (*.toml) formats are supported.
    ///
    /// See docs/config.md for details of the YAML/TOML format.
    #[arg(short, long, env = "MIRAKC_CONFIG", verbatim_doc_comment)]
    config: PathBuf,

    /// Logging format.
    #[arg(long, value_enum, env = "MIRAKC_LOG_FORMAT", default_value = "text")]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    Openapi(openapi::CommandLine),
    RebuildTimeshift(rebuild_timeshift::CommandLine),
}

#[tokio::main]
async fn main() {
    let cl = CommandLine::parse();

    // Disable logging in the `openapi` command if the OpenAPI document will be output to STDOUT.
    if !matches!(cl.command, Some(Command::Openapi(ref cl)) if !cl.has_file()) {
        mirakc_core::tracing_ext::init_tracing(match cl.log_format {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        });
    }

    let config = mirakc_core::config::load(&cl.config);

    match cl.command {
        Some(Command::Openapi(ref cl)) => openapi::main(config, cl).await,
        Some(Command::RebuildTimeshift(ref cl)) => rebuild_timeshift::main(config, cl).await,
        None => serve::main(config).await,
    }
}
