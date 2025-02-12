mod migrate;
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
    Migrate(migrate::CommandLine),
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
        Some(Command::Migrate(ref cl)) => migrate::main(&config, cl).await,
        Some(Command::Openapi(ref cl)) => openapi::main(config, cl).await,
        Some(Command::RebuildTimeshift(ref cl)) => rebuild_timeshift::main(config, cl).await,
        None => {
            if auto_migrate() {
                tracing::info!("Migrating existing data automatically...");
                migrate::main(&config, &Default::default()).await;
            }
            serve::main(config).await
        }
    }
}

fn auto_migrate() -> bool {
    // Always perform auto-migration in release versions.
    let version = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    if version.pre.is_empty() {
        return true;
    }

    // DO NOT USE THE FOLLOWING ENVIRONMENT VARIABLE EXCEPT FOR DEBUGGING PURPOSES.
    match std::env::var_os("MIRAKC_DEBUG_FORCE_MIGRATE") {
        Some(v) => v == "1",
        None => false,
    }
}
