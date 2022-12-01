use super::*;

use std::collections::HashMap;

use path_dedot::ParseDot;

use crate::recording::Schedule;

/// Lists recording schedules.
#[utoipa::path(
    get,
    path = "/recording/schedules",
    responses(
        (status = 200, description = "OK", body = [WebRecordingSchedule]),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingSchedule>>, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::QueryRecordingSchedules>,
{
    let mut results = vec![];
    let schedules = state
        .recording_manager
        .call(recording::QueryRecordingSchedules)
        .await?;
    for schedule in schedules.into_iter() {
        let program = state
            .epg
            .call(epg::QueryProgram::ByMirakurunProgramId(schedule.program_id))
            .await??;
        results.push(WebRecordingSchedule {
            program: program.into(),
            content_path: schedule.content_path.clone(),
            priority: schedule.priority,
            pre_filters: schedule.pre_filters.clone(),
            post_filters: schedule.post_filters.clone(),
            tags: schedule.tags.clone(),
        });
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
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<Json<WebRecordingSchedule>, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::QueryRecordingSchedule>,
{
    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    let schedule = state
        .recording_manager
        .call(recording::QueryRecordingSchedule { program_id })
        .await??;
    Ok(Json(WebRecordingSchedule {
        program: program.into(),
        content_path: schedule.content_path.clone(),
        priority: schedule.priority,
        pre_filters: schedule.pre_filters.clone(),
        post_filters: schedule.post_filters.clone(),
        tags: schedule.tags.clone(),
    }))
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
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn create<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<(StatusCode, Json<WebRecordingSchedule>), Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::AddRecordingSchedule>,
    R: Call<recording::QueryRecordingSchedule>,
{
    if input.content_path.is_absolute() {
        let err = Error::InvalidPath;
        tracing::error!(%err, ?input.content_path);
        return Err(err);
    }

    let contents_dir = state.config.recording.contents_dir.as_ref().unwrap();
    if !contents_dir
        .join(&input.content_path)
        .parse_dot()?
        .starts_with(contents_dir)
    {
        let err = Error::InvalidPath;
        tracing::error!(%err, ?input.content_path);
        return Err(err);
    }

    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(input.program_id))
        .await??;

    let schedule = Schedule {
        program_id: input.program_id,
        content_path: input.content_path,
        priority: input.priority,
        pre_filters: input.pre_filters,
        post_filters: input.post_filters,
        tags: input.tags,
        start_at: program.start_at,
    };
    let schedule = state
        .recording_manager
        .call(recording::AddRecordingSchedule { schedule })
        .await??;

    Ok((
        StatusCode::CREATED,
        Json(WebRecordingSchedule {
            program: program.into(),
            content_path: schedule.content_path.clone(),
            priority: schedule.priority,
            pre_filters: schedule.pre_filters.clone(),
            post_filters: schedule.post_filters.clone(),
            tags: schedule.tags.clone(),
        }),
    ))
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
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn delete<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingSchedule>,
{
    state
        .recording_manager
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
/// When deleting recording schedules by a tag, a recording schedule won't be
/// deleted if the recording will be started soon.
#[utoipa::path(
    delete,
    path = "/recording/schedules",
    params(
        ("tag" = Option<String>, Query, description = "Tag name"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn clear<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingSchedules>,
{
    let target = match query.get("tag") {
        Some(tag) => crate::recording::RemoveTarget::Tag(tag.clone()),
        None => crate::recording::RemoveTarget::All,
    };
    state
        .recording_manager
        .call(recording::RemoveRecordingSchedules { target })
        .await?;
    Ok(())
}
