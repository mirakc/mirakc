use super::content_source::ContentSourceKind;
use super::*;
use crate::epg::stub::EpgStub;
use crate::onair::stub::OnairProgramManagerStub;
use crate::tuner::stub::TunerManagerStub;
use assert_matches::assert_matches;
use indexmap::indexmap;
use maplit::hashset;
use sha2::Digest;
use sha2::Sha256;
use tempfile::TempDir;
use test_log::test;
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

const RECORDING_DIR: &str = "recording";
const RECORDS_DIR: &str = ".records";

#[test]
fn test_record_id() {
    assert_eq!(
        RecordId::new(0, 0).value(),
        "00000000000000000000000000000000"
    );
}

#[test]
fn test_save_and_load() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config.clone());

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Tracking,
        program!((0, 1, 2), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 3), now - Duration::try_hours(1).unwrap(), "2h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("3.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let num_schedules = manager.schedules.len();

    manager.save_schedules();
    assert!(make_schedules_path(&config).unwrap().is_file());

    let mut manager = recording_manager!(config.clone());
    manager.load_schedules();
    assert_eq!(manager.schedules.len(), num_schedules);
    assert!(manager.schedules.contains_key(&(0, 1, 1).into()));
    assert!(manager.schedules.contains_key(&(0, 1, 2).into()));
    assert!(manager.schedules.contains_key(&(0, 1, 3).into()));
}

#[test]
fn test_rebuild_queue() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now + Duration::try_hours(1).unwrap()),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 2), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Tracking,
        program!((0, 1, 3), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("3.m2ts", 1)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 4), now - Duration::try_minutes(30).unwrap()),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("4.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    manager.rebuild_queue();
    assert_matches!(manager.queue.pop(), Some(item) => {
        assert_eq!(item.program_id, (0, 1, 3).into());
    });
    assert_matches!(manager.queue.pop(), Some(item) => {
        assert_eq!(item.program_id, (0, 1, 2).into());
    });
    assert_matches!(manager.queue.pop(), Some(item) => {
        assert_eq!(item.program_id, (0, 1, 1).into());
    });
    assert_matches!(manager.queue.pop(), None);
}

#[allow(clippy::get_first)]
#[test]
fn test_query_schedules() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now + Duration::try_hours(1).unwrap()),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 2), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Tracking,
        program!((0, 1, 3), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("3.m2ts", 1)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 4), now - Duration::try_minutes(30).unwrap()),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("4.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    assert_eq!(manager.schedules.len(), 4);

    let schedules = manager.query_schedules();
    assert_eq!(schedules.len(), 4);
    assert_matches!(schedules.get(0), Some(schedule) => {
        assert_eq!(schedule.program.id, (0, 1, 4).into());
    });
    assert_matches!(schedules.get(1), Some(schedule) => {
        assert_eq!(schedule.program.id, (0, 1, 3).into());
    });
    assert_matches!(schedules.get(2), Some(schedule) => {
        assert_eq!(schedule.program.id, (0, 1, 2).into());
    });
    assert_matches!(schedules.get(3), Some(schedule) => {
        assert_eq!(schedule.program.id, (0, 1, 1).into());
    });
}

#[test]
fn test_query_schedule() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    assert_matches!(manager.query_schedule((0, 1, 1).into()), Ok(schedule) => {
        assert_eq!(schedule.program.id, (0, 1, 1).into());
    });
    assert_matches!(manager.query_schedule((0, 1, 2).into()), Err(err) => {
        assert_matches!(err, Error::ScheduleNotFound);
    });
}

#[test]
fn test_add_schedule() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    assert_eq!(manager.schedules.len(), 1);

    // Schedule already exists.
    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Err(Error::AlreadyExists));
    assert_eq!(manager.schedules.len(), 1);

    // Adding a schedule for an ended program is allowed.
    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 2), now - Duration::try_hours(1).unwrap(), "3h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    assert_eq!(manager.schedules.len(), 2);

    // Adding a schedule for a program already started is allowed.
    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 3), now - Duration::try_hours(1).unwrap(), "30m"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("3.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    assert_eq!(manager.schedules.len(), 3);
}

