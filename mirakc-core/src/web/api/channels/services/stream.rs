use super::*;

use crate::web::api::stream::do_get_service_stream;
use crate::web::api::stream::do_head_stream;

/// Gets a media stream of a service.
#[utoipa::path(
    get,
    path = "/channels/{type}/{channel}/services/{sid}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("type" = ChannelType, Path, description = "Channel type"),
        ("channel" = String, Path, description = "Channel number"),
        ("sid" = u16, Path, description = "Service ID (not Mirakurun Service ID)"),
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
    operation_id = "getServiceStreamByChannel",
)]
pub(in crate::web::api) async fn get<T, E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(path): Path<ChannelServicePath>,
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

    do_get_service_stream(
        &config,
        &tuner_manager,
        channel,
        path.sid,
        user,
        filter_setting,
    )
    .await
}

#[utoipa::path(
    head,
    path = "/channels/{type}/{channel}/services/{sid}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("type" = ChannelType, Path, description = "Channel type"),
        ("channel" = String, Path, description = "Channel number"),
        ("sid" = u16, Path, description = "Service ID (not Mirakurun Service ID)"),
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
)]
pub(in crate::web::api) async fn head<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(path): Path<ChannelServicePath>,
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
