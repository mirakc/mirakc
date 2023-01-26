use super::*;

use crate::recording::RecordingSchedule;

/// Lists recorders.
#[utoipa::path(
    get,
    path = "/recording/recorders",
    responses(
        (status = 200, description = "OK", body = [WebRecordingRecorder]),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn list<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
) -> Result<Json<Vec<WebRecordingRecorder>>, Error>
where
    R: Call<recording::QueryRecordingRecorders>,
{
    let mut results = vec![];
    let recorders = recording_manager
        .call(recording::QueryRecordingRecorders)
        .await?;
    for recorder in recorders.into_iter() {
        results.push(recorder.into());
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
pub(in crate::web::api) async fn get<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<ProgramId>,
) -> Result<Json<WebRecordingRecorder>, Error>
where
    R: Call<recording::QueryRecordingRecorder>,
{
    let recorder = recording_manager
        .call(recording::QueryRecordingRecorder { program_id })
        .await??;
    Ok(Json(recorder.into()))
}

/// Starts recording immediately.
///
/// > **Warning**: Use `POST /api/recording/schedules` instead.
/// > The recording will start even if the TV program has not started.
/// > In this case, the recording will always fail.
#[utoipa::path(
    post,
    path = "/recording/recorders",
    request_body = WebRecordingScheduleInput,
    responses(
        (status = 201, description = "Created"),
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
) -> Result<StatusCode, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::StartRecording>,
{
    input.validate(&config)?;
    let msg = epg::QueryProgram {
        program_id: input.program_id,
    };
    let program = epg.call(msg).await??;
    let schedule = RecordingSchedule::new(Arc::new(program), input.options, input.tags);
    let msg = recording::StartRecording { schedule };
    recording_manager.call(msg).await??;
    Ok(StatusCode::CREATED)
}

/// Stops recording.
///
/// Unlike `DELETE /api/recording/schedules/{program_id}`, this endpoint only
/// stops the recording without removing the corresponding recording schedule.
///
/// A `recording.stopped` event will occur
/// and `GET /api/recording/schedules/{program_id}` will return the schedule
/// information.
#[utoipa::path(
    delete,
    path = "/recording/recorders/{program_id}",
    params(
        ("program_id" = u64, Path, description = "Mirakurun program ID"),
    ),
    responses(
        (status = 201, description = "Created"),
        (status = 401, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn delete<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Path(program_id): Path<ProgramId>,
) -> Result<(), Error>
where
    R: Call<recording::StopRecording>,
{
    recording_manager
        .call(recording::StopRecording { program_id })
        .await??;
    Ok(())
}
