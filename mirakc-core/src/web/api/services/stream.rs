use super::*;

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Call<epg::QueryService>,
{
    let service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    do_get_service_stream(
        &state.config,
        &state.tuner_manager,
        service.channel,
        service.sid,
        user,
        filter_setting,
    )
    .await
}

// IPTV Simple Client in Kodi sends a HEAD request before streaming.
pub(in crate::web::api) async fn head<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryService>,
{
    let _service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&state.config, &user, &filter_setting)
}
