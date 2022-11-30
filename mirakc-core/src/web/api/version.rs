use super::*;
use crate::web::api::models::Version;

/// Gets version information.
#[utoipa::path(
    get,
    path = "/version",
    responses(
        (status = 200, description = "OK", body = Version),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "checkVersion",
)]
pub(super) async fn get() -> impl IntoResponse {
    Json(Version {
        current: env!("CARGO_PKG_VERSION"),
        latest: env!("CARGO_PKG_VERSION"), // unsupported
    })
}