#[test]
fn test_remove_schedules() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!(
            (0, 1, 1),
            now + Duration::try_seconds(PREP_SECS + 1).unwrap(),
            "1h"
        ),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0),
        hashset!["tag1".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!(
            (0, 1, 2),
            now + Duration::try_seconds(PREP_SECS + 1).unwrap(),
            "1h"
        ),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0),
        hashset!["tag2".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    // Schedules which will start soon are always retained.
    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!(
            (0, 1, 3),
            now + Duration::try_seconds(PREP_SECS - 1).unwrap(),
            "1h"
        ),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("3.m2ts", 0),
        hashset!["tag1".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    // Schedules in "Tracking" are always retained.
    let schedule = recording_schedule!(
        RecordingScheduleState::Tracking,
        program!((0, 1, 4), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("4.m2ts", 0),
        hashset!["tag2".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    // Schedules in "Recording" are always retained.
    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 5), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("5.m2ts", 0),
        hashset!["tag2".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    // Schedules in "Rescheduling" are always removed.
    let schedule = recording_schedule!(
        RecordingScheduleState::Rescheduling,
        program!((0, 1, 6), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("6.m2ts", 0),
        hashset!["tag2".to_string()]
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    assert_eq!(manager.schedules.len(), 6);

    manager.remove_schedules(RemovalTarget::Tag("tag2".to_string()), now);
    assert_eq!(manager.schedules.len(), 4);
    assert!(manager.schedules.contains_key(&(0, 1, 1).into()));
    assert!(manager.schedules.contains_key(&(0, 1, 3).into()));
    assert!(manager.schedules.contains_key(&(0, 1, 4).into()));
    assert!(manager.schedules.contains_key(&(0, 1, 5).into()));

    manager.remove_schedules(RemovalTarget::All, now);
    assert!(manager.schedules.is_empty());
}

#[test(tokio::test)]
async fn test_start_recording() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));
    let content_filename = "1.m2ts";
    let log_filename = "1.m2ts.log";

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut seq = mockall::Sequence::new();
    let mut record_saved = MockRecordSavedValidator::new();
    let mut content_sha256_calculated = MockContentSha256CalculatedValidator::new();

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Recording)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    content_sha256_calculated
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(move |_| notify2.notify_one())
        .once()
        .in_sequence(&mut seq);

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordSaved(Emitter::new(record_saved)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::ContentSha256Calculated(Emitter::new(
                content_sha256_calculated,
            )))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(content_filename, 0)
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let record_pattern = format!(
            "{}/{RECORDS_DIR}/*{:08X}.record.json",
            temp_dir.path().to_str().unwrap(),
            program_id.value()
        );
        assert_matches!(glob::glob(&record_pattern).unwrap().next(), Some(Ok(record_path)) => {
            assert_matches!(load_record(&config, &record_path).await, Ok((record, _)) => {
                assert_eq!(record.program.id, program_id);
                assert_matches!(record.recording_status, RecordingStatus::Recording);
                assert_eq!(record.content_path, Path::new("1.m2ts"));
                assert_eq!(record.content_type, "video/MP2T");
                assert!(record.content_sha256.is_none());
            });
        });

        // Waiting for the last RecordSaved.
        notify.notified().await;
    }
    system.shutdown().await;

    let content_path = temp_dir.path().join(RECORDING_DIR).join(content_filename);
    assert!(content_path.is_file());

    let log_path = temp_dir.path().join(RECORDING_DIR).join(log_filename);
    assert!(!log_path.exists());

    let record_pattern = format!(
        "{}/{RECORDS_DIR}/*{:08X}.record.json",
        temp_dir.path().to_str().unwrap(),
        program_id.value()
    );
    assert_matches!(glob::glob(&record_pattern).unwrap().next(), Some(Ok(record_path)) => {
        assert_matches!(load_record(&config, &record_path).await, Ok((record, _)) => {
            assert_eq!(record.program.id, program_id);
            // The recording stops when the system stops.
            assert_matches!(record.recording_status, RecordingStatus::Finished);
            assert_matches!(record.content_sha256.as_deref(), Some(sha256) => {
                let content_path = make_content_path(&config, &record).unwrap();
                let content = std::fs::read(content_path).unwrap();
                assert_eq!(sha256, sha256::format(Sha256::digest(content)));
            });
        });
    });
}

#[test(tokio::test)]
async fn test_start_recording_without_content_path() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut seq = mockall::Sequence::new();
    let mut record_saved = MockRecordSavedValidator::new();
    let mut content_sha256_calculated = MockContentSha256CalculatedValidator::new();

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Recording)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    content_sha256_calculated
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(move |_| notify2.notify_one())
        .once()
        .in_sequence(&mut seq);

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordSaved(Emitter::new(record_saved)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::ContentSha256Calculated(Emitter::new(
                content_sha256_calculated,
            )))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(0)
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let record_pattern = format!(
            "{}/{RECORDS_DIR}/*{:08X}.record.json",
            temp_dir.path().to_str().unwrap(),
            program_id.value()
        );
        assert_matches!(glob::glob(&record_pattern).unwrap().next(), Some(Ok(record_path)) => {
            assert_matches!(load_record(&config, &record_path).await, Ok((record, _)) => {
                assert_eq!(record.program.id, program_id);
                assert_matches!(record.recording_status, RecordingStatus::Recording);
                assert!(record.content_path.to_str().unwrap().ends_with(".content"));
                assert!(record.content_sha256.is_none());
            });
        });

        // Waiting for the last RecordSaved.
        notify.notified().await;
    }
    system.shutdown().await;

    let content_pattern = format!(
        "{}/{RECORDING_DIR}/*{:08X}.content",
        temp_dir.path().to_str().unwrap(),
        program_id.value()
    );
    assert_matches!(glob::glob(&content_pattern).unwrap().next(), Some(Ok(_)));

    let log_pattern = format!(
        "{}/{RECORDING_DIR}/*{:08X}.content.log",
        temp_dir.path().to_str().unwrap(),
        program_id.value()
    );
    assert_matches!(glob::glob(&log_pattern).unwrap().next(), None);

    let record_pattern = format!(
        "{}/{RECORDS_DIR}/*{:08X}.record.json",
        temp_dir.path().to_str().unwrap(),
        program_id.value()
    );
    assert_matches!(glob::glob(&record_pattern).unwrap().next(), Some(Ok(record_path)) => {
        assert_matches!(load_record(&config, &record_path).await, Ok((record, _)) => {
            assert_eq!(record.program.id, program_id);
            // The recording stops when the system stops.
            assert_matches!(record.recording_status, RecordingStatus::Finished);
            assert_matches!(record.content_sha256.as_deref(), Some(sha256) => {
                let content_path = make_content_path(&config, &record).unwrap();
                let content = std::fs::read(content_path).unwrap();
                assert_eq!(sha256, sha256::format(Sha256::digest(content)));
            });
        });
    });
}

