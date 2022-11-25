pub(in crate::web::api) mod records;

use super::*;
use crate::filter::FilterPipelineBuilder;
use crate::models::TimeshiftRecordId;
use crate::models::TunerUser;
use crate::timeshift::TimeshiftRecorderQuery;
use crate::web::api::streaming;

pub(in crate::web::api) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebTimeshiftRecorder>>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecorders>,
{
    state
        .timeshift_manager
        .call(timeshift::QueryTimeshiftRecorders)
        .await?
        .map(|recorders| {
            recorders
                .into_iter()
                .map(WebTimeshiftRecorder::from)
                .collect::<Vec<WebTimeshiftRecorder>>()
        })
        .map(Json::from)
}

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder): Path<String>,
) -> Result<Json<WebTimeshiftRecorder>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecorder>,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecorder::from)
        .map(Json::from)
}

pub(in crate::web::api) async fn stream<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder_id): Path<String>,
    record_id: Option<Query<TimeshiftRecordId>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    S: Call<timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
    };
    let recorder = state.timeshift_manager.call(msg).await??;

    let msg = timeshift::CreateTimeshiftLiveStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
        record_id: record_id.map(|Query(id)| id),
    };
    let src = state.timeshift_manager.call(msg).await??;

    let (stream, stop_trigger) = src.create_stream().await?;

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &recorder.service.channel.name)
        .insert("channel_type", &recorder.service.channel.channel_type)?
        .insert_str("channel", &recorder.service.channel.channel)
        .insert("sid", &recorder.service.sid.value())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&state.config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&state.config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(
        &state.config,
        user,
        stream,
        filters,
        content_type,
        stop_trigger,
    )
    .await
}
