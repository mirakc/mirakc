use super::*;

/// Lists recording history.
#[utoipa::path(
    get,
    path = "/recording/history",
    responses(
        (status = 200, description = "OK", body = [WebRecordingSchedule]),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordingHistory",
)]
pub(in crate::web::api) async fn list<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
) -> Result<Json<Vec<WebRecordingSchedule>>, Error>
where
    R: Call<recording::QueryRecordingHistory>,
{
    let mut results = vec![];
    let schedules = recording_manager
        .call(recording::QueryRecordingHistory)
        .await?;
    for schedule in schedules.into_iter() {
        results.push(schedule.into());
    }

    Ok(Json(results))
}