#[test(tokio::test)]
async fn test_stop_recording() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));
    let content_filename = "1.m2ts";

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut seq = mockall::Sequence::new();
    let mut stopped = MockRecordingStoppedValidator::new();
    let mut record_saved = MockRecordSavedValidator::new();
    let mut content_sha256_calculated = MockContentSha256CalculatedValidator::new();

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Recording)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    stopped
        .expect_emit()
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    content_sha256_calculated
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_saved
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
                && matches!(msg.recording_status, RecordingStatus::Finished)
        })
        .returning(move |_| notify2.notify_one())
        .once()
        .in_sequence(&mut seq);

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordingStopped(Emitter::new(stopped)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::RecordSaved(Emitter::new(record_saved)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::ContentSha256Calculated(Emitter::new(
                content_sha256_calculated,
            )))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(content_filename, 0)
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let result = manager.call(StopRecording { program_id }).await;
        assert_matches!(result, Ok(Ok(())));

        // Waiting for the last RecordSaved.
        notify.notified().await;

        let record_pattern = format!(
            "{}/{RECORDS_DIR}/*{:08X}.record.json",
            temp_dir.path().to_str().unwrap(),
            program_id.value()
        );
        assert_matches!(glob::glob(&record_pattern).unwrap().next(), Some(Ok(record_path)) => {
            assert_matches!(load_record(&config, &record_path).await, Ok((record, _)) => {
                assert_eq!(record.program.id, program_id);
                assert_matches!(record.recording_status, RecordingStatus::Finished);
                assert_matches!(record.content_sha256.as_deref(), Some(sha256) => {
                    let content_path = make_content_path(&config, &record).unwrap();
                    let content = std::fs::read(content_path).unwrap();
                    assert_eq!(sha256, sha256::format(Sha256::digest(content)));
                });
            });
        });
    }
    system.shutdown().await;

    let content_path = temp_dir.path().join(RECORDING_DIR).join(content_filename);
    assert!(content_path.is_file());
}

#[test(tokio::test)]
async fn test_maintain_schedules() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let max_delay = Duration::try_hours(MAX_DELAY_HOURS).unwrap();

    let mut manager = recording_manager!(config.clone());
    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now + Duration::try_hours(1).unwrap(), "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    let changed = manager.maintain_schedules(now + max_delay).await;
    assert!(!changed);
    assert_eq!(manager.schedules.len(), 1);
    manager.schedules.clear();

    let mut manager = recording_manager!(config.clone());
    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 1), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));
    let changed = manager.maintain_schedules(now + max_delay).await;
    assert!(!changed);
    assert_eq!(manager.schedules.len(), 1);
    manager.schedules.clear();

    let states = [
        RecordingScheduleState::Scheduled,
        RecordingScheduleState::Tracking,
        RecordingScheduleState::Rescheduling,
    ];
    for state in states {
        let mut manager = recording_manager!(config.clone());
        let mut failed = MockRecordingFailedValidator::new();
        failed.expect_emit().times(1).returning(|msg| {
            assert_eq!(msg.program_id, (0, 1, 1).into());
        });
        manager.recording_failed.register(Emitter::new(failed));
        let schedule = recording_schedule!(
            state,
            program!((0, 1, 1), now, "1h"),
            service!((0, 1), "sv", channel_gr!("ch", "ch")),
            recording_options!("1.m2ts", 0)
        );
        let result = manager.add_schedule(schedule);
        assert_matches!(result, Ok(()));
        let changed = manager.maintain_schedules(now + max_delay).await;
        assert!(changed);
        assert!(manager.schedules.is_empty());
    }
}

