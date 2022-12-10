use super::*;

use crate::recording::Schedule;

/// Lists recorders.
#[utoipa::path(
    get,
    path = "/recording/recorders",
    responses(
        (status = 200, description = "OK", body = [WebRecordingRecorder]),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingRecorder>>, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::QueryRecordingRecorders>,
{
    let mut results = vec![];
    let recorders = state
        .recording_manager
        .call(recording::QueryRecordingRecorders)
        .await?;
    for recorder in recorders.into_iter() {
        let program = state
            .epg
            .call(epg::QueryProgram::ByProgramQuad(
                recorder.schedule.program_quad,
            ))
            .await??;
        results.push(WebRecordingRecorder {
            program: program.into(),
            content_path: recorder.schedule.content_path.clone(),
            priority: recorder.schedule.priority,
            pipeline: recorder
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            tags: recorder.schedule.tags.clone(),
            start_time: recorder.start_time,
        });
    }
    Ok(Json(results))
}

/// Gets a recorder.
#[utoipa::path(
    get,
    path = "/recording/recorders/{program_id}",
    params(
        ("program_id" = u64, Path, description = "Mirakurun program ID"),
    ),
    responses(
        (status = 200, description = "OK", body = WebRecordingRecorder),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<Json<WebRecordingRecorder>, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::QueryRecordingRecorder>,
{
    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    let recorder = state
        .recording_manager
        .call(recording::QueryRecordingRecorder {
            program_quad: program.quad,
        })
        .await??;
    Ok(Json(WebRecordingRecorder {
        program: program.into(),
        content_path: recorder.schedule.content_path.clone(),
        priority: recorder.schedule.priority,
        pipeline: recorder
            .pipeline
            .into_iter()
            .map(WebProcessModel::from)
            .collect(),
        tags: recorder.schedule.tags.clone(),
        start_time: recorder.start_time,
    }))
}

/// Starts recording.
#[utoipa::path(
    post,
    path = "/recording/recorders",
    request_body = WebRecordingScheduleInput,
    responses(
        (status = 201, description = "Created", body = WebRecordingRecorder),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn create<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<(StatusCode, Json<WebRecordingRecorder>), Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::StartRecording>,
{
    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(input.program_id))
        .await??;
    let schedule = Arc::new(Schedule {
        program_quad: program.quad,
        content_path: input.content_path,
        priority: input.priority,
        pre_filters: input.pre_filters,
        post_filters: input.post_filters,
        tags: input.tags,
        start_at: program.start_at,
    });
    let recorder = state
        .recording_manager
        .call(recording::StartRecording { schedule })
        .await??;
    Ok((
        StatusCode::CREATED,
        Json(WebRecordingRecorder {
            program: program.into(),
            content_path: recorder.schedule.content_path.clone(),
            priority: recorder.schedule.priority,
            pipeline: recorder
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            tags: recorder.schedule.tags.clone(),
            start_time: recorder.start_time,
        }),
    ))
}

/// Stops recording.
#[utoipa::path(
    delete,
    path = "/recording/recorders/{program_id}",
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
    E: Call<epg::QueryProgram>,
    R: Call<recording::StopRecording>,
{
    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    state
        .recording_manager
        .call(recording::StopRecording {
            program_quad: program.quad,
        })
        .await??;
    Ok(())
}
