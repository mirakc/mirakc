pub(in crate::web::api) mod stream;

use super::*;

use crate::recording::RecordId;
use crate::recording::RecordingStatus;

// NOTE: Record files can be directly accessible in this module, but we send messages to the
// `RecordingManager` actor in order to serialize all requests and process them one by one.
// This approach avoids race conditions regarding file operations and ensures consistency of
// records stored in the file system.  In addition, we don't need to use record files for testing
// purposes because we can use stub implementation like as others.

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
pub(in crate::web::api) async fn list<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
) -> Result<Json<Vec<WebRecord>>, Error>
where
    R: Call<recording::QueryRecords>,
{
    let records: Vec<WebRecord> = recording_manager
        .call(recording::QueryRecords)
        .await??
        .into_iter()
        .map(WebRecord::from)
        .collect();
    Ok(Json(records))
}

/// Gets metadata of a record.
#[utoipa::path(
    get,
    path = "/recording/records/{id}",
    params(
        ("id" = String, Path, description = "Record ID"),
    ),
    responses(
        (status = 200, description = "OK", body = WebRecord),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecord",
)]
pub(in crate::web::api) async fn get<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(id): Path<RecordId>,
) -> Result<Json<WebRecord>, Error>
where
    R: Call<recording::QueryRecord>,
{
    let record: WebRecord = recording_manager
        .call(recording::QueryRecord { id })
        .await??
        .into();
    Ok(Json(record))
}

/// Removes a record.
#[utoipa::path(
    delete,
    path = "/recording/records/{id}",
    params(
        ("id" = String, Path, description = "Record ID"),
        WebRecordRemovalSetting,
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "removeRecord",
)]
pub(in crate::web::api) async fn delete<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(id): Path<RecordId>,
    Qs(removal_setting): Qs<WebRecordRemovalSetting>,
) -> Result<Json<WebRecordRemovalResult>, Error>
where
    R: Call<recording::RemoveRecord>,
{
    let (record_removed, content_removed) = recording_manager
        .call(recording::RemoveRecord {
            id,
            purge: removal_setting.purge,
        })
        .await??;
    Ok(Json(WebRecordRemovalResult {
        record_removed,
        content_removed,
    }))
}