#[test(tokio::test)]
async fn test_maintain_history() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let max_delay = Duration::try_hours(MAX_HISTORY_RETAIN_HOURS).unwrap();

    let mut manager = recording_manager!(config.clone());
    let schedule = recording_schedule!(
        RecordingScheduleState::Finished,
        program!((0, 1, 1), now, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    manager.history.push(schedule);
    assert!(!manager.history.is_empty());

    let changed = manager.maintain_history(now);
    assert!(!changed);
    assert!(!manager.history.is_empty());

    let changed = manager.maintain_history(now + max_delay);
    assert!(changed);
    assert!(manager.history.is_empty());
}

#[test(tokio::test)]
async fn test_remove_record() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));
    let content_filename = "1.m2ts";

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut seq = mockall::Sequence::new();
    let mut stopped = MockRecordingStoppedValidator::new();
    let mut record_removed = MockRecordRemovedValidator::new();
    let mut content_removed = MockContentRemovedValidator::new();

    stopped
        .expect_emit()
        .returning(move |_| notify2.notify_one())
        .once()
        .in_sequence(&mut seq);

    content_removed
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    record_removed
        .expect_emit()
        .withf(move |msg| {
            let program_id_part = format!("{:08X}", program_id.value());
            msg.record_id.value().ends_with(&program_id_part)
        })
        .returning(|_| ())
        .once()
        .in_sequence(&mut seq);

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordingStopped(Emitter::new(stopped)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::RecordRemoved(Emitter::new(record_removed)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(RegisterEmitter::ContentRemoved(Emitter::new(
                content_removed,
            )))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(content_filename, 0)
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let result = manager.call(StopRecording { program_id }).await;
        assert_matches!(result, Ok(Ok(())));

        notify.notified().await;

        let result = manager.call(QueryRecords).await;
        let id = assert_matches!(result, Ok(Ok(tuples)) => {
            assert_eq!(tuples.len(), 1);
            tuples[0].0.id.clone()
        });

        let record_path = temp_dir
            .path()
            .join(RECORDS_DIR)
            .join(format!("{}.record.json", id.value()));
        assert!(record_path.exists());

        let content_path = temp_dir.path().join(RECORDING_DIR).join(content_filename);
        assert!(content_path.exists());

        let result = manager.call(RemoveRecord { id, purge: true }).await;
        assert_matches!(result, Ok(Ok((true, true))));

        assert!(!record_path.exists());
        assert!(!content_path.exists());
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_open_content() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record = record!(finished: id.value());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let content_path = make_content_path(&config, &record).unwrap();
    assert!(file_util::save_data(b"0123456789", &content_path));

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager.call(OpenContent::new(id.clone(), None)).await;
        let (stream, stop_trigger) = match result {
            Ok(Ok(tuple)) => tuple,
            _ => panic!(),
        };

        let mut reader = tokio_util::io::StreamReader::new(stream);

        let mut buf = [0; 10];
        reader.read_exact(&mut buf).await.unwrap(); // EOF reaches.
        assert_eq!(&buf, b"0123456789");

        append(&content_path, b"abc").await;

        assert_matches!(reader.read_exact(&mut buf).await, Err(err) => {
            assert_matches!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        });

        drop(stop_trigger);
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_open_content_during_recording() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record = record!(recording: id.clone());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let content_path = make_content_path(&config, &record).unwrap();
    tokio::fs::write(&content_path, b"0123456789")
        .await
        .unwrap();

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager.call(OpenContent::new(id.clone(), None)).await;
        let (stream, stop_trigger) = match result {
            Ok(Ok(tuple)) => tuple,
            _ => panic!(),
        };

        let mut reader = tokio_util::io::StreamReader::new(stream);

        let mut buf = [0; 10];
        reader.read_exact(&mut buf).await.unwrap(); // EOF reaches.
        assert_eq!(&buf, b"0123456789");

        append(&content_path, b"abc").await;

        let mut buf = [0; 3];
        reader.read_exact(&mut buf).await.unwrap(); // EOF reaches again.
        assert_eq!(&buf, b"abc");

        // The streaming will stop within 100ms without explicit `drop(stop_trigger)`.
        assert_matches!(reader.read_exact(&mut buf).await, Err(err) => {
            assert_matches!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        });

        drop(stop_trigger);
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_open_content_stop_trigger() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record = record!(recording: id.clone());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let content_path = make_content_path(&config, &record).unwrap();
    tokio::fs::write(&content_path, b"0123456789")
        .await
        .unwrap();

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager.call(OpenContent::new(id.clone(), None)).await;
        let (stream, stop_trigger) = match result {
            Ok(Ok(tuple)) => tuple,
            _ => panic!(),
        };

        let mut reader = tokio_util::io::StreamReader::new(stream);

        let mut buf = [0; 10];
        reader.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"0123456789");

        drop(stop_trigger);

        // The streaming will stop soon before the timeout.
        assert_matches!(reader.read_exact(&mut buf).await, Err(err) => {
            assert_matches!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        });
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_recording_with_log_filter_info() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));
    let content_filename = "1.m2ts";
    let log_filename = "1.m2ts.log";

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut stopped = MockRecordingStoppedValidator::new();

    stopped
        .expect_emit()
        .returning(move |_| notify2.notify_one())
        .once();

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordingStopped(Emitter::new(stopped)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(content_filename, 0, "info")
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let result = manager.call(StopRecording { program_id }).await;
        assert_matches!(result, Ok(Ok(())));

        notify.notified().await;

        let log_path = temp_dir.path().join(RECORDING_DIR).join(log_filename);
        assert!(log_path.is_file());

        let result = manager.call(QueryRecords).await;
        let id = assert_matches!(result, Ok(Ok(tuples)) => {
            assert_eq!(tuples.len(), 1);
            tuples[0].0.id.clone()
        });

        let result = manager.call(RemoveRecord { id, purge: true }).await;
        assert_matches!(result, Ok(Ok((true, true))));

        assert!(!log_path.exists());
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_recording_with_log_filter_off() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let program_id = ProgramId::from((0, 1, 1));
    let content_filename = "1.m2ts";
    let log_filename = "1.m2ts.log";

    let notify = Arc::new(Notify::new());
    let notify2 = notify.clone();

    let mut stopped = MockRecordingStoppedValidator::new();

    stopped
        .expect_emit()
        .returning(move |_| notify2.notify_one())
        .once();

    let system = System::new();
    {
        let manager = system.spawn_actor(recording_manager!(config.clone())).await;

        let result = manager
            .call(RegisterEmitter::RecordingStopped(Emitter::new(stopped)))
            .await;
        assert_matches!(result, Ok(_));

        let result = manager
            .call(StartRecording {
                schedule: recording_schedule!(
                    RecordingScheduleState::Scheduled,
                    program!(program_id, now, "1h"),
                    service!((0, 1), "sv", channel_gr!("ch", "ch")),
                    recording_options!(content_filename, 0, "off")
                ),
            })
            .await;
        assert_matches!(result, Ok(Ok(())));

        let result = manager.call(StopRecording { program_id }).await;
        assert_matches!(result, Ok(Ok(())));

        notify.notified().await;

        let log_path = temp_dir.path().join(RECORDING_DIR).join(log_filename);
        assert!(!log_path.exists());

        let result = manager.call(QueryRecords).await;
        let id = assert_matches!(result, Ok(Ok(tuples)) => {
            assert_eq!(tuples.len(), 1);
            tuples[0].0.id.clone()
        });

        let result = manager.call(RemoveRecord { id, purge: true }).await;
        assert_matches!(result, Ok(Ok((true, true))));
    }
    system.shutdown().await;
}

