use super::*;

use crate::web::api::stream::do_get_service_stream;
use crate::web::api::stream::do_head_stream;

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
        (status = 503, description = "Tuner Resource Unavailable"),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getServiceStream",
)]
pub(in crate::web::api) async fn get<T, E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(service_id): Path<ServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Call<epg::QueryService>,
{
    let service = epg.call(epg::QueryService { service_id }).await??;

    do_get_service_stream(
        &config,
        &tuner_manager,
        service.channel,
        service_id.sid(),
        user,
        filter_setting,
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
        (status = 503, description = "Tuner Resource Unavailable"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(in crate::web::api) async fn head<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(service_id): Path<ServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryService>,
{
    let _service = epg.call(epg::QueryService { service_id }).await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&config, &user, &filter_setting)
}
