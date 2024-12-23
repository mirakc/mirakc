use super::*;

use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;

/// Gets a media stream of a channel.
#[utoipa::path(
    get,
    path = "/channels/{type}/{channel}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("type" = ChannelType, Path, description = "Channel type"),
        ("channel" = String, Path, description = "Channel number"),
        FilterSetting,
    ),
    responses(
        (status = 200, description = "OK",
         headers(
             ("X-Mirakurun-Tuner-User-ID" = String, description = "Tuner user ID"),
         ),
        ),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getChannelStream",
)]
pub(in crate::web::api) async fn get<T, E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(path): Path<ChannelPath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: TriggerFactory<tuner::StopStreaming>,
    E: Call<epg::QueryChannel>,
{
    let channel = epg
        .call(epg::QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    let stream = tuner_manager
        .call(tuner::StartStreaming {
            channel: channel.clone(),
            user: user.clone(),
            stream_id: None,
        })
        .await??;

    // stop_trigger must be created here in order to stop streaming when an
    // error occurs.
    let msg = tuner::StopStreaming { id: stream.id() };
    let stop_trigger = tuner_manager.trigger(msg);

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &channel.name)
        .insert("channel_type", &channel.channel_type)?
        .insert_str("channel", &channel.channel)
        .insert("user", &user)?
        .build();

    let mut builder = FilterPipelineBuilder::new(data, false); // not seekable
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    if !stream.is_decoded() && filter_setting.decode {
        builder.add_decode_filter(&config.filters.decode_filter)?;
    }
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type, _) = builder.build();

    streaming(&config, &user, stream, filters, content_type, stop_trigger).await
}

#[utoipa::path(
    head,
    path = "/channels/{type}/{channel}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("type" = ChannelType, Path, description = "Channel type"),
        ("channel" = String, Path, description = "Channel number"),
        FilterSetting,
    ),
    responses(
        (status = 200, description = "OK",
         headers(
             ("X-Mirakurun-Tuner-User-ID" = String, description = "Tuner user ID"),
         ),
        ),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
    ),
    operation_id = "checkChannelStream",
)]
pub(in crate::web::api) async fn head<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(path): Path<ChannelPath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryChannel>,
{
    let _channel = epg
        .call(epg::QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&config, &user, &filter_setting)
}
