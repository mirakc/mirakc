use super::*;
use crate::web::api::models::Status;

pub(super) async fn get() -> impl IntoResponse {
    Json(Status {})
}
