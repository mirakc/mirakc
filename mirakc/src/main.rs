use std::path::PathBuf;

use actlet::*;
use clap::Parser;
use clap::ValueEnum;
use mirakc_core::error::Error;
use mirakc_core::tracing_ext::init_tracing;
use mirakc_core::*;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;

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
}

#[derive(Clone, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let opt = Opt::parse();

    init_tracing(match opt.log_format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    });

    let config = config::load(&opt.config);
    let string_table = string_table::load(&config.resource.strings_yaml);

    let system = System::new();

    let tuner_manager = system
        .spawn_actor(tuner::TunerManager::new(config.clone()))
        .await;

    let epg = system
        .spawn_actor(epg::Epg::new(config.clone(), tuner_manager.clone()))
        .await;

    let recording_manager = system
        .spawn_actor(recording::RecordingManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
        ))
        .await;

    let timeshift_manager = system
        .spawn_actor(timeshift::TimeshiftManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
        ))
        .await;

    let _script_runner = system
        .spawn_actor(script_runner::ScriptRunner::new(
            config.clone(),
            epg.clone(),
            recording_manager.clone(),
        ))
        .await;

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        result = web::serve(config, string_table, tuner_manager, epg, recording_manager, timeshift_manager) => result?,
        _ = sigint.recv() => {
            tracing::info!("SIGINT received");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received");
        }
    }

    tracing::info!("Stopping...");
    system.stop();

    Ok(())
}
