use super::*;

use crate::epg::EpgProgram;
use crate::epg::EpgService;
use crate::models::Clock;
use crate::web::api::stream::do_head_stream;
use crate::web::api::stream::streaming;
use crate::web::api::stream::StreamingHeaderParams;

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
#[allow(clippy::too_many_arguments)]
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
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
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
    Path(program_id): Path<ProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: TriggerFactory<tuner::StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryClock>,
    E: Clone + Send + Sync + 'static,
    O: Call<onair::SpawnTemporalTracker>,
{
    let service_id = program_id.into();
    let program = epg.call(epg::QueryProgram { program_id }).await??;
    let service = epg.call(epg::QueryService { service_id }).await??;
    let clock = epg.call(epg::QueryClock { service_id }).await??;

    let stream = tuner_manager
        .call(tuner::StartStreaming {
            channel: service.channel.clone(),
            user: user.clone(),
            stream_id: None,
        })
        .await??;

    // stream_stop_trigger must be created here in order to stop streaming when
    // an error occurs.
    let msg = tuner::StopStreaming { id: stream.id() };
    let stream_stop_trigger = tuner_manager.trigger(msg);

    let (filters, content_type, seekable) = build_filters(
        &config,
        &user,
        &filter_setting,
        &clock,
        &service,
        &program,
        stream.is_decoded(),
    )?;
    debug_assert!(!seekable);

    // Ignore the range header.

    if is_epgstation(&user_agent) {
        // The temporal tracker will stop within 1 minute after the streaming stopped.
        onair_manager
            .call(onair::SpawnTemporalTracker {
                service,
                stream_id: stream.id(),
            })
            .await?;
    }

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length: None,
        range: None,
        user,
    };

    let stop_triggers = vec![stream_stop_trigger];

    let result = streaming(&config, stream, filters, &params, stop_triggers).await;

    if let Err(Error::ProgramNotFound) = result {
        tracing::warn!(program.id = %program_id, "No stream for the program, maybe canceled");
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
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
    ),
    operation_id = "checkProgramStream",
)]
pub(in crate::web::api) async fn head<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(program_id): Path<ProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryClock>,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryService>,
{
    let service_id = program_id.into();
    let program = epg.call(epg::QueryProgram { program_id }).await??;
    let service = epg.call(epg::QueryService { service_id }).await??;
    let clock = epg.call(epg::QueryClock { service_id }).await??;

    let (_, content_type, seekable) = build_filters(
        &config,
        &user,
        &filter_setting,
        &clock,
        &service,
        &program,
        false, // This is a dummy but works properly.
    )?;
    debug_assert!(!seekable);

    let params = StreamingHeaderParams {
        seekable,
        content_type,
        length: None,
        range: None,
        user,
    };

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_head_stream(&params)
}

fn build_filters(
    config: &Config,
    user: &TunerUser,
    filter_setting: &FilterSetting,
    clock: &Clock,
    service: &EpgService,
    program: &EpgProgram,
    decoded: bool,
) -> Result<(Vec<String>, String, bool), Error> {
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
        .insert("user", &user)?
        .insert("sid", &program.id.sid().value())?
        .insert("eid", &program.id.eid().value())?
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
        let duration = Duration::try_seconds(max_start_delay.as_secs() as i64).unwrap();
        let wait_until = program.start_at.unwrap() + duration;
        builder = builder.insert("wait_until", &wait_until.timestamp_millis())?;
    }
    let data = builder.build();

    let mut builder = FilterPipelineBuilder::new(data, false); // not seekable
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    if !decoded && filter_setting.decode {
        builder.add_decode_filter(&config.filters.decode_filter)?;
    }
    builder.add_program_filter(&config.filters.program_filter)?;
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    Ok(builder.build())
}
