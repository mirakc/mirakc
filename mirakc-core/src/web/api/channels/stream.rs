use super::*;

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelPath>,
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

    let stream = state
        .tuner_manager
        .call(tuner::StartStreaming {
            channel: channel.clone(),
            user: user.clone(),
        })
        .await??;

    // stop_trigger must be created here in order to stop streaming when an
    // error occurs.
    let stop_trigger = TunerStreamStopTrigger::new(stream.id(), state.tuner_manager.clone().into());

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &channel.name)
        .insert("channel_type", &channel.channel_type)?
        .insert_str("channel", &channel.channel)
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&state.config.pre_filters, &filter_setting.pre_filters)?;
    if !stream.is_decoded() && filter_setting.decode {
        builder.add_decode_filter(&state.config.filters.decode_filter)?;
    }
    builder.add_post_filters(&state.config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(
        &state.config,
        user,
        stream,
        filters,
        content_type,
        stop_trigger,
    )
    .await
}

pub(in crate::web::api) async fn head<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelPath>,
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
