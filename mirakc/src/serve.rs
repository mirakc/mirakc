use std::sync::Arc;

use actlet::prelude::*;
use mirakc_core::*;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;

pub async fn main(config: Arc<config::Config>) {
    let string_table = string_table::load(&config.resource.strings_yaml);

    let system = System::new();

    let tuner_manager = system
        .spawn_actor(tuner::TunerManager::new(config.clone()))
        .await;

    let epg = system
        .spawn_actor(epg::Epg::new(config.clone(), tuner_manager.clone()))
        .await;

    let onair_manager = system
        .spawn_actor(onair::OnairProgramManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
        ))
        .await;

    let recording_manager = system
        .spawn_actor(recording::RecordingManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
            onair_manager.clone(),
        ))
        .await;

    let timeshift_manager = system
        .spawn_actor(timeshift::TimeshiftManager::new(
            config.clone(),
            tuner_manager.clone(),
            epg.clone(),
        ))
        .await;

    let mut sigint = signal(SignalKind::interrupt()).unwrap();
    let mut sigterm = signal(SignalKind::terminate()).unwrap();

    tokio::select! {
        result = web::serve(config, string_table, tuner_manager, epg, recording_manager, timeshift_manager, onair_manager) => {
            match result {
                Ok(_) => (),
                Err(err) => tracing::error!(%err),
            }
        }
        _ = sigint.recv() => {
            tracing::info!("SIGINT received");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received");
        }
    }

    tracing::info!("Stopping...");
    // TODO
    // ----
    // Replace `system.stop()` with `system.shutdown().await`.
    // Currently, `system.shutdown().await` blocks due to some bugs.
    system.stop();
}
