use super::*;

use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamRange;
use crate::web::api::stream::calc_start_pos_in_ranges;
use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;

/// Gets a media stream of a record.
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
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getRecordStream",
)]
pub(in crate::web::api) async fn get(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
    ranges: Option<TypedHeader<axum_extra::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error> {
    let record_path = make_record_path(&config, &id).unwrap();
    let (record, size) = load_record(&config, &record_path).await?;

    let content_path = make_content_path(&config, &record.options.content_path).unwrap();
    if !content_path.exists() {
        tracing::warn!(?content_path, "No such file, maybe it has been deleted");
        return Err(Error::NoContent);
    }

    let content = tokio::fs::File::open(&content_path).await?;

    // `size` must not be `None` at this point.
    let size = size.unwrap();

    let stream = tokio_util::io::ReaderStream::new(content);
    let stream = if ranges.is_none() {
        MpegTsStream::new(id, stream)
    } else {
        let start_pos = calc_start_pos_in_ranges(ranges, size);
        let range = if matches!(record.recording_status, RecordingStatus::Recording) {
            MpegTsStreamRange::unbound(start_pos, size)?
        } else {
            MpegTsStreamRange::bound(start_pos, size)?
        };
        MpegTsStream::with_range(id, stream, range)
    };

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
        .insert("size", &size)?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(&config, &user, stream, filters, content_type, ()).await
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
pub(in crate::web::api) async fn head(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    Path(id): Path<RecordId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error> {
    let record_path = make_record_path(&config, &id).unwrap();
    let (record, _) = load_record(&config, &record_path).await?;

    let content_path = make_content_path(&config, &record.options.content_path).unwrap();
    if !content_path.exists() {
        tracing::warn!(?content_path, "No such file, maybe it has been deleted");
        return Err(Error::NoContent);
    }

    do_head_stream(&config, &user, &filter_setting)
}
