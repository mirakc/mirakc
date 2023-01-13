use super::*;

use crate::recording::{RecordingSchedule, RecordingScheduleState};

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
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<StatusCode, Error>
where
    E: Call<epg::QueryProgram>,
    R: Call<recording::StartRecording>,
{
    let program = epg
        .call(epg::QueryProgram {
            program_id: input.program_id,
        })
        .await??;
    let schedule = RecordingSchedule {
        state: RecordingScheduleState::Recording,
        program: Arc::new(program),
        options: input.options,
        tags: input.tags,
    };
    recording_manager
        .call(recording::StartRecording { schedule })
        .await??;
    Ok(StatusCode::CREATED)
}
