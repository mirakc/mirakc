use super::*;

use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;

/// Gets a media stream of a program.
///
/// ### A special hack for EPGStation
///
/// If the User-Agent header string starts with "EPGStation/", this endpoint
/// creates a temporal on-air program tracker if there is no tracker defined in
/// config.yml, which can be reused for tracking changes of the TV program
/// metadata.
///
/// The temporal on-air program tracker will be stopped within 1 minute after
/// the streaming stopped.
///
/// The metadata will be returned from [/programs/{id}](#/programs/getProgram).
#[utoipa::path(
    get,
    path = "/programs/{id}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("id" = u64, Path, description = "Mirakurun program ID"),
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
    operation_id = "getProgramStream",
)]
pub(in crate::web::api) async fn get<T, E, O>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
    user_agent: Option<TypedHeader<UserAgent>>,
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
    E: Clone + Send + Sync + 'static,
    O: Call<onair::SpawnTemporalTracker>,
{
    let program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = epg
        .call(epg::QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let clock = epg
        .call(epg::QueryClock {
            service_triple: service.triple(),
        })
        .await??;

    let stream = tuner_manager
        .call(tuner::StartStreaming {
            channel: service.channel.clone(),
            user: user.clone(),
            stream_id: None,
        })
        .await??;

    // stream_stop_trigger must be created here in order to stop streaming when
    // an error occurs.
    let stream_stop_trigger =
        TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

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
    if let Some(max_start_delay) = config.server.program_stream_max_start_delay {
        // Round off the fractional (nanosecond) part of the duration.
        //
        // The value can be safely converted into i64 because the value is less
        // than 24h.
        let duration = Duration::seconds(max_start_delay.as_secs() as i64);
        let wait_until = program.start_at.unwrap() + duration;
        builder = builder.insert("wait_until", &wait_until.timestamp_millis())?;
    }
    let data = builder.build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    if !stream.is_decoded() && filter_setting.decode {
        builder.add_decode_filter(&config.filters.decode_filter)?;
    }
    builder.add_program_filter(&config.filters.program_filter)?;
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    if is_epgstation(&user_agent) {
        // The temporal tracker will stop within 1 minute after the streaming stopped.
        onair_manager.call(onair::SpawnTemporalTracker {
            service,
            stream_id: stream.id(),
        })
        .await?;
    }

    let stop_triggers = vec![stream_stop_trigger];

    let result = streaming(&config, user, stream, filters, content_type, stop_triggers).await;

    match result {
        Err(Error::ProgramNotFound) => {
            tracing::warn!("No stream for the program#{}, maybe canceled", id)
        }
        _ => (),
    }

    result
}

#[utoipa::path(
    head,
    path = "/programs/{id}/stream",
    params(
        ("X-Mirakurun-Priority" = Option<i32>, Header, description = "Priority of the tuner user"),
        ("id" = u64, Path, description = "Mirakurun program ID"),
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
    Path(id): Path<MirakurunProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryClock>,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
{
    let _program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = epg
        .call(epg::QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let _clock = epg
        .call(epg::QueryClock {
            service_triple: service.triple(),
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&config, &user, &filter_setting)
}
