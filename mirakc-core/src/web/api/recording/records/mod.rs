pub(in crate::web::api) mod stream;

use super::*;

/// List records.
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
    State(ConfigExtractor(_config)): State<ConfigExtractor>,
) -> Result<Json<Vec<WebRecord>>, Error> {
    Ok(Json(vec![]))
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
    State(ConfigExtractor(_config)): State<ConfigExtractor>,
    Path(_id): Path<RecordId>,
) -> Result<Json<WebRecord>, Error> {
    Err(Error::RecordNotFound)
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
    State(ConfigExtractor(_config)): State<ConfigExtractor>,
    Path(_id): Path<RecordId>,
) -> Result<(), Error> {
    Ok(())
}
