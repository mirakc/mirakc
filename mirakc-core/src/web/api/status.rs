use super::*;
use crate::web::api::models::Status;

/// Gets current status information.
///
/// mirakc doesn't implement this endpoint and always returns an empty object.
#[utoipa::path(
    get,
    path = "/status",
    responses(
        (status = 200, description = "OK", body = Status),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getStatus",
)]
pub(super) async fn get() -> impl IntoResponse {
    Json(Status {})
}
