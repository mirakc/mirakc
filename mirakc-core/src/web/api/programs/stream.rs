use super::*;

pub(in crate::web::api) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryClock>,
    E: Call<epg::RemoveAirtime>,
    E: Call<epg::UpdateAirtime>,
{
    let program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let clock = state
        .epg
        .call(epg::QueryClock {
            triple: service.triple(),
        })
        .await??;

    let stream = state
        .tuner_manager
        .call(tuner::StartStreaming {
            channel: service.channel.clone(),
            user: user.clone(),
        })
        .await??;

    // stream_stop_trigger must be created here in order to stop streaming when
    // an error occurs.
    let stream_stop_trigger =
        TunerStreamStopTrigger::new(stream.id(), state.tuner_manager.clone().into());

    let video_tags: Vec<u8> = program
        .video
        .iter()
        .map(|video| video.component_tag)
        .collect();

    let audio_tags: Vec<u8> = program
        .audios
        .values()
        .map(|audio| audio.component_tag)
        .collect();

    let mut builder = mustache::MapBuilder::new();
    builder = builder
        .insert_str("channel_name", &service.channel.name)
        .insert("channel_type", &service.channel.channel_type)?
        .insert_str("channel", &service.channel.channel)
        .insert("sid", &program.quad.sid().value())?
        .insert("eid", &program.quad.eid().value())?
        .insert("clock_pid", &clock.pid)?
        .insert("clock_pcr", &clock.pcr)?
        .insert("clock_time", &clock.time)?
        .insert("video_tags", &video_tags)?
        .insert("audio_tags", &audio_tags)?;
    if let Some(max_start_delay) = state.config.recording.max_start_delay {
        // Round off the fractional (nanosecond) part of the duration.
        //
        // The value can be safely converted into i64 because the value is less
        // than 24h.
        let duration = Duration::seconds(max_start_delay.as_secs() as i64);
        let wait_until = program.start_at + duration;
        builder = builder.insert("wait_until", &wait_until.timestamp_millis())?;
    }
    let data = builder.build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&state.config.pre_filters, &filter_setting.pre_filters)?;
    if !stream.is_decoded() && filter_setting.decode {
        builder.add_decode_filter(&state.config.filters.decode_filter)?;
    }
    builder.add_program_filter(&state.config.filters.program_filter)?;
    builder.add_post_filters(&state.config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    let tracker_stop_trigger = airtime_tracker::track_airtime(
        &state.config.recording.track_airtime_command,
        &service.channel,
        &program,
        stream.id(),
        state.tuner_manager.clone(),
        state.epg.clone(),
    )
    .await?;

    let stop_triggers = vec![stream_stop_trigger, tracker_stop_trigger];

    let result = streaming(
        &state.config,
        user,
        stream,
        filters,
        content_type,
        stop_triggers,
    )
    .await;

    match result {
        Err(Error::ProgramNotFound) => {
            tracing::warn!("No stream for the program#{}, maybe canceled", id)
        }
        _ => (),
    }

    result
}

pub(in crate::web::api) async fn head<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryClock>,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
{
    let _program = state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let _clock = state
        .epg
        .call(epg::QueryClock {
            triple: service.triple(),
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&state.config, &user, &filter_setting)
}
