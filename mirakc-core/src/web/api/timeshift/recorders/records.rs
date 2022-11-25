use super::*;

use std::ops::Bound;

use crate::filter::FilterPipelineBuilder;
use crate::models::TunerUser;
use crate::web::api::streaming;

pub(in crate::web::api) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder): Path<String>,
) -> Result<Json<Vec<WebTimeshiftRecord>>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecords>,
{
    let msg = timeshift::QueryTimeshiftRecords {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(|records| {
            records
                .into_iter()
                .map(WebTimeshiftRecord::from)
                .collect::<Vec<WebTimeshiftRecord>>()
        })
        .map(Json::from)
}

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<TimeshiftRecordPath>,
) -> Result<Json<WebTimeshiftRecord>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecord>,
{
    let msg = timeshift::QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder),
        record_id: path.id,
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecord::from)
        .map(Json::from)
}

pub(in crate::web::api) async fn stream<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<TimeshiftRecordPath>,
    ranges: Option<TypedHeader<axum::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    S: Call<timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecord>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
    };
    let recorder = state.timeshift_manager.call(msg).await??;

    let msg = timeshift::QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id.clone(),
    };
    let record = state.timeshift_manager.call(msg).await??;

    let start_pos = if let Some(TypedHeader(ranges)) = ranges {
        ranges
            .iter()
            .next()
            .map(|(start, _)| match start {
                Bound::Included(n) => Some(n),
                Bound::Excluded(n) => Some(n + 1),
                _ => None,
            })
            .flatten()
    } else {
        None
    };

    let msg = timeshift::CreateTimeshiftRecordStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id.clone(),
        start_pos,
    };
    let src = state.timeshift_manager.call(msg).await??;

    // We assume that pre-filters don't change TS packets.
    let seekable = filter_setting.post_filters.is_empty();

    let (stream, stop_trigger) = src.create_stream(seekable).await?;

    let video_tags: Vec<u8> = record
        .program
        .video
        .iter()
        .map(|video| video.component_tag)
        .collect();

    let audio_tags: Vec<u8> = record
        .program
        .audios
        .values()
        .map(|audio| audio.component_tag)
        .collect();

    let duration = record.end_time - record.start_time;

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &recorder.service.channel.name)
        .insert("channel_type", &recorder.service.channel.channel_type)?
        .insert_str("channel", &recorder.service.channel.channel)
        .insert("sid", &recorder.service.sid.value())?
        .insert("eid", &record.program.quad.eid())?
        .insert("video_tags", &video_tags)?
        .insert("audio_tags", &audio_tags)?
        .insert("id", &record.id)?
        .insert("duration", &duration.num_seconds())?
        .insert("size", &record.size)?
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
