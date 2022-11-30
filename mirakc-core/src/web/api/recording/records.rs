use super::*;

use std::collections::HashMap;
use std::io::SeekFrom;
use std::ops::Bound;

use axum::body::StreamBody;
use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CONTENT_RANGE;
use axum::http::header::CONTENT_TYPE;
use tokio::io::AsyncSeekExt;
use tokio_util::io::ReaderStream;

use crate::web::body::SeekableStreamBody;

/// Lists records.
#[utoipa::path(
    get,
    path = "/recording/records",
    responses(
        (status = 200, description = "OK", body = [WebRecordingRecord]),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingRecord>>, Error>
where
    R: Call<recording::QueryRecordingRecords>,
{
    let records: Vec<WebRecordingRecord> = state
        .recording_manager
        .call(recording::QueryRecordingRecords)
        .await??
        .into_iter()
        .map(WebRecordingRecord::from)
        .collect();
    Ok(Json(records))
}

/// Gets a record.
#[utoipa::path(
    get,
    path = "/recording/records/{id}",
    params(
        ("id" = String, Path, description = "Record ID"),
    ),
    responses(
        (status = 200, description = "OK", body = WebRecordingRecord),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
) -> Result<Json<WebRecordingRecord>, Error>
where
    R: Call<recording::QueryRecordingRecord>,
{
    state
        .recording_manager
        .call(recording::QueryRecordingRecord { id })
        .await?
        .map(WebRecordingRecord::from)
        .map(Json::from)
}

/// Deletes a record.
#[utoipa::path(
    delete,
    path = "/recording/records/{id}",
    params(
        ("id" = String, Path, description = "Record ID"),
        ("content" = Option<String>, Query, description = "Remove contents or not"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn delete<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingRecord>,
{
    let remove_content = match query.get("content") {
        Some(content) if content == "remove" => true,
        _ => false,
    };
    state
        .recording_manager
        .call(recording::RemoveRecordingRecord { id, remove_content })
        .await?
}

pub(in crate::web::api) async fn stream<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
    ranges: Option<TypedHeader<axum::headers::Range>>,
) -> Result<Response, Error>
where
    R: Call<recording::QueryRecordingRecord>,
{
    // Use only the first start position for the seek support.
    let start = ranges
        .map(|TypedHeader(ranges)| {
            ranges
                .iter()
                .next()
                .map(|(start, _)| match start {
                    Bound::Included(n) => Some(n),
                    Bound::Excluded(n) => Some(n + 1),
                    _ => None,
                })
                .flatten()
        })
        .flatten()
        .unwrap_or(0);

    let record = state
        .recording_manager
        .call(recording::QueryRecordingRecord { id })
        .await??;

    let mut file = tokio::fs::File::open(record.content_path).await?;
    if start > 0 {
        file.seek(SeekFrom::Start(start)).await?;
    }
    let size = file.metadata().await?.len();

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, header_value!(record.content_type));
    headers.insert(ACCEPT_RANGES, header_value!("bytes"));
    headers.insert(CONTENT_RANGE, header_value!(format!("{}-", start)));

    let stream = ReaderStream::new(file);
    let body = StreamBody::new(stream);
    let body = SeekableStreamBody::new(body, size - start);

    if start > 0 {
        Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
    } else {
        Ok((headers, body).into_response())
    }
}
