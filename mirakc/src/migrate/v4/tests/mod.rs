use super::*;
use tempfile::TempDir;
use test_log::test;

#[test]
fn test_migrate_recording_schedules_no_recording_dir() {
    let config = config::Config::default();
    assert!(config.recording.basedir.is_none());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_no_schedules_json() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    assert!(!old_path.exists());

    let mut config = config::Config::default();
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_already_migrated() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    std::fs::write(&old_path, b"").unwrap();
    assert!(old_path.is_file());

    let new_path = recording_dir.join("schedules.v1.json");
    std::fs::write(&new_path, b"").unwrap();
    assert!(new_path.is_file());

    let mut config = config::Config::default();
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_no_epg_data() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    std::fs::write(&old_path, b"").unwrap();
    assert!(old_path.is_file());

    let new_path = recording_dir.join("schedules.v1.json");
    assert!(!new_path.exists());

    let epg_dir = temp_dir.as_ref().join("epg");
    assert!(!epg_dir.exists());

    let mut config = config::Config::default();
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_no_services_json() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    std::fs::write(&old_path, b"").unwrap();
    assert!(old_path.is_file());

    let new_path = recording_dir.join("schedules.v1.json");
    assert!(!new_path.exists());

    let epg_dir = temp_dir.as_ref().join("epg");
    std::fs::create_dir(&epg_dir).unwrap();
    assert!(epg_dir.is_dir());

    let services_json = epg_dir.join("services.json");
    assert!(!services_json.exists());

    let mut config = config::Config::default();
    config.epg.cache_dir = Some(epg_dir.clone());
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_no_such_service() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    // TODO: write test data
    std::fs::write(&old_path, include_bytes!("schedules.json")).unwrap();
    assert!(old_path.is_file());

    let new_path = recording_dir.join("schedules.v1.json");
    assert!(!new_path.exists());

    let epg_dir = temp_dir.as_ref().join("epg");
    std::fs::create_dir(&epg_dir).unwrap();
    assert!(epg_dir.is_dir());

    let services_json = epg_dir.join("services.json");
    std::fs::write(&services_json, b"[]").unwrap();
    assert!(services_json.is_file());

    let mut config = config::Config::default();
    config.epg.cache_dir = Some(epg_dir.clone());
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(!migrated);
}

#[test]
fn test_migrate_recording_schedules_migrated() {
    let temp_dir = TempDir::new().unwrap();

    let recording_dir = temp_dir.as_ref().join("recording");
    std::fs::create_dir(&recording_dir).unwrap();
    assert!(recording_dir.is_dir());

    let old_path = recording_dir.join("schedules.json");
    // TODO: write test data
    std::fs::write(&old_path, include_bytes!("schedules.json")).unwrap();
    assert!(old_path.is_file());

    let new_path = recording_dir.join("schedules.v1.json");
    assert!(!new_path.exists());

    let epg_dir = temp_dir.as_ref().join("epg");
    std::fs::create_dir(&epg_dir).unwrap();
    assert!(epg_dir.is_dir());

    let services_json = epg_dir.join("services.json");
    std::fs::write(&services_json, include_bytes!("services.json")).unwrap();
    assert!(services_json.is_file());

    let mut config = config::Config::default();
    config.epg.cache_dir = Some(epg_dir.clone());
    config.recording.basedir = Some(recording_dir.clone());

    let migrated = migrate_recording_schedules(&config, &Default::default());
    assert!(migrated);
    assert!(old_path.is_file());
    assert!(new_path.is_file());

    let actual: serde_json::Value = {
        let file = std::fs::File::open(&new_path).unwrap();
        serde_json::from_reader(file).unwrap()
    };
    let expected: serde_json::Value =
        serde_json::from_slice(include_bytes!("schedules.v1.json")).unwrap();
    assert_eq!(actual, expected);
}
