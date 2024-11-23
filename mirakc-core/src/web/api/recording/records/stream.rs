use super::*;

/// Gets a media stream of a record.
#[utoipa::path(
    get,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = u64, Path, description = "Record ID"),
        FilterSetting,
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordStream",
)]
pub(in crate::web::api) async fn get(
    State(ConfigExtractor(_config)): State<ConfigExtractor>,
    Path(_id): Path<RecordId>,
    Qs(_filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error> {
    Err(Error::RecordNotFound)
}

#[utoipa::path(
    head,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = u64, Path, description = "Record ID"),
        FilterSetting,
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "checkRecordStream",
)]
pub(in crate::web::api) async fn head(
    State(ConfigExtractor(_config)): State<ConfigExtractor>,
    Path(_id): Path<RecordId>,
    Qs(_filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error> {
    Err(Error::RecordNotFound)
}
