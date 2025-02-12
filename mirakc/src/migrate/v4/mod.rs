#[cfg(test)]
mod tests;

use super::*;

use std::collections::HashMap;

pub fn migrate(config: &config::Config, cl: &CommandLine) {
    migrate_recording_schedules(config, cl);
}

pub fn migrate_recording_schedules(config: &config::Config, cl: &CommandLine) -> bool {
    let basedir = match config.recording.basedir.as_ref() {
        Some(basedir) => basedir,
        None => {
            tracing::info!("The recording feature is disabled");
            return false;
        }
    };

    let old_path = basedir.join("schedules.json");
    if !old_path.is_file() {
        tracing::info!(file = %old_path.display(), "File not found");
        return false;
    }

    let new_path = basedir.join("schedules.v1.json");
    if !cl.force && new_path.is_file() {
        tracing::info!(file = %new_path.display(), "Already migrated");
        return false;
    }

    let epg_cache_dir = match config.epg.cache_dir.as_ref() {
        Some(epg_cache_dir) => epg_cache_dir,
        None => {
            tracing::error!("No EGP data");
            return false;
        }
    };

    let services: Vec<(models::ServiceId, epg::EpgService)> = {
        let path = epg_cache_dir.join("services.json");
        if !path.is_file() {
            tracing::error!(file = %path.display(), "No EPG data");
            return false;
        }
        let file = std::fs::File::open(path).unwrap();
        serde_json::from_reader(file).unwrap()
    };

    let service_map: HashMap<models::ServiceId, epg::EpgService> = HashMap::from_iter(services);

    let mut schedules: serde_json::Value = {
        let file = std::fs::File::open(&old_path).unwrap();
        serde_json::from_reader(file).unwrap()
    };

    tracing::info!(
        file = %old_path.display(),
        reason = "feat(recording)!: add `RecordingSchedule.service`",
        commit = "b4e324db670947dfdd056402faaa7bcedcc2e4dc",
        "Migrating...",
    );
    for sched in schedules.as_array_mut().unwrap().iter_mut() {
        let program_id = sched["program"]["id"].clone();
        let program_id: models::ProgramId = serde_json::from_value(program_id).unwrap();
        let service_id = program_id.into();
        let service = match service_map.get(&service_id) {
            Some(service) => service,
            None => {
                tracing::error!(%service_id, "No such service");
                return false;
            }
        };
        sched["service"] = serde_json::to_value(service).unwrap();
        tracing::debug!(?program_id, "Updated the recording schedule");
    }

    if file_util::save_json(&schedules, &new_path) {
        tracing::info!("Migrated successfully");
        true
    } else {
        tracing::error!(?new_path, "Failed to save");
        false
    }
}
