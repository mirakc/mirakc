use std::sync::Arc;

use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;

use actlet::prelude::*;
use mirakc_core::*;

pub async fn main(config: Arc<config::Config>) {
    let system = System::new();

    let mut sigint = signal(SignalKind::interrupt()).unwrap();
    let mut sigterm = signal(SignalKind::terminate()).unwrap();

    tokio::select! {
        result = serve(config, &system) => {
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
    system.shutdown().await;
}

async fn serve(config: Arc<config::Config>, system: &System) -> Result<(), error::Error> {
    let string_table = string_table::load(&config.resource.strings_yaml);

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

    web::serve(
        config,
        string_table,
        tuner_manager,
        epg,
        recording_manager,
        timeshift_manager,
        onair_manager,
        system.spawner(),
    )
    .await
}
