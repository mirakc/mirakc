use super::*;

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelServicePath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Call<epg::QueryChannel>,
{
    let channel = state
        .epg
        .call(epg::QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    do_get_service_stream(
        &state.config,
        &state.tuner_manager,
        channel,
        path.sid,
        user,
        filter_setting,
    )
    .await
}

pub(in crate::web::api) async fn head<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelServicePath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryChannel>,
{
    let _channel = state
        .epg
        .call(epg::QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&state.config, &user, &filter_setting)
}
