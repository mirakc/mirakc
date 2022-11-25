use std::sync::Arc;

use actlet::*;
use axum::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::epg;
use crate::error::Error;
use crate::models::*;
use crate::recording;
use crate::recording::Schedule;
use crate::web::models::*;
use crate::web::AppState;

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
            .call(epg::QueryProgram::ByMirakurunProgramId(
                recorder.schedule.program_id,
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
        .call(recording::QueryRecordingRecorder { program_id })
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
        program_id: input.program_id,
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

pub(in crate::web::api) async fn delete<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<(), Error>
where
    R: Call<recording::StopRecording>,
{
    state
        .recording_manager
        .call(recording::StopRecording { program_id })
        .await??;
    Ok(())
}
