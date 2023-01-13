use super::*;

use std::collections::HashMap;

use path_dedot::ParseDot;

use crate::recording::{RecordingSchedule, RecordingScheduleState};

/// Lists recording schedules.
#[utoipa::path(
    get,
    path = "/recording/schedules",
    responses(
        (status = 200, description = "OK", body = [WebRecordingSchedule]),
        (status = 505, description = "Internal Server Error"),
    ),
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
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn get<E, R>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<Json<WebRecordingSchedule>, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::QueryRecordingSchedule>,
{
    let program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    let schedule = recording_manager
        .call(recording::QueryRecordingSchedule {
            program_id: program.id,
        })
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
        (status = 505, description = "Internal Server Error"),
    ),
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
    if input.options.content_path.is_absolute() {
        let err = Error::InvalidPath;
        tracing::error!(%err, ?input.options.content_path);
        return Err(err);
    }

    let basedir = config.recording.basedir.as_ref().unwrap();
    if !basedir
        .join(&input.options.content_path)
        .parse_dot()?
        .starts_with(basedir)
    {
        let err = Error::InvalidPath;
        tracing::error!(%err, ?input.options.content_path);
        return Err(err);
    }

    let program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(input.program_id))
        .await??;

    let schedule = RecordingSchedule {
        state: RecordingScheduleState::Scheduled,
        program: Arc::new(program),
        options: input.options,
        tags: input.tags,
    };
    let schedule = recording_manager
        .call(recording::AddRecordingSchedule { schedule })
        .await??;

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
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn delete<E, R>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<(), Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::RemoveRecordingSchedule>,
{
    let program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(program_id))
        .await??;

    recording_manager
        .call(recording::RemoveRecordingSchedule {
            program_id: program.id,
        })
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
///   * Recording schedules in the `tracing` or `recording` state
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
        (status = 505, description = "Internal Server Error"),
    ),
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
