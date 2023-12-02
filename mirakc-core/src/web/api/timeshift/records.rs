use super::*;

use std::ops::Bound;

use crate::models::TunerUser;
use crate::web::api::stream::streaming;

/// Lists timeshift records.
#[utoipa::path(
    get,
    path = "/timeshift/{recorder}/records",
    params(
        ("recorder" = String, Path, description = "Timeshift recorder name"),
    ),
    responses(
        (status = 200, description = "OK", body = [WebTimeshiftRecord]),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getTimeshiftRecords",
)]
pub(in crate::web::api) async fn list<S>(
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    Path(recorder): Path<String>,
) -> Result<Json<Vec<WebTimeshiftRecord>>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecords>,
{
    let msg = timeshift::QueryTimeshiftRecords {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    timeshift_manager
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

/// Gets a timeshift record.
#[utoipa::path(
    get,
    path = "/timeshift/{recorder}/records/{id}",
    params(
        TimeshiftRecordPath,
    ),
    responses(
        (status = 200, description = "OK", body = WebTimeshiftRecord),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getTimeshiftRecord",
)]
pub(in crate::web::api) async fn get<S>(
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    Path(path): Path<TimeshiftRecordPath>,
) -> Result<Json<WebTimeshiftRecord>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecord>,
{
    let msg = timeshift::QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder),
        record_id: path.id,
    };
    timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecord::from)
        .map(Json::from)
}

/// Gets a media stream of a timeshift record.
#[utoipa::path(
    get,
    path = "/timeshift/{recorder}/records/{id}/stream",
    params(
        TimeshiftRecordPath,
        ("pre-filters" = Option<[String]>, Query, description = "Pre-filters"),
        ("post-filters" = Option<[String]>, Query, description = "post-filters"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
    ),
    operation_id = "getTimeshiftRecordStream",
)]
pub(in crate::web::api) async fn stream<S>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    Path(path): Path<TimeshiftRecordPath>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
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
    let recorder = timeshift_manager.call(msg).await??;

    let msg = timeshift::QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id.clone(),
    };
    let record = timeshift_manager.call(msg).await??;

    let start_pos = if let Some(TypedHeader(ranges)) = ranges {
        ranges
            .satisfiable_ranges(record.size)
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
    let src = timeshift_manager.call(msg).await??;

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
        .insert("sid", &recorder.service.sid())?
        .insert("eid", &record.program.eid())?
        .insert("video_tags", &video_tags)?
        .insert("audio_tags", &audio_tags)?
        .insert("id", &record.id)?
        .insert("duration", &duration.num_seconds())?
        .insert("size", &record.size)?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(&config, user, stream, filters, content_type, stop_trigger).await
}