#[test(tokio::test)]
async fn test_handle_recording_stopped() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let mut stopped = MockRecordingStoppedValidator::new();
    stopped.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
    });
    manager.recording_stopped.register(Emitter::new(stopped));

    let mut failed = MockRecordingFailedValidator::new();
    failed.expect_emit().never();
    manager.recording_failed.register(Emitter::new(failed));

    let start_time = now - Duration::try_minutes(30).unwrap();

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 1), start_time, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0),
        hashset!["tag1".to_string()]
    );
    manager.schedules.insert((0, 1, 1).into(), schedule);

    let recorder = recorder!(start_time, pipeline!["true"]);
    manager.recorders.insert((0, 1, 1).into(), recorder);

    let changed = manager.handle_recording_stopped((0, 1, 1).into()).await;
    assert!(changed);
    assert!(!manager.recorders.contains_key(&(0, 1, 1).into()));
    // The schedule has been moved from `schedules` to `history`.
    assert_matches!(manager.schedules.get(&(0, 1, 1).into()), None);
    assert_matches!(manager.history.first(), Some(schedule) => {
        assert_matches!(schedule.state, RecordingScheduleState::Finished);
    });
}

#[test(tokio::test)]
async fn test_handle_recording_stopped_retry() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let mut stopped = MockRecordingStoppedValidator::new();
    stopped.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
    });
    manager.recording_stopped.register(Emitter::new(stopped));

    let mut failed = MockRecordingFailedValidator::new();
    failed.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
        assert_matches!(msg.reason, RecordingFailedReason::NeedRescheduling);
    });
    manager.recording_failed.register(Emitter::new(failed));

    let start_time = now - Duration::try_minutes(30).unwrap();

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 1), start_time, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0),
        hashset!["tag1".to_string()]
    );
    manager.schedules.insert((0, 1, 1).into(), schedule);

    let recorder = recorder!(start_time, pipeline![format!("sh -c 'exit {EXIT_RETRY}'")]);
    manager.recorders.insert((0, 1, 1).into(), recorder);

    let changed = manager.handle_recording_stopped((0, 1, 1).into()).await;
    assert!(changed);
    assert!(!manager.recorders.contains_key(&(0, 1, 1).into()));
    assert_matches!(manager.schedules.get(&(0, 1, 1).into()), Some(schedule) => {
        assert_matches!(schedule.state, RecordingScheduleState::Rescheduling);
    });
}

#[test(tokio::test)]
async fn test_handle_recording_stopped_pipeline_error() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);

    let mut stopped = MockRecordingStoppedValidator::new();
    stopped.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
    });
    manager.recording_stopped.register(Emitter::new(stopped));

    let mut failed = MockRecordingFailedValidator::new();
    failed.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
        assert_matches!(
            msg.reason,
            RecordingFailedReason::PipelineError { exit_code: 1 }
        );
    });
    manager.recording_failed.register(Emitter::new(failed));

    let start_time = now - Duration::try_minutes(30).unwrap();

    let schedule = recording_schedule!(
        RecordingScheduleState::Recording,
        program!((0, 1, 1), start_time, "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0),
        hashset!["tag1".to_string()]
    );
    manager.schedules.insert((0, 1, 1).into(), schedule);

    let recorder = recorder!(start_time, pipeline!["false"]);
    manager.recorders.insert((0, 1, 1).into(), recorder);

    let changed = manager.handle_recording_stopped((0, 1, 1).into()).await;
    assert!(changed);
    assert!(!manager.recorders.contains_key(&(0, 1, 1).into()));
    // The schedule has been moved from `schedules` to `history`.
    assert_matches!(manager.schedules.get(&(0, 1, 1).into()), None);
    assert_matches!(manager.history.first(), Some(schedule) => {
        assert_matches!(schedule.state, RecordingScheduleState::Failed);
    });
}

#[test(tokio::test)]
async fn test_update_schedules_by_epg_services() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut manager = recording_manager!(config);
    let mut mock = MockRecordingFailedValidator::new();
    mock.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 2, 1).into());
        assert_matches!(msg.reason, RecordingFailedReason::RemovedFromEpg);
    });
    manager.recording_failed.register(Emitter::new(mock));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 1), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 2, 1), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let services = indexmap! {
        (0, 1).into() => service!((0, 1), "test", channel_gr!("gr", "1")),
    };
    let changed = manager.update_schedules_by_epg_services(&services).await;
    assert!(changed);
    assert_eq!(manager.schedules.len(), 1);
    assert!(manager.schedules.contains_key(&(0, 1, 1).into()));
    assert!(!manager.schedules.contains_key(&(0, 2, 1).into()));

    let changed = manager.update_schedules_by_epg_services(&services).await;
    assert!(!changed);
}

