use super::*;

use crate::models::TunerUser;
use crate::timeshift::TimeshiftRecordModel;
use crate::timeshift::TimeshiftRecorderModel;
use crate::web::api::stream::compute_content_length;
use crate::web::api::stream::compute_content_range;
use crate::web::api::stream::streaming;
use crate::web::api::stream::StreamingHeaderParams;

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
pub(in crate::web::api) async fn stream<S, W>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    State(SpawnerExtractor(spawner)): State<SpawnerExtractor<W>>,
    Path(path): Path<TimeshiftRecordPath>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    S: Call<timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecord>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
    W: Spawn,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
    };
    let recorder = timeshift_manager.call(msg).await??;

    let msg = timeshift::QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id,
    };
    let record = timeshift_manager.call(msg).await??;

    let (filters, content_type, seekable) =
        build_filters(&config, &filter_setting, &recorder, &record)?;
    let range = compute_content_range(&ranges, record.size, record.recording, seekable)?;
    let length = compute_content_length(record.size, record.recording, range.as_ref());

    let msg = timeshift::CreateTimeshiftRecordStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id,
        range: range.clone(),
    };
    let src = timeshift_manager.call(msg).await??;

    let (stream, stop_trigger) = src.create_stream(seekable).await?;

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length,
        range,
        user,
    };

    streaming(&config, &spawner, stream, filters, &params, stop_trigger).await
}

fn build_filters(
    config: &Config,
    filter_setting: &FilterSetting,
    recorder: &TimeshiftRecorderModel,
    record: &TimeshiftRecordModel,
) -> Result<(Vec<String>, String, bool), Error> {
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

    let mut builder = FilterPipelineBuilder::new(data, true); // seekable by default
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    Ok(builder.build())
}
