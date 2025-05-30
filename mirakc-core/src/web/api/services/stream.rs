use super::*;

use crate::epg::EpgChannel;
use crate::web::api::stream::StreamingHeaderParams;
use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;

/// Gets a media stream of a service.
#[utoipa::path(
    get,
    path = "/services/{id}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("id" = u64, Path, description = "Mirakurun service ID"),
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
    operation_id = "getServiceStream",
)]
pub(in crate::web::api) async fn get<T, E, W>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(SpawnerExtractor(spawner)): State<SpawnerExtractor<W>>,
    Path(service_id): Path<ServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: TriggerFactory<tuner::StopStreaming>,
    E: Call<epg::QueryService>,
    W: Spawn,
{
    let service = epg.call(epg::QueryService { service_id }).await??;

    do_get_service_stream(
        &config,
        &tuner_manager,
        &spawner,
        &service.channel,
        service_id.sid(),
        &user,
        &filter_setting,
    )
    .await
}

// IPTV Simple Client in Kodi sends a HEAD request before streaming.
#[utoipa::path(
    head,
    path = "/services/{id}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("id" = u64, Path, description = "Mirakurun service ID"),
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
    operation_id = "checkServiceStream",
)]
pub(in crate::web::api) async fn head<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(service_id): Path<ServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    E: Call<epg::QueryService>,
{
    let service = epg.call(epg::QueryService { service_id }).await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_service_stream(
        &config,
        &service.channel,
        service.sid(),
        &user,
        &filter_setting,
    )
    .await
}

pub(in crate::web::api) async fn do_get_service_stream<T, W>(
    config: &Config,
    tuner_manager: &T,
    spawner: &W,
    channel: &EpgChannel,
    sid: Sid,
    user: &TunerUser,
    filter_setting: &FilterSetting,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: TriggerFactory<tuner::StopStreaming>,
    W: Spawn,
{
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

    let (filters, content_type, seekable) = build_filters(
        config,
        user,
        filter_setting,
        channel,
        sid,
        stream.is_decoded(),
    )?;
    debug_assert!(!seekable);

    // Ignore the range header.

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length: None,
        range: None,
        user: user.clone(),
    };

    streaming(config, spawner, stream, filters, &params, stop_trigger).await
}

pub(in crate::web::api) async fn do_head_service_stream(
    config: &Config,
    channel: &EpgChannel,
    sid: Sid,
    user: &TunerUser,
    filter_setting: &FilterSetting,
) -> Result<Response, Error> {
    let (_, content_type, seekable) = build_filters(
        config,
        user,
        filter_setting,
        channel,
        sid,
        false, // This is a dummy but works properly.
    )?;
    debug_assert!(!seekable);

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length: None,
        range: None,
        user: user.clone(),
    };

    do_head_stream(&params)
}

fn build_filters(
    config: &Config,
    user: &TunerUser,
    filter_setting: &FilterSetting,
    channel: &EpgChannel,
    sid: Sid,
    decoded: bool,
) -> Result<(Vec<String>, String, bool), Error> {
    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &channel.name)
        .insert("channel_type", &channel.channel_type)?
        .insert_str("channel", &channel.channel)
        .insert("user", &user)?
        .insert("sid", &sid.value())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data, false); // not seekable
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    if !decoded && filter_setting.decode {
        builder.add_decode_filter(&config.filters.decode_filter)?;
    }
    builder.add_service_filter(&config.filters.service_filter)?;
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    Ok(builder.build())
}
