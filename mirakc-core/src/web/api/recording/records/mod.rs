pub(in crate::web::api) mod stream;

use super::*;

use crate::recording::Record;
use crate::recording::RecordId;
use crate::recording::RecordingStatus;

/// List records.
///
/// The following kind of records are also listed:
///
/// * Records currently recording
/// * Records failed recording but have recorded data
/// * Records whose recorded data has been deleted outside the system after recording
///
#[utoipa::path(
    get,
    path = "/recording/records",
    responses(
        (status = 200, description = "OK", body = [WebRecord]),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecords",
)]
pub(in crate::web::api) async fn list(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
) -> Result<Json<Vec<WebRecord>>, Error> {
    let records_dir = config
        .recording
        .records_dir
        .as_ref()
        .expect("config.recording.records-dir must be defined");
    let record_pattern = format!("{}/*.record.json", records_dir.display());
    let mut records: Vec<WebRecord> = vec![];
    for record_path in glob::glob(&record_pattern)? {
        let record_path = record_path?;
        if !record_path.is_file() {
            tracing::warn!(?record_path, "Should be a regular file");
            continue;
        }
        let tuple = load_record(&config, &record_path)?;
        records.push(tuple.into());
    }
    Ok(Json(records))
}

/// Gets metadata of a record.
#[utoipa::path(
    get,
    path = "/recording/records/{id}",
    params(
        ("id" = u64, Path, description = "Record ID"),
    ),
    responses(
        (status = 200, description = "OK", body = WebRecord),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecord",
)]
pub(in crate::web::api) async fn get(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
) -> Result<Json<WebRecord>, Error> {
    let record_path = make_record_path(&config, id);
    let tuple = load_record(&config, &record_path)?;
    Ok(Json(tuple.into()))
}

/// Deletes a record.
#[utoipa::path(
    delete,
    path = "/recording/records/{id}",
    params(
        ("id" = u64, Path, description = "Record ID"),
        RecordDeletionSetting,
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "deleteRecord",
)]
pub(in crate::web::api) async fn delete(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
    Qs(deletion_setting): Qs<RecordDeletionSetting>,
) -> Result<(), Error> {
    let record_path = make_record_path(&config, id);
    let (record, _) = load_record(&config, &record_path)?;
    if matches!(record.recording_status, RecordingStatus::Recording) {
        return Err(Error::NowRecording);
    }
    if deletion_setting.purge {
        let content_path = make_content_path(&config, &record.options.content_path);
        match std::fs::remove_file(&content_path) {
            Ok(_) => {
                // TODO: emit recording.data-removed
            }
            Err(err) => tracing::error!(?err, ?content_path),
        }
    }
    match std::fs::remove_file(&record_path) {
        Ok(_) => {
            // TODO: emit recording.metadata-removed
        }
        Err(err) => tracing::error!(?err, ?record_path),
    }
    Ok(())
}

// helper functions

fn make_record_path(config: &Config, id: RecordId) -> std::path::PathBuf {
    let records_dir = config
        .recording
        .records_dir
        .as_ref()
        .expect("config.recording.records-dir must be defined");
    records_dir.join(format!("{}.record.json", id.value()))
}

fn make_content_path(config: &Config, content_path: &std::path::Path) -> std::path::PathBuf {
    let basedir = config
        .recording
        .basedir
        .as_ref()
        .expect("config.recording.basedir must be defined");
    basedir.join(content_path)
}

fn load_record(
    config: &Config,
    record_path: &std::path::Path,
) -> Result<(Record, Option<u64>), Error> {
    let record_file = std::fs::File::open(record_path)?;
    let record: Record = serde_json::from_reader(record_file)?;
    let data_path = make_content_path(config, &record.options.content_path);
    let size = if data_path.is_file() {
        let size = data_path
            .metadata()
            .ok()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Some(size)
    } else {
        None
    };
    Ok((record, size))
}
