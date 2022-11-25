use axum::response::IntoResponse;
use axum::Json;

use crate::web::models::Status;

pub(super) async fn get() -> impl IntoResponse {
    Json(Status {})
}