#[test(tokio::test)]
async fn test_update_schedules_by_epg_programs() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut epg = MockEpg::new();
    epg.expect_call().returning(move |_| {
        Ok(Arc::new(indexmap! {
            1.into() => program!((0, 1, 1), now, "1h"),
        }))
    });

    let mut manager = recording_manager!(
        config,
        TunerManagerStub::default(),
        epg,
        OnairProgramManagerStub
    );
    let mut failed_mock = MockRecordingFailedValidator::new();
    failed_mock.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 2).into());
        assert_matches!(msg.reason, RecordingFailedReason::RemovedFromEpg);
    });
    manager.recording_failed.register(Emitter::new(failed_mock));

    let mut rescheduled_mock = MockRecordingRescheduledValidator::new();
    rescheduled_mock.expect_emit().never();
    manager
        .recording_rescheduled
        .register(Emitter::new(rescheduled_mock));

    let schedule = recording_schedule!(
        RecordingScheduleState::Rescheduling,
        program!((0, 1, 1), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let schedule = recording_schedule!(
        RecordingScheduleState::Scheduled,
        program!((0, 1, 2), now),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("2.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let changed = manager
        .update_schedules_by_epg_programs(now, (0, 1).into())
        .await;
    assert!(changed);
    assert_eq!(manager.schedules.len(), 1);
    assert!(manager.schedules.contains_key(&(0, 1, 1).into()));
    assert!(!manager.schedules.contains_key(&(0, 1, 2).into()));

    let changed = manager
        .update_schedules_by_epg_programs(now, (0, 0).into())
        .await;
    assert!(!changed);
}

#[test(tokio::test)]
async fn test_update_schedules_by_epg_programs_rescheduled() {
    let now = Jst::now();

    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let mut epg = MockEpg::new();
    epg.expect_call().returning(move |_| {
        Ok(Arc::new(indexmap! {
            1.into() => program!((0, 1, 1), now, "1h"),
        }))
    });

    let mut manager = recording_manager!(
        config,
        TunerManagerStub::default(),
        epg,
        OnairProgramManagerStub
    );

    let mut mock = MockRecordingRescheduledValidator::new();
    mock.expect_emit().times(1).returning(|msg| {
        assert_eq!(msg.program_id, (0, 1, 1).into());
    });
    manager.recording_rescheduled.register(Emitter::new(mock));

    let schedule = recording_schedule!(
        RecordingScheduleState::Rescheduling,
        program!((0, 1, 1), now - Duration::try_minutes(30).unwrap(), "1h"),
        service!((0, 1), "sv", channel_gr!("ch", "ch")),
        recording_options!("1.m2ts", 0)
    );
    let result = manager.add_schedule(schedule);
    assert_matches!(result, Ok(()));

    let changed = manager
        .update_schedules_by_epg_programs(now, (0, 1).into())
        .await;
    assert!(changed);
    assert_eq!(manager.schedules.len(), 1);
    assert_matches!(manager.schedules.get(&(0, 1, 1).into()), Some(schedule) => {
        assert_matches!(schedule.state, RecordingScheduleState::Scheduled);
    });
}

#[test(tokio::test)]
async fn test_content_source_create_stream() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let ctx = actlet::stubs::Context::default();

    let id = RecordId("1".to_string());
    let record = record!(recording: id.value());

    let content_path = make_content_path(&config, &record).unwrap();
    let content_path_str = content_path.to_str().unwrap();
    tokio::fs::write(&content_path, b"0123456789")
        .await
        .unwrap();

    // recording, w/o range
    let mut source = ContentSource::new(&config, &record, None, &ctx)
        .await
        .unwrap();
    assert_matches!(source.kind(), ContentSourceKind::Pipeline(pipeline) => {
        let models = pipeline.get_model();
        assert_eq!(models.len(), 1);
        assert_matches!(models[0], CommandPipelineProcessModel { ref command, pid } => {
            assert_eq!(*command, format!("tail -f -c +0 '{content_path_str}'"));
            assert!(pid.is_some());
        });
    });
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut content = String::new();
    assert_matches!(reader.read_to_string(&mut content).await, Ok(size) => {
        assert_eq!(size, 10);
        assert_eq!(content, "0123456789");
    });

    // recording, w/ range: uses seek + take instead of `dd`
    let range = Some(ContentRange::without_size(1, 3).unwrap());
    let mut source = ContentSource::new(&config, &record, range.as_ref(), &ctx)
        .await
        .unwrap();
    assert_matches!(source.kind(), ContentSourceKind::File(Some(_)));
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut content = String::new();
    assert_matches!(reader.read_to_string(&mut content).await, Ok(size) => {
        assert_eq!(size, 3);
        assert_eq!(content, "123");
    });

    let record = record!(finished: id.value());

    // finished, w/o range
    let mut source = ContentSource::new(&config, &record, None, &ctx)
        .await
        .unwrap();
    assert_matches!(source.kind(), ContentSourceKind::File(Some(_)));
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut content = String::new();
    assert_matches!(reader.read_to_string(&mut content).await, Ok(size) => {
        assert_eq!(size, 10);
        assert_eq!(content, "0123456789");
    });

    // finished, w/ range: uses seek + take instead of `dd`
    let range = Some(ContentRange::with_size(1, 3, 10).unwrap());
    let mut source = ContentSource::new(&config, &record, range.as_ref(), &ctx)
        .await
        .unwrap();
    assert_matches!(source.kind(), ContentSourceKind::File(Some(_)));
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut content = String::new();
    assert_matches!(reader.read_to_string(&mut content).await, Ok(size) => {
        assert_eq!(size, 3);
        assert_eq!(content, "123");
    });
}

// The content used in `test_content_source_create_stream` is 10 bytes, far smaller than
// CHUNK_SIZE, so it never exercises a stream that spans multiple chunks.
#[test(tokio::test)]
async fn test_content_source_create_stream_spanning_chunks() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let ctx = actlet::stubs::Context::default();

    let id = RecordId("1".to_string());
    let record = record!(finished: id.value());

    // Deliberately not a multiple of CHUNK_SIZE so that the last chunk is partial.
    const LEN: usize = 4096 * 8 * 2 + 1234;
    let content: Vec<u8> = (0..LEN).map(|i| (i % 251) as u8).collect();

    let content_path = make_content_path(&config, &record).unwrap();
    tokio::fs::write(&content_path, &content).await.unwrap();

    // The whole content, w/o range.
    let mut source = ContentSource::new(&config, &record, None, &ctx)
        .await
        .unwrap();
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut got = Vec::new();
    assert_matches!(reader.read_to_end(&mut got).await, Ok(size) => {
        assert_eq!(size, LEN);
    });
    assert_eq!(got, content);

    // A range spanning several chunks, starting at an offset that is not chunk-aligned.
    let first = 4096 * 8 + 777;
    let last = LEN - 999;
    let range = Some(ContentRange::with_size(first as u64, last as u64, LEN as u64).unwrap());
    let mut source = ContentSource::new(&config, &record, range.as_ref(), &ctx)
        .await
        .unwrap();
    let stream = source.create_stream(1000);
    let mut reader = tokio_util::io::StreamReader::new(stream);
    let mut got = Vec::new();
    assert_matches!(reader.read_to_end(&mut got).await, Ok(size) => {
        assert_eq!(size, last - first + 1);
    });
    assert_eq!(got, content[first..=last]);
}

// HTTP ranges are normalized against the known content length before reaching
// `ContentSource`.  This covers ranges built directly with `ContentRange::without_size`, which
// checks `first <= last` but has no content length to check against, and the case where a file
// shrinks after its length is read.  Both have to keep the old `dd` behavior: return whatever
// is available and stop, rather than fail.
#[test(tokio::test)]
async fn test_content_source_create_stream_range_past_eof() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let ctx = actlet::stubs::Context::default();

    let id = RecordId("1".to_string());
    let record = record!(recording: id.value());

    let content_path = make_content_path(&config, &record).unwrap();
    tokio::fs::write(&content_path, b"0123456789")
        .await
        .unwrap();

    // (first, last, expected content)
    let cases: &[(u64, u64, &str)] = &[
        // Ends exactly at the last byte.
        (5, 9, "56789"),
        // Straddles the end of the file; only the available bytes come back.
        (5, 1000, "56789"),
        // Starts exactly at the end of the file.
        (10, 1000, ""),
        // Starts past the end of the file.
        (1000, 2000, ""),
    ];

    for &(first, last, expected) in cases {
        let range = Some(ContentRange::without_size(first, last).unwrap());
        let mut source = ContentSource::new(&config, &record, range.as_ref(), &ctx)
            .await
            .unwrap();
        let stream = source.create_stream(1000);
        let mut reader = tokio_util::io::StreamReader::new(stream);
        let mut content = String::new();
        assert_matches!(reader.read_to_string(&mut content).await, Ok(size) => {
            assert_eq!(size, expected.len(), "bytes=({first}, {last})");
        });
        assert_eq!(content, expected, "bytes=({first}, {last})");
    }
}

