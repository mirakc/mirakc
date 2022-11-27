use super::*;
use crate::web::api::models::Version;

pub(super) async fn get() -> impl IntoResponse {
    Json(Version {
        current: env!("CARGO_PKG_VERSION"),
        latest: env!("CARGO_PKG_VERSION"), // unsupported
    })
}
