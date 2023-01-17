mod rebuild_timeshift;
mod serve;

use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

#[derive(Parser)]
#[command(author, version, about)]
struct Opt {
    /// Path to a configuration file in a YAML format.
    ///
    /// The MIRAKC_CONFIG environment variable is used if this option is not
    /// specified.  Its value has to be an absolute path.
    ///
    /// See docs/config.md for details of the YAML format.
    #[arg(short, long, env = "MIRAKC_CONFIG")]
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
    /// Rebuild timeshift files.
    RebuildTimeshift(rebuild_timeshift::Opt),
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();

    mirakc_core::tracing_ext::init_tracing(match opt.log_format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    });

    let config = mirakc_core::config::load(&opt.config);

    match opt.command {
        Some(Command::RebuildTimeshift(opt)) => rebuild_timeshift::main(config, opt).await,
        None => serve::main(config).await,
    }
}