#[test(tokio::test)]
async fn test_check_retry() {
    // exit(0)
    let mut pipeline: CommandPipeline<u8> = pipeline!["true"];
    let results = pipeline.wait().await;
    assert!(!check_retry(&results));

    // exit(1)
    let mut pipeline: CommandPipeline<u8> = pipeline!["false"];
    let results = pipeline.wait().await;
    assert!(!check_retry(&results));

    // no such command
    let mut pipeline: CommandPipeline<u8> = pipeline!["sh -c 'command_not_fond'"];
    let results = pipeline.wait().await;
    assert!(!check_retry(&results));

    // retry
    let mut pipeline: CommandPipeline<u8> = pipeline![format!("sh -c 'exit {EXIT_RETRY}'")];
    let results = pipeline.wait().await;
    assert!(check_retry(&results));
}

#[test(tokio::test)]
async fn test_recorder_get_first_error() {
    // exit(0)
    let mut pipeline: CommandPipeline<u8> = pipeline!["true"];
    let results = pipeline.wait().await;
    assert_matches!(get_first_error(&results), None);

    // exit(1)
    let mut pipeline: CommandPipeline<u8> = pipeline!["false"];
    let results = pipeline.wait().await;
    assert_matches!(get_first_error(&results), Some(1));
}

#[test(tokio::test)]
async fn test_check_records_has_content_sha256() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let mut record = record!(recording: id.value());
    record.content_sha256 = Some("dummy".to_string());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let content_path = make_content_path(&config, &record).unwrap();
    assert!(file_util::save_data(b"0123456789", &content_path));

    let mut emitter = MockCalculateSha256Validator::new();
    emitter.expect_emit().never();

    let mut broken = MockRecordBrokenValidator::new();
    broken.expect_emit().never();

    let mut manager = recording_manager!(config);
    manager.record_broken.register(Emitter::new(broken));
    manager.check_records(Emitter::new(emitter)).await;
}

