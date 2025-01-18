use super::*;

use crate::recording::Record;
use crate::web::api::stream::compute_content_length;
use crate::web::api::stream::compute_content_range;
use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;
use crate::web::api::stream::StreamingHeaderParams;

/// Gets a media stream of the content of a record.
///
/// It's possible to get a media stream of the record even while it's recording.  In this case, data
/// will be sent when data is appended to the content file event if the stream reaches EOF at that
/// point.  The streaming will stop within 2 seconds after the stream reaches the *true* EOF.
///
/// A request for a record without content file always returns status code 204.
///
/// A range request with filters always causes an error response with status code 400.
#[utoipa::path(
    get,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = String, Path, description = "Record ID"),
        ("pre-filters" = Option<[String]>, Query, description = "pre-filters"),
        ("post-filters" = Option<[String]>, Query, description = "post-filters"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 204, description = "No Content"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordStream",
)]
pub(in crate::web::api) async fn get<R, W>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(SpawnerExtractor(spawner)): State<SpawnerExtractor<W>>,
    Path(id): Path<RecordId>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    R: Call<recording::OpenContent>,
    R: Call<recording::QueryRecord>,
    W: Spawn,
{
    let (record, content_length) = recording_manager
        .call(recording::QueryRecord { id: id.clone() })
        .await??;

    let content_length = match content_length {
        Some(content_length) if content_length > 0 => content_length,
        _ => return Err(Error::NoContent),
    };

    let (filters, content_type, seekable) = build_filters(&config, &filter_setting, &record)?;
    let incomplete = matches!(record.recording_status, RecordingStatus::Recording);
    let range = compute_content_range(&ranges, content_length, incomplete, seekable)?;
    let length = compute_content_length(content_length, incomplete, range.as_ref());

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length,
        range,
        user,
    };

    let (stream, stop_trigger) = recording_manager
        .call(recording::OpenContent::new(
            id.clone(),
            params.range.clone(),
        ))
        .await??;

    streaming(&config, &spawner, stream, filters, &params, stop_trigger).await
}

#[utoipa::path(
    head,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = String, Path, description = "Record ID"),
        ("pre-filters" = Option<[String]>, Query, description = "pre-filters"),
        ("post-filters" = Option<[String]>, Query, description = "post-filters"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 204, description = "No Content"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "checkRecordStream",
)]
pub(in crate::web::api) async fn head<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    R: Call<recording::QueryRecord>,
{
    let (record, content_length) = recording_manager
        .call(recording::QueryRecord { id: id.clone() })
        .await??;

    let content_length = match content_length {
        Some(content_length) if content_length > 0 => content_length,
        _ => return Err(Error::NoContent),
    };

    let (_, content_type, seekable) = build_filters(&config, &filter_setting, &record)?;
    let incomplete = matches!(record.recording_status, RecordingStatus::Recording);
    let range = compute_content_range(&ranges, content_length, incomplete, seekable)?;
    let length = compute_content_length(content_length, incomplete, range.as_ref());

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length,
        range,
        user,
    };

    do_head_stream(&params)
}

fn build_filters(
    config: &Config,
    filter_setting: &FilterSetting,
    record: &Record,
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

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &record.service.channel.name)
        .insert("channel_type", &record.service.channel.channel_type)?
        .insert_str("channel", &record.service.channel.channel)
        .insert("sid", &record.program.id.sid().value())?
        .insert("eid", &record.program.id.eid().value())?
        .insert("video_tags", &video_tags)?
        .insert("audio_tags", &audio_tags)?
        .insert("id", &record.id.value())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data, true); // seekable by default
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    Ok(builder.build())
}
