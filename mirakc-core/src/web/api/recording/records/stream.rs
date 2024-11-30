use super::*;

use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;

/// Gets a media stream of the content of a record.
///
/// It's possible to get a media stream of a record even while it's recording.  Data will be sent
/// when data is appended to the content file event if the stream reaches EOF at some point.
///
/// A request for a record with no content file always returns status code 204.
///
/// A range requests with filters causes an error response with status code 400.
#[utoipa::path(
    get,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = String, Path, description = "Record ID"),
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
pub(in crate::web::api) async fn get<R>(
    State(RecordingManagerExtractor(recording_manager)): State<RecordingManagerExtractor<R>>,
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    R: Call<recording::OpenContent>,
    R: Call<recording::QueryRecord>,
{
    let (record, content_length) = recording_manager
        .call(recording::QueryRecord { id: id.clone() })
        .await??;

    let content_length = match content_length {
        Some(content_length) if content_length > 0 => content_length,
        _ => return Err(Error::NoContent),
    };

    if ranges.is_some() && !filter_setting.post_filters.is_empty() {
        return Err(Error::InvalidRequest(
            "Filters cannot be used in range requests",
        ));
    }

    let ranges = match ranges {
        Some(TypedHeader(ranges)) => ranges
            .satisfiable_ranges(match record.recording_status {
                RecordingStatus::Recording => 0,
                _ => content_length,
            })
            .collect(),
        None => vec![],
    };

    let (stream, stop_trigger) = recording_manager
        .call(recording::OpenContent::new(id.clone(), ranges))
        .await??;

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

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(&config, &user, stream, filters, content_type, stop_trigger).await
}

#[utoipa::path(
    head,
    path = "/recording/records/{id}/stream",
    params(
        ("id" = String, Path, description = "Record ID"),
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
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    R: Call<recording::QueryRecord>,
{
    let (_record, content_length) = recording_manager
        .call(recording::QueryRecord { id: id.clone() })
        .await??;

    let _content_length = match content_length {
        Some(content_length) if content_length > 0 => content_length,
        _ => return Err(Error::NoContent),
    };

    do_head_stream(&config, &user, &filter_setting)
}
