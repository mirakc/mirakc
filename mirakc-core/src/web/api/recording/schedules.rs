use super::*;

use std::collections::HashMap;

use crate::recording::RecordingSchedule;

/// Lists recording schedules.
#[utoipa::path(
    get,
    path = "/recording/schedules",
    responses(
        (status = 200, description = "OK", body = [WebRecordingSchedule]),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordingSchedules",
)]
pub(in crate::web::api) async fn list<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
) -> Result<Json<Vec<WebRecordingSchedule>>, Error>
where
    R: Call<recording::QueryRecordingSchedules>,
{
    let mut results = vec![];
    let schedules = recording_manager
        .call(recording::QueryRecordingSchedules)
        .await?;
    for schedule in schedules.into_iter() {
        results.push(schedule.into());
    }

    Ok(Json(results))
}

/// Gets a recording schedule.
#[utoipa::path(
    get,
    path = "/recording/schedules/{program_id}",
    params(
        ("program_id" = u64, Path, description = "Mirakurun program ID"),
    ),
    responses(
        (status = 200, description = "OK", body = WebRecordingSchedule),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordingSchedule",
)]
pub(in crate::web::api) async fn get<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<ProgramId>,
) -> Result<Json<WebRecordingSchedule>, Error>
where
    R: Call<recording::QueryRecordingSchedule>,
{
    let schedule = recording_manager
        .call(recording::QueryRecordingSchedule { program_id })
        .await??;
    Ok(Json(schedule.into()))
}

/// Books a recording schedule.
#[utoipa::path(
    post,
    path = "/recording/schedules",
    request_body = WebRecordingScheduleInput,
    responses(
        (status = 201, description = "Created", body = WebRecordingSchedule),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "createRecordingSchedule",
)]
pub(in crate::web::api) async fn create<E, R>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<(StatusCode, Json<WebRecordingSchedule>), Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::AddRecordingSchedule>,
    R: Call<recording::QueryRecordingSchedule>,
{
    input.validate(&config)?;
    let msg = epg::QueryProgram {
        program_id: input.program_id,
    };
    let program = epg.call(msg).await??;
    let schedule = RecordingSchedule::new(Arc::new(program), input.options, input.tags);
    let msg = recording::AddRecordingSchedule { schedule };
    let schedule = recording_manager.call(msg).await??;
    Ok((StatusCode::CREATED, Json(schedule.into())))
}

/// Deletes a recording schedule.
#[utoipa::path(
    delete,
    path = "/recording/schedules/{program_id}",
    params(
        ("program_id" = u64, Path, description = "Mirakurun program ID"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "deleteRecordingSchedule",
)]
pub(in crate::web::api) async fn delete<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<ProgramId>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingSchedule>,
{
    recording_manager
        .call(recording::RemoveRecordingSchedule { program_id })
        .await??;
    Ok(())
}

/// Clears recording schedules.
///
/// If a tag name is specified in the `tag` query parameter, recording schedules
/// tagged with the specified name will be deleted.  Otherwise, all recording
/// schedules will be deleted.
///
/// When deleting recording schedules by a tag, recording schedules that meet
/// any of the following conditions won't be deleted:
///
///   * Recording schedules without the specified tag
///   * Recording schedules in the `tracking` or `recording` state
///   * Recording schedules in the `scheduled` state and will start recording
///     soon
#[utoipa::path(
    delete,
    path = "/recording/schedules",
    params(
        ("tag" = Option<String>, Query, description = "Tag name"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "deleteRecordingSchedules",
)]
pub(in crate::web::api) async fn clear<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingSchedules>,
{
    let target = match query.get("tag") {
        Some(tag) => crate::recording::RemovalTarget::Tag(tag.clone()),
        None => crate::recording::RemovalTarget::All,
    };
    recording_manager
        .call(recording::RemoveRecordingSchedules { target })
        .await?;
    Ok(())
}