#[test(tokio::test)]
async fn test_check_records_not_has_content_sha256() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record = record!(recording: id.value());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let content_path = make_content_path(&config, &record).unwrap();
    assert!(file_util::save_data(b"0123456789", &content_path));

    let mut emitter = MockCalculateSha256Validator::new();
    emitter.expect_emit().returning(|_| ()).once();

    let mut broken = MockRecordBrokenValidator::new();
    broken.expect_emit().never();

    let mut manager = recording_manager!(config);
    manager.record_broken.register(Emitter::new(broken));
    manager.check_records(Emitter::new(emitter)).await;
}

#[test(tokio::test)]
async fn test_check_records_no_content_file() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record = record!(recording: id.value());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_json(&record, &record_path));

    let mut emitter = MockCalculateSha256Validator::new();
    emitter.expect_emit().never();

    let mut broken = MockRecordBrokenValidator::new();
    broken.expect_emit().never();

    let mut manager = recording_manager!(config);
    manager.record_broken.register(Emitter::new(broken));
    manager.check_records(Emitter::new(emitter)).await;
}

#[test(tokio::test)]
async fn test_check_records_broken() {
    let temp_dir = TempDir::new().unwrap();
    let config = config_for_test(temp_dir.path());

    let id = RecordId("1".to_string());
    let record_path = make_record_path(&config, &id).unwrap();
    assert!(file_util::save_data(b"0123456789", &record_path));

    let mut emitter = MockCalculateSha256Validator::new();
    emitter.expect_emit().never();

    let mut broken = MockRecordBrokenValidator::new();
    broken.expect_emit().returning(|_| ()).once();

    let mut manager = recording_manager!(config);
    manager.record_broken.register(Emitter::new(broken));
    manager.check_records(Emitter::new(emitter)).await;
}

fn config_for_test<P: AsRef<Path>>(dir: P) -> Arc<Config> {
    let mut config = Config::default();

    let recording_dir = dir.as_ref().join(RECORDING_DIR);
    std::fs::create_dir(&recording_dir).unwrap();
    config.recording.basedir = Some(recording_dir);

    let records_dir = dir.as_ref().join(RECORDS_DIR);
    std::fs::create_dir(&records_dir).unwrap();
    config.recording.records_dir = Some(records_dir);

    config.filters.program_filter.command = "cat".to_string();

    Arc::new(config)
}

async fn append(path: &Path, data: &[u8]) {
    use tokio::io::AsyncWriteExt;
    tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .unwrap()
        .write_all(data)
        .await
        .unwrap();
}

mockall::mock! {
    Epg {}

    #[async_trait]
    impl Call<QueryPrograms> for Epg {
        async fn call(&self, msg: QueryPrograms) -> actlet::Result<<QueryPrograms as Message>::Reply>;
    }
}

mockall::mock! {
    RecordingStoppedValidator {}

    #[async_trait]
    impl Emit<RecordingStopped> for RecordingStoppedValidator {
        async fn emit(&self, msg: RecordingStopped);
    }
}

mockall::mock! {
    RecordingFailedValidator {}

    #[async_trait]
    impl Emit<RecordingFailed> for RecordingFailedValidator {
        async fn emit(&self, msg: RecordingFailed);
    }
}

mockall::mock! {
    RecordingRescheduledValidator {}

    #[async_trait]
    impl Emit<RecordingRescheduled> for RecordingRescheduledValidator {
        async fn emit(&self, msg: RecordingRescheduled);
    }
}

mockall::mock! {
    RecordSavedValidator {}

    #[async_trait]
    impl Emit<RecordSaved> for RecordSavedValidator {
        async fn emit(&self, msg: RecordSaved);
    }
}

mockall::mock! {
    RecordRemovedValidator {}

    #[async_trait]
    impl Emit<RecordRemoved> for RecordRemovedValidator {
        async fn emit(&self, msg: RecordRemoved);
    }
}

mockall::mock! {
    ContentRemovedValidator {}

    #[async_trait]
    impl Emit<ContentRemoved> for ContentRemovedValidator {
        async fn emit(&self, msg: ContentRemoved);
    }
}

mockall::mock! {
    RecordBrokenValidator {}

    #[async_trait]
    impl Emit<RecordBroken> for RecordBrokenValidator {
        async fn emit(&self, msg: RecordBroken);
    }
}

mockall::mock! {
    ContentSha256CalculatedValidator {}

    #[async_trait]
    impl Emit<ContentSha256Calculated> for ContentSha256CalculatedValidator {
        async fn emit(&self, msg: ContentSha256Calculated);
    }
}

mockall::mock! {
    CalculateSha256Validator {}

    #[async_trait]
    impl Emit<CalculateSha256> for CalculateSha256Validator {
        async fn emit(&self, msg: CalculateSha256);
    }
}
