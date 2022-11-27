use super::*;

use std::collections::HashMap;

use path_dedot::ParseDot;

use crate::recording::Schedule;

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

pub(in crate::web::api) async fn clear<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<recording::RemoveRecordingSchedules>,
{
    let target = match query.get("target") {
        Some(tag) => crate::recording::RemoveTarget::Tag(tag.clone()),
        None => crate::recording::RemoveTarget::All,
    };
    state
        .recording_manager
        .call(recording::RemoveRecordingSchedules { target })
        .await?;
    Ok(())
}
