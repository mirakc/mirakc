mod access_control;
mod body;
mod default_headers;
mod escape;
mod qs;
mod uds;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::ops::Bound;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use actlet::*;
use axum::async_trait;
use axum::body::StreamBody;
use axum::extract::FromRequestParts;
use axum::extract::Host;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONNECTION;
use axum::http::header::CONTENT_RANGE;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::SERVER;
use axum::http::header::TRANSFER_ENCODING;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing;
use axum::Json;
use axum::Router;
use axum::Server;
use axum::TypedHeader;
use bytes::Bytes;
use chrono::DateTime;
use chrono::Duration;
use futures::future::join_all;
use futures::future::FutureExt;
use futures::stream::Stream;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncSeekExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;
use tower_http::services::ServeDir;
use tower_http::services::ServeFile;
use tower_http::trace::TraceLayer;

use crate::airtime_tracker;
use crate::command_util::spawn_pipeline;
use crate::command_util::CommandPipelineProcessModel;
use crate::config::Config;
use crate::datetime_ext::serde_duration_in_millis;
use crate::datetime_ext::serde_jst;
use crate::datetime_ext::Jst;
use crate::epg::EpgChannel;
use crate::epg::QueryChannel;
use crate::epg::QueryChannels;
use crate::epg::QueryClock;
use crate::epg::QueryProgram;
use crate::epg::QueryPrograms;
use crate::epg::QueryService;
use crate::epg::QueryServices;
use crate::epg::RemoveAirtime;
use crate::epg::UpdateAirtime;
use crate::error::Error;
use crate::filter::FilterPipelineBuilder;
use crate::models::*;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamRange;
use crate::mpeg_ts_stream::MpegTsStreamTerminator;
use crate::recording::AddRecordingSchedule;
use crate::recording::QueryRecordingRecord;
use crate::recording::QueryRecordingRecorder;
use crate::recording::QueryRecordingRecorders;
use crate::recording::QueryRecordingRecords;
use crate::recording::QueryRecordingSchedule;
use crate::recording::QueryRecordingSchedules;
use crate::recording::Record;
use crate::recording::RemoveRecordingRecord;
use crate::recording::RemoveRecordingSchedule;
use crate::recording::RemoveRecordingSchedules;
use crate::recording::Schedule;
use crate::recording::StartRecording;
use crate::recording::StopRecording;
use crate::string_table::StringTable;
use crate::timeshift::CreateTimeshiftLiveStreamSource;
use crate::timeshift::CreateTimeshiftRecordStreamSource;
use crate::timeshift::QueryTimeshiftRecord;
use crate::timeshift::QueryTimeshiftRecorder;
use crate::timeshift::QueryTimeshiftRecorders;
use crate::timeshift::QueryTimeshiftRecords;
use crate::timeshift::TimeshiftRecordModel;
use crate::timeshift::TimeshiftRecorderModel;
use crate::timeshift::TimeshiftRecorderQuery;
use crate::tuner::QueryTuners;
use crate::tuner::StartStreaming;
use crate::tuner::StopStreaming;
use crate::tuner::TunerStreamStopTrigger;
use crate::web::access_control::AccessControlLayer;
use crate::web::body::SeekableStreamBody;
use crate::web::body::StaticFileBody;
use crate::web::default_headers::DefaultHeadersLayer;
use crate::web::escape::escape;
use crate::web::qs::Qs;
use crate::web::uds::UdsListener;

pub async fn serve<T, E, R, S>(
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
) -> Result<(), Error>
where
    T: Clone + Send + Sync + 'static,
    T: Call<QueryTuners>,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryChannel>,
    E: Call<QueryChannels>,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<QueryServices>,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
    R: Send + Sync + 'static,
    R: Call<AddRecordingSchedule>,
    R: Call<QueryRecordingRecord>,
    R: Call<QueryRecordingRecorder>,
    R: Call<QueryRecordingRecorders>,
    R: Call<QueryRecordingRecords>,
    R: Call<QueryRecordingSchedule>,
    R: Call<QueryRecordingSchedules>,
    R: Call<RemoveRecordingRecord>,
    R: Call<RemoveRecordingSchedule>,
    R: Call<RemoveRecordingSchedules>,
    R: Call<StartRecording>,
    R: Call<StopRecording>,
    S: Send + Sync + 'static,
    S: Call<CreateTimeshiftLiveStreamSource>,
    S: Call<CreateTimeshiftRecordStreamSource>,
    S: Call<QueryTimeshiftRecord>,
    S: Call<QueryTimeshiftRecords>,
    S: Call<QueryTimeshiftRecorder>,
    S: Call<QueryTimeshiftRecorders>,
{
    let app = build_app(Arc::new(AppState {
        config: config.clone(),
        string_table,
        tuner_manager,
        epg,
        recording_manager,
        timeshift_manager,
    }));

    let http_servers = config
        .server
        .http_addrs()
        .map(|addr| serve_http(addr, app.clone()).boxed());
    let uds_servers = config
        .server
        .uds_paths()
        .map(|path| serve_uds(path, app.clone()).boxed());

    let servers = http_servers.chain(uds_servers);
    join_all(servers).await;

    Ok(())
}

// http

async fn serve_http(addr: SocketAddr, app: Router) -> hyper::Result<()> {
    Server::bind(&addr)
        .http1_keepalive(false)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
}

// uds

async fn serve_uds(path: &std::path::Path, app: Router) -> hyper::Result<()> {
    // Cleanup the previous socket if it exists.
    let _ = tokio::fs::remove_file(&path).await;
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();

    Server::builder(UdsListener::new(path))
        .http1_keepalive(false)
        .serve(app.into_make_service())
        .await
}

// macros

macro_rules! header_value {
    ($v:literal) => {
        HeaderValue::from_static($v)
    };
    ($v:expr) => {
        HeaderValue::from_str(&$v).unwrap()
    };
}

// headers

const X_MIRAKURUN_TUNER_USER_ID: &'static str = "x-mirakurun-tuner-user-id";

// endpoints

fn build_app<T, E, R, S>(state: Arc<AppState<T, E, R, S>>) -> Router
where
    T: Clone + Send + Sync + 'static,
    T: Call<QueryTuners>,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryChannel>,
    E: Call<QueryChannels>,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<QueryServices>,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
    R: Send + Sync + 'static,
    R: Call<AddRecordingSchedule>,
    R: Call<QueryRecordingRecord>,
    R: Call<QueryRecordingRecorder>,
    R: Call<QueryRecordingRecorders>,
    R: Call<QueryRecordingRecords>,
    R: Call<QueryRecordingSchedule>,
    R: Call<QueryRecordingSchedules>,
    R: Call<RemoveRecordingRecord>,
    R: Call<RemoveRecordingSchedule>,
    R: Call<RemoveRecordingSchedules>,
    R: Call<StartRecording>,
    R: Call<StopRecording>,
    S: Send + Sync + 'static,
    S: Call<CreateTimeshiftLiveStreamSource>,
    S: Call<CreateTimeshiftRecordStreamSource>,
    S: Call<QueryTimeshiftRecord>,
    S: Call<QueryTimeshiftRecords>,
    S: Call<QueryTimeshiftRecorder>,
    S: Call<QueryTimeshiftRecorders>,
{
    let api_routes = build_api(state.clone());

    let mut router = Router::new().nest("/api", api_routes);

    async fn convert_error(err: io::Error) -> Error {
        err.into()
    }

    for (mount_point, mount) in state.config.server.mounts.iter() {
        let path = std::path::Path::new(&mount.path);
        router = if path.is_dir() {
            router.nest_service(
                &mount_point,
                routing::get_service(ServeDir::new(&path)).handle_error(convert_error),
            )
        } else if path.is_file() {
            router.route(
                &mount_point,
                routing::get_service(ServeFile::new(&path)).handle_error(convert_error),
            )
        } else {
            router
        };
    }

    let mut default_headers = HeaderMap::new();
    default_headers.append(SERVER, header_value!(server_name()));
    // Disable HTTP keep-alive.
    default_headers.append(CONNECTION, header_value!("close"));

    router = router
        // Allow access only from local addresses.
        .layer(AccessControlLayer)
        // Append the default headers.
        .layer(DefaultHeadersLayer::new(default_headers))
        // Output tracing logs.
        .layer(TraceLayer::new_for_http());

    router
}

fn build_api<T, E, R, S>(state: Arc<AppState<T, E, R, S>>) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Clone + Send + Sync + 'static,
    T: Call<QueryTuners>,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryChannel>,
    E: Call<QueryChannels>,
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryPrograms>,
    E: Call<QueryService>,
    E: Call<QueryServices>,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
    R: Send + Sync + 'static,
    R: Call<AddRecordingSchedule>,
    R: Call<QueryRecordingRecord>,
    R: Call<QueryRecordingRecorder>,
    R: Call<QueryRecordingRecorders>,
    R: Call<QueryRecordingRecords>,
    R: Call<QueryRecordingSchedule>,
    R: Call<QueryRecordingSchedules>,
    R: Call<RemoveRecordingRecord>,
    R: Call<RemoveRecordingSchedule>,
    R: Call<RemoveRecordingSchedules>,
    R: Call<StartRecording>,
    R: Call<StopRecording>,
    S: Send + Sync + 'static,
    S: Call<CreateTimeshiftLiveStreamSource>,
    S: Call<CreateTimeshiftRecordStreamSource>,
    S: Call<QueryTimeshiftRecord>,
    S: Call<QueryTimeshiftRecords>,
    S: Call<QueryTimeshiftRecorder>,
    S: Call<QueryTimeshiftRecorders>,
{
    let mut default_headers = HeaderMap::new();
    default_headers.append(CACHE_CONTROL, header_value!("no-store"));

    // As described in the `axum` documentation, a request handler registered
    // by `routing::get()` can be also used for HEAD requests.
    //
    // We implement a HEAD request handler for each streaming endpoint so that
    // we don't allocate a tuner for the request.
    let mut router = Router::with_state(state.clone())
        .route("/version", routing::get(version_gh))
        .route("/status", routing::get(status_gh))
        .route("/channels", routing::get(channels_gh))
        .route("/services", routing::get(services_gh))
        .route("/services/:id", routing::get(service_gh))
        .route("/services/:id/logo", routing::get(service_logo_gh))
        .route("/programs", routing::get(programs_gh))
        .route("/programs/:id", routing::get(program_gh))
        .route("/tuners", routing::get(tuners_gh))
        .route(
            "/channels/:channel_type/:channel/stream",
            routing::get(channel_stream_g).head(channel_stream_h),
        )
        .route(
            "/channels/:channel_type/:channel/services/:sid/stream",
            routing::get(channel_service_stream_g).head(channel_service_stream_h),
        )
        .route(
            "/services/:id/stream",
            routing::get(service_stream_g).head(service_stream_h),
        )
        .route(
            "/programs/:id/stream",
            routing::get(program_stream_g).head(program_stream_h),
        )
        .route("/timeshift", routing::get(timeshift_recorders_gh))
        .route("/timeshift/:recorder", routing::get(timeshift_recorder_gh))
        .route(
            "/timeshift/:recorder/records",
            routing::get(timeshift_records_gh),
        )
        .route(
            "/timeshift/:recorder/records/:id",
            routing::get(timeshift_record_gh),
        )
        // The following two endpoints won't allocate any tuner.
        .route(
            "/timeshift/:recorder/stream",
            routing::get(timeshift_stream_gh),
        )
        .route(
            "/timeshift/:recorder/records/:id/stream",
            routing::get(timeshift_record_stream_gh),
        )
        .route("/iptv/playlist", routing::get(iptv_playlist_gh))
        // For compatibility with EPGStation
        .route("/iptv/channel.m3u8", routing::get(iptv_playlist_gh))
        .route("/iptv/epg", routing::get(iptv_epg_gh))
        // For compatibility with Mirakurun
        .route("/iptv/xmltv", routing::get(iptv_xmltv_gh))
        .route("/docs", routing::get(docs_gh))
        // Disable caching.
        .layer(DefaultHeadersLayer::new(default_headers));

    if state.config.recorder.record_dir.is_some() {
        tracing::info!("Enable endpoints for recording");
        router = router
            .route(
                "/recording/schedules",
                routing::get(get_recording_schedules),
            )
            .route(
                "/recording/schedules",
                routing::post(create_recording_schedule),
            )
            .route(
                "/recording/schedules",
                routing::delete(delete_recording_schedules),
            )
            .route(
                "/recording/schedules/:id",
                routing::get(get_recording_schedule),
            )
            .route(
                "/recording/schedules/:id",
                routing::delete(delete_recording_schedule),
            )
            .route(
                "/recording/recorders",
                routing::get(get_recording_recorders),
            )
            .route(
                "/recording/recorders",
                routing::post(create_recording_recorder),
            )
            .route(
                "/recording/recorders/:id",
                routing::get(get_recording_recorder),
            )
            .route(
                "/recording/recorders/:id",
                routing::delete(delete_recording_recorder),
            )
            .route("/recording/records", routing::get(get_recording_records))
            .route("/recording/records/:id", routing::get(get_recording_record))
            .route(
                "/recording/records/:id",
                routing::delete(delete_recording_record),
            )
            .route(
                "/recording/records/:id/stream",
                routing::get(get_recording_record_stream),
            );
    };

    router
}

// Request Handlers
//
// Function names are ended with the following marks:
//
//   * g: Used for GET requests
//   * h: Used for HEAD requests
//

async fn version_gh() -> impl IntoResponse {
    Json(Version {
        current: env!("CARGO_PKG_VERSION"),
        latest: env!("CARGO_PKG_VERSION"), // unsupported
    })
}

async fn status_gh() -> impl IntoResponse {
    Json(Status {})
}

async fn channels_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunChannel>>, Error>
where
    E: Call<QueryChannels>,
{
    state
        .epg
        .call(QueryChannels)
        .await
        .map(Json::from)
        .map_err(Error::from)
}

async fn services_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunService>>, Error>
where
    E: Call<QueryServices>,
{
    Ok(state
        .epg
        .call(QueryServices)
        .await?
        .values()
        .cloned()
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&state.config.resource);
            service
        })
        .collect::<Vec<MirakurunService>>()
        .into())
}

async fn service_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Json<MirakurunService>, Error>
where
    E: Call<QueryService>,
{
    state
        .epg
        .call(QueryService::ByMirakurunServiceId(id))
        .await?
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&state.config.resource);
            Json(service)
        })
}

async fn service_logo_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Response<StaticFileBody>, Error>
where
    E: Call<QueryService>,
{
    let service = state
        .epg
        .call(QueryService::ByMirakurunServiceId(id))
        .await??;

    match state.config.resource.logos.get(&service.triple()) {
        Some(path) => {
            Ok(Response::builder()
                // TODO: The type should be specified in config.yml.
                .header(CONTENT_TYPE, "image/png")
                .body(StaticFileBody::new(path).await?)?)
        }
        None => Err(Error::NoLogoData),
    }
}

async fn programs_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunProgram>>, Error>
where
    E: Call<QueryPrograms>,
    E: Call<QueryServices>,
{
    let services = state.epg.call(QueryServices).await?;
    let mut result = vec![];
    for triple in services.keys() {
        let programs = state
            .epg
            .call(QueryPrograms {
                service_triple: triple.clone(),
            })
            .await?;
        result.reserve(programs.len());
        result.extend(programs.values().cloned().map(MirakurunProgram::from));
    }
    Ok(result.into())
}

async fn program_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
) -> Result<Json<MirakurunProgram>, Error>
where
    E: Call<QueryProgram>,
{
    state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(id))
        .await?
        .map(MirakurunProgram::from)
        .map(Json::from)
}

async fn tuners_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunTuner>>, Error>
where
    T: Call<QueryTuners>,
{
    state
        .tuner_manager
        .call(QueryTuners)
        .await
        .map(Json::from)
        .map_err(Error::from)
}

async fn channel_stream_g<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelPath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Call<QueryChannel>,
{
    let channel = state
        .epg
        .call(QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    let stream = state
        .tuner_manager
        .call(StartStreaming {
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

async fn channel_stream_h<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelPath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<QueryChannel>,
{
    let _channel = state
        .epg
        .call(QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_stream_h(&state.config, &user, &filter_setting)
}

async fn channel_service_stream_g<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelServicePath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Call<QueryChannel>,
{
    let channel = state
        .epg
        .call(QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    do_service_stream(
        &state.config,
        &state.tuner_manager,
        channel,
        path.sid,
        user,
        filter_setting,
    )
    .await
}

async fn channel_service_stream_h<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<ChannelServicePath>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<QueryChannel>,
{
    let _channel = state
        .epg
        .call(QueryChannel {
            channel_type: path.channel_type,
            channel: path.channel,
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_stream_h(&state.config, &user, &filter_setting)
}

async fn service_stream_g<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Call<QueryService>,
{
    let service = state
        .epg
        .call(QueryService::ByMirakurunServiceId(id))
        .await??;

    do_service_stream(
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
async fn service_stream_h<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<QueryService>,
{
    let _service = state
        .epg
        .call(QueryService::ByMirakurunServiceId(id))
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_stream_h(&state.config, &user, &filter_setting)
}

async fn program_stream_g<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
    E: Call<QueryClock>,
    E: Call<RemoveAirtime>,
    E: Call<UpdateAirtime>,
{
    let program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = state
        .epg
        .call(QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let clock = state
        .epg
        .call(QueryClock {
            triple: service.triple(),
        })
        .await??;

    let stream = state
        .tuner_manager
        .call(StartStreaming {
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
    if let Some(max_start_delay) = state.config.recorder.max_start_delay {
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
        &state.config.recorder.track_airtime_command,
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

async fn program_stream_h<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<QueryClock>,
    E: Call<QueryProgram>,
    E: Call<QueryService>,
{
    let _program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(id))
        .await??;

    let service = state
        .epg
        .call(QueryService::ByMirakurunServiceId(id.into()))
        .await??;

    let _clock = state
        .epg
        .call(QueryClock {
            triple: service.triple(),
        })
        .await??;

    // This endpoint returns a positive response even when no tuner is available
    // for streaming at this point.  No one knows whether this request handler
    // will success or not until actually starting streaming.
    do_stream_h(&state.config, &user, &filter_setting)
}

async fn get_recording_schedules<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingSchedule>>, Error>
where
    E: Call<QueryProgram>,
    R: Call<QueryRecordingSchedules>,
{
    let mut results = vec![];
    let schedules = state
        .recording_manager
        .call(QueryRecordingSchedules)
        .await?;
    for schedule in schedules.into_iter() {
        let program = state
            .epg
            .call(QueryProgram::ByMirakurunProgramId(schedule.program_id))
            .await??;
        results.push(WebRecordingSchedule {
            program: program.into(),
            content_path: schedule.content_path.clone(),
            priority: schedule.priority,
            pre_filters: schedule.pre_filters.clone(),
            post_filters: schedule.post_filters.clone(),
            tags: schedule.tags.clone(),
        });
    }

    Ok(Json(results))
}

async fn get_recording_schedule<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<Json<WebRecordingSchedule>, Error>
where
    E: Call<QueryProgram>,
    R: Call<QueryRecordingSchedule>,
{
    let program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    let schedule = state
        .recording_manager
        .call(QueryRecordingSchedule { program_id })
        .await??;
    Ok(Json(WebRecordingSchedule {
        program: program.into(),
        content_path: schedule.content_path.clone(),
        priority: schedule.priority,
        pre_filters: schedule.pre_filters.clone(),
        post_filters: schedule.post_filters.clone(),
        tags: schedule.tags.clone(),
    }))
}

async fn create_recording_schedule<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<(StatusCode, Json<WebRecordingSchedule>), Error>
where
    E: Call<QueryProgram>,
    R: Call<AddRecordingSchedule>,
    R: Call<QueryRecordingSchedule>,
{
    let program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(input.program_id))
        .await??;
    let schedule = Schedule {
        program_id: input.program_id,
        content_path: input.content_path,
        priority: input.priority,
        pre_filters: input.pre_filters,
        post_filters: input.post_filters,
        tags: input.tags,
        start_at: program.start_at,
    };
    let schedule = state
        .recording_manager
        .call(AddRecordingSchedule { schedule })
        .await??;
    Ok((
        StatusCode::CREATED,
        Json(WebRecordingSchedule {
            program: program.into(),
            content_path: schedule.content_path.clone(),
            priority: schedule.priority,
            pre_filters: schedule.pre_filters.clone(),
            post_filters: schedule.post_filters.clone(),
            tags: schedule.tags.clone(),
        }),
    ))
}

async fn delete_recording_schedule<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<(), Error>
where
    R: Call<RemoveRecordingSchedule>,
{
    state
        .recording_manager
        .call(RemoveRecordingSchedule { program_id })
        .await??;
    Ok(())
}

async fn delete_recording_schedules<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<RemoveRecordingSchedules>,
{
    let target = match query.get("target") {
        Some(tag) => crate::recording::RemoveTarget::Tag(tag.clone()),
        None => crate::recording::RemoveTarget::All,
    };
    state
        .recording_manager
        .call(RemoveRecordingSchedules { target })
        .await?;
    Ok(())
}

async fn get_recording_recorders<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingRecorder>>, Error>
where
    E: Call<QueryProgram>,
    R: Call<QueryRecordingRecorders>,
{
    let mut results = vec![];
    let recorders = state
        .recording_manager
        .call(QueryRecordingRecorders)
        .await?;
    for recorder in recorders.into_iter() {
        let program = state
            .epg
            .call(QueryProgram::ByMirakurunProgramId(
                recorder.schedule.program_id,
            ))
            .await??;
        results.push(WebRecordingRecorder {
            program: program.into(),
            content_path: recorder.schedule.content_path.clone(),
            priority: recorder.schedule.priority,
            pipeline: recorder
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            tags: recorder.schedule.tags.clone(),
            start_time: recorder.start_time,
        });
    }
    Ok(Json(results))
}

async fn get_recording_recorder<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<Json<WebRecordingRecorder>, Error>
where
    E: Call<QueryProgram>,
    R: Call<QueryRecordingRecorder>,
{
    let program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(program_id))
        .await??;
    let recorder = state
        .recording_manager
        .call(QueryRecordingRecorder { program_id })
        .await??;
    Ok(Json(WebRecordingRecorder {
        program: program.into(),
        content_path: recorder.schedule.content_path.clone(),
        priority: recorder.schedule.priority,
        pipeline: recorder
            .pipeline
            .into_iter()
            .map(WebProcessModel::from)
            .collect(),
        tags: recorder.schedule.tags.clone(),
        start_time: recorder.start_time,
    }))
}

async fn create_recording_recorder<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Json(input): Json<WebRecordingScheduleInput>,
) -> Result<(StatusCode, Json<WebRecordingRecorder>), Error>
where
    E: Call<QueryProgram>,
    R: Call<StartRecording>,
{
    let program = state
        .epg
        .call(QueryProgram::ByMirakurunProgramId(input.program_id))
        .await??;
    let schedule = Arc::new(Schedule {
        program_id: input.program_id,
        content_path: input.content_path,
        priority: input.priority,
        pre_filters: input.pre_filters,
        post_filters: input.post_filters,
        tags: input.tags,
        start_at: program.start_at,
    });
    let recorder = state
        .recording_manager
        .call(StartRecording { schedule })
        .await??;
    Ok((
        StatusCode::CREATED,
        Json(WebRecordingRecorder {
            program: program.into(),
            content_path: recorder.schedule.content_path.clone(),
            priority: recorder.schedule.priority,
            pipeline: recorder
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            tags: recorder.schedule.tags.clone(),
            start_time: recorder.start_time,
        }),
    ))
}

async fn delete_recording_recorder<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(program_id): Path<MirakurunProgramId>,
) -> Result<(), Error>
where
    R: Call<StopRecording>,
{
    state
        .recording_manager
        .call(StopRecording { program_id })
        .await??;
    Ok(())
}

async fn get_recording_records<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebRecordingRecord>>, Error>
where
    R: Call<QueryRecordingRecords>,
{
    let records: Vec<WebRecordingRecord> = state
        .recording_manager
        .call(QueryRecordingRecords)
        .await??
        .into_iter()
        .map(WebRecordingRecord::from)
        .collect();
    Ok(Json(records))
}

async fn get_recording_record<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
) -> Result<Json<WebRecordingRecord>, Error>
where
    R: Call<QueryRecordingRecord>,
{
    state
        .recording_manager
        .call(QueryRecordingRecord { id })
        .await?
        .map(WebRecordingRecord::from)
        .map(Json::from)
}

async fn delete_recording_record<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<(), Error>
where
    R: Call<RemoveRecordingRecord>,
{
    let remove_content = match query.get("content") {
        Some(content) if content == "remove" => true,
        _ => false,
    };
    state
        .recording_manager
        .call(RemoveRecordingRecord { id, remove_content })
        .await?
}

async fn get_recording_record_stream<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<String>,
    ranges: Option<TypedHeader<axum::headers::Range>>,
) -> Result<Response, Error>
where
    R: Call<QueryRecordingRecord>,
{
    // Use only the first start position for the seek support.
    let start = ranges
        .map(|TypedHeader(ranges)| {
            ranges
                .iter()
                .next()
                .map(|(start, _)| match start {
                    Bound::Included(n) => Some(n),
                    Bound::Excluded(n) => Some(n + 1),
                    _ => None,
                })
                .flatten()
        })
        .flatten()
        .unwrap_or(0);

    let record = state
        .recording_manager
        .call(QueryRecordingRecord { id })
        .await??;

    let mut file = tokio::fs::File::open(record.content_path).await?;
    if start > 0 {
        file.seek(SeekFrom::Start(start)).await?;
    }
    let size = file.metadata().await?.len();

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, header_value!(record.content_type));
    headers.insert(ACCEPT_RANGES, header_value!("bytes"));
    headers.insert(CONTENT_RANGE, header_value!(format!("{}-", start)));

    let stream = ReaderStream::new(file);
    let body = StreamBody::new(stream);
    let body = SeekableStreamBody::new(body, size - start);

    if start > 0 {
        Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
    } else {
        Ok((headers, body).into_response())
    }
}

async fn timeshift_recorders_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<WebTimeshiftRecorder>>, Error>
where
    S: Call<QueryTimeshiftRecorders>,
{
    state
        .timeshift_manager
        .call(QueryTimeshiftRecorders)
        .await?
        .map(|recorders| {
            recorders
                .into_iter()
                .map(WebTimeshiftRecorder::from)
                .collect::<Vec<WebTimeshiftRecorder>>()
        })
        .map(Json::from)
}

async fn timeshift_recorder_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder): Path<String>,
) -> Result<Json<WebTimeshiftRecorder>, Error>
where
    S: Call<QueryTimeshiftRecorder>,
{
    let msg = QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecorder::from)
        .map(Json::from)
}

async fn timeshift_records_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder): Path<String>,
) -> Result<Json<Vec<WebTimeshiftRecord>>, Error>
where
    S: Call<QueryTimeshiftRecords>,
{
    let msg = QueryTimeshiftRecords {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(|records| {
            records
                .into_iter()
                .map(WebTimeshiftRecord::from)
                .collect::<Vec<WebTimeshiftRecord>>()
        })
        .map(Json::from)
}

async fn timeshift_record_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<TimeshiftRecordPath>,
) -> Result<Json<WebTimeshiftRecord>, Error>
where
    S: Call<QueryTimeshiftRecord>,
{
    let msg = QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder),
        record_id: path.id,
    };
    state
        .timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecord::from)
        .map(Json::from)
}

async fn timeshift_stream_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(recorder_id): Path<String>,
    record_id: Option<Query<TimeshiftRecordId>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    S: Call<CreateTimeshiftLiveStreamSource>,
    S: Call<QueryTimeshiftRecorder>,
{
    let msg = QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
    };
    let recorder = state.timeshift_manager.call(msg).await??;

    let msg = CreateTimeshiftLiveStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
        record_id: record_id.map(|Query(id)| id),
    };
    let src = state.timeshift_manager.call(msg).await??;

    let (stream, stop_trigger) = src.create_stream().await?;

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &recorder.service.channel.name)
        .insert("channel_type", &recorder.service.channel.channel_type)?
        .insert_str("channel", &recorder.service.channel.channel)
        .insert("sid", &recorder.service.sid.value())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&state.config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
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

async fn timeshift_record_stream_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(path): Path<TimeshiftRecordPath>,
    ranges: Option<TypedHeader<axum::headers::Range>>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
) -> Result<Response, Error>
where
    S: Call<CreateTimeshiftRecordStreamSource>,
    S: Call<QueryTimeshiftRecord>,
    S: Call<QueryTimeshiftRecorder>,
{
    let msg = QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
    };
    let recorder = state.timeshift_manager.call(msg).await??;

    let msg = QueryTimeshiftRecord {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id.clone(),
    };
    let record = state.timeshift_manager.call(msg).await??;

    let start_pos = if let Some(TypedHeader(ranges)) = ranges {
        ranges
            .iter()
            .next()
            .map(|(start, _)| match start {
                Bound::Included(n) => Some(n),
                Bound::Excluded(n) => Some(n + 1),
                _ => None,
            })
            .flatten()
    } else {
        None
    };

    let msg = CreateTimeshiftRecordStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(path.recorder.clone()),
        record_id: path.id.clone(),
        start_pos,
    };
    let src = state.timeshift_manager.call(msg).await??;

    // We assume that pre-filters don't change TS packets.
    let seekable = filter_setting.post_filters.is_empty();

    let (stream, stop_trigger) = src.create_stream(seekable).await?;

    let video_tags: Vec<u8> = record
        .program
        .video
        .iter()
        .map(|video| video.component_tag)
        .collect();

    let audio_tags: Vec<u8> = record
        .program
        .audios
        .values()
        .map(|audio| audio.component_tag)
        .collect();

    let duration = record.end_time - record.start_time;

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &recorder.service.channel.name)
        .insert("channel_type", &recorder.service.channel.channel_type)?
        .insert_str("channel", &recorder.service.channel.channel)
        .insert("sid", &recorder.service.sid.value())?
        .insert("eid", &record.program.quad.eid())?
        .insert("video_tags", &video_tags)?
        .insert("audio_tags", &audio_tags)?
        .insert("id", &record.id)?
        .insert("duration", &duration.num_seconds())?
        .insert("size", &record.size)?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&state.config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
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

async fn iptv_playlist_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<QueryServices>,
{
    do_iptv_playlist(&state.config, &state.epg, &host, filter_setting).await
}

async fn do_iptv_playlist<E>(
    config: &Config,
    epg: &E,
    host: &str,
    mut filter_setting: FilterSetting,
) -> Result<Response<String>, Error>
where
    E: Call<QueryServices>,
{
    const INITIAL_BUFSIZE: usize = 8 * 1024; // 8KB

    filter_setting.decode = true; // always decode
    let query = serde_qs::to_string(&filter_setting).expect("Never fails");

    let services = epg.call(QueryServices).await?;

    // TODO: URL scheme

    let mut buf = String::with_capacity(INITIAL_BUFSIZE);
    write!(buf, "#EXTM3U\n")?;
    for sv in services.values() {
        let id = MirakurunServiceId::from(sv.triple());
        let logo_url = format!("http://{}/api/services/{}/logo", host, id.value());
        // The following format is compatible with EPGStation.
        // See API docs for the `/api/channel.m3u8` endpoint.
        //
        // U+3000 (IDEOGRAPHIC SPACE) at the end of each line is required for
        // avoiding garbled characters in `ＮＨＫＢＳプレミアム`.  Kodi or PVR
        // IPTV Simple Client seems to treat it as Latin-1 when removing U+3000.
        match sv.service_type {
            0x01 | 0xA1 | 0xA5 | 0xAD => {
                // video
                // Special optimization for IPTV Simple Client.
                //
                // Explicitly specifying the mime type of each channel avoids
                // redundant requests.
                match determine_stream_content_type(&config, &filter_setting) {
                    "video/MP2T" => {
                        // The mime type MUST be `video/mp2t`.
                        // See StreamUtils::GetStreamType() in
                        // src/iptvsimple/utilities/StreamUtils.cpp in
                        // kodi-pvr/pvr.iptvsimple.
                        write!(buf, "#KODIPROP:mimetype=video/mp2t\n")?;
                    }
                    mimetype => {
                        write!(buf, "#KODIPROP:mimetype={}\n", mimetype)?;
                    }
                }
                write!(buf, r#"#EXTINF:-1 tvg-id="{}""#, id.value())?;
                if config.resource.logos.contains_key(&sv.triple()) {
                    write!(buf, r#" tvg-logo="{}""#, logo_url)?;
                }
                write!(
                    buf,
                    r#" group-title="{}", {}　"#,
                    sv.channel.channel_type, sv.name
                )?;
            }
            0x02 | 0xA2 | 0xA6 => {
                // audio
                write!(buf, r#"#EXTINF:-1 tvg-id="{}""#, id.value())?;
                if config.resource.logos.contains_key(&sv.triple()) {
                    write!(buf, r#" tvg-logo="{}""#, logo_url)?;
                }
                write!(
                    buf,
                    r#" group-title="{}-Radio" radio=true, {}　"#,
                    sv.channel.channel_type, sv.name
                )?;
            }
            _ => unreachable!(),
        }
        write!(
            buf,
            "\nhttp://{}/api/services/{}/stream?{}\n",
            host,
            id.value(),
            query
        )?;
    }

    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/x-mpegurl; charset=UTF-8")
        .body(buf)?)
}

async fn iptv_epg_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
    Query(query): Query<IptvEpgQuery>,
) -> impl IntoResponse
where
    E: Call<QueryPrograms>,
    E: Call<QueryServices>,
{
    do_iptv_epg(&state.config, &state.string_table, &state.epg, &host, query).await
}

// For compatibility with Mirakurun
async fn iptv_xmltv_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
) -> impl IntoResponse
where
    E: Call<QueryPrograms>,
    E: Call<QueryServices>,
{
    // Mirakurun doesn't support the days query parameter and returns all
    // programs.
    let query = IptvEpgQuery { days: 10 };
    do_iptv_epg(&state.config, &state.string_table, &state.epg, &host, query).await
}

async fn do_iptv_epg<E>(
    config: &Config,
    string_table: &StringTable,
    epg: &E,
    host: &str,
    query: IptvEpgQuery,
) -> Result<Response<String>, Error>
where
    E: Call<QueryPrograms>,
    E: Call<QueryServices>,
{
    const INITIAL_BUFSIZE: usize = 8 * 1024 * 1024; // 8MB
    const DATETIME_FORMAT: &'static str = "%Y%m%d%H%M%S %z";

    let end_after = Jst::midnight();
    let start_before = end_after + chrono::Duration::days(query.days as i64);

    let services = epg.call(QueryServices).await?;

    // TODO: URL scheme

    let mut buf = String::with_capacity(INITIAL_BUFSIZE);
    write!(buf, r#"<?xml version="1.0" encoding="UTF-8" ?>"#)?;
    write!(buf, r#"<!DOCTYPE tv SYSTEM "xmltv.dtd">"#)?;
    write!(
        buf,
        r#"<tv generator-info-name="{}">"#,
        escape(&server_name())
    )?;
    for sv in services.values() {
        let id = MirakurunServiceId::from(sv.triple());
        let logo_url = format!("http://{}/api/services/{}/logo", host, id.value());
        write!(buf, r#"<channel id="{}">"#, id.value())?;
        write!(
            buf,
            r#"<display-name lang="ja">{}</display-name>"#,
            escape(&sv.name)
        )?;
        if config.resource.logos.contains_key(&sv.triple()) {
            write!(buf, r#"<icon src="{}" />"#, logo_url)?;
        }
        write!(buf, r#"</channel>"#)?;
    }
    for triple in services.keys() {
        let programs = epg
            .call(QueryPrograms {
                service_triple: triple.clone(),
            })
            .await?;
        for pg in programs
            .values()
            .filter(|pg| pg.name.is_some())
            .filter(|pg| pg.start_at < start_before && pg.end_at() > end_after)
        {
            let id = MirakurunServiceId::from(pg.quad);
            write!(
                buf,
                r#"<programme start="{}" stop="{}" channel="{}">"#,
                pg.start_at.format(DATETIME_FORMAT),
                pg.end_at().format(DATETIME_FORMAT),
                id.value()
            )?;
            if let Some(name) = pg.name.as_ref() {
                write!(buf, r#"<title lang="ja">{}</title>"#, escape(&name))?;
            }
            if let Some(desc) = pg.description.as_ref() {
                write!(buf, r#"<desc lang="ja">"#)?;
                write!(buf, "{}", escape(&desc))?;
                if let Some(extended) = pg.extended.as_ref() {
                    for (key, value) in extended.iter() {
                        if key.is_empty() {
                            write!(buf, "{}", escape(&value))?;
                        } else {
                            write!(buf, "\n{}\n{}", escape(&key), escape(&value))?;
                        }
                    }
                }
                write!(buf, r#"</desc>"#)?;
            }
            if let Some(genres) = pg.genres.as_ref() {
                for genre in genres.iter() {
                    let genre_str = &string_table.genres[genre.lv1 as usize].genre;
                    let subgenre_str =
                        &string_table.genres[genre.lv1 as usize].subgenres[genre.lv2 as usize];
                    if subgenre_str.is_empty() {
                        write!(
                            buf,
                            r#"<category lang="ja">{}</category>"#,
                            escape(&genre_str)
                        )?;
                    } else {
                        write!(
                            buf,
                            r#"<category lang="ja">{} / {}</category>"#,
                            escape(&genre_str),
                            escape(&subgenre_str)
                        )?;
                    }
                }
            }
            write!(buf, r#"</programme>"#)?;
        }
    }
    write!(buf, r#"</tv>"#)?;

    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/xml; charset=UTF-8")
        .body(buf)?)
}

async fn docs_gh<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Response<StaticFileBody>, Error> {
    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/json")
        .body(StaticFileBody::new(&state.config.mirakurun.openapi_json).await?)?)
}

async fn do_service_stream<T>(
    config: &Config,
    tuner_manager: &T,
    channel: EpgChannel,
    sid: ServiceId,
    user: TunerUser,
    filter_setting: FilterSetting,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<StartStreaming>,
    T: Into<Emitter<StopStreaming>>,
{
    let stream = tuner_manager
        .call(StartStreaming {
            channel: channel.clone(),
            user: user.clone(),
        })
        .await??;

    // stop_trigger must be created here in order to stop streaming when an
    // error occurs.
    let stop_trigger = TunerStreamStopTrigger::new(stream.id(), tuner_manager.clone().into());

    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &channel.name)
        .insert("channel_type", &channel.channel_type)?
        .insert_str("channel", &channel.channel)
        .insert("sid", &sid.value())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data);
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    if !stream.is_decoded() && filter_setting.decode {
        builder.add_decode_filter(&config.filters.decode_filter)?;
    }
    builder.add_service_filter(&config.filters.service_filter)?;
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    let (filters, content_type) = builder.build();

    streaming(&config, user, stream, filters, content_type, stop_trigger).await
}

async fn streaming<T, S, D>(
    config: &Config,
    user: TunerUser,
    stream: MpegTsStream<T, S>,
    filters: Vec<String>,
    content_type: String,
    stop_triggers: D,
) -> Result<Response, Error>
where
    T: fmt::Display + Clone + Send + Unpin + 'static,
    S: Stream<Item = io::Result<Bytes>> + Send + Unpin + 'static,
    D: Send + Unpin + 'static,
{
    let range = stream.range();
    if filters.is_empty() {
        do_streaming(
            user,
            stream,
            content_type,
            range,
            stop_triggers,
            config.server.stream_time_limit,
        )
        .await
    } else {
        tracing::debug!("Streaming with filters: {:?}", filters);

        let mut pipeline = spawn_pipeline(filters, stream.id())?;

        let (input, output) = pipeline.take_endpoints()?;

        let stream_id = stream.id();
        tokio::spawn(async move {
            let _ = stream.pipe(input).await;
        });

        // Use an MPSC channel as a buffer.
        //
        // The command pipeline often breaks when the client stops reading for a
        // few seconds.
        let mut stream = ReaderStream::with_capacity(output, config.server.stream_chunk_size);
        let (sender, receiver) = mpsc::channel(config.server.stream_max_chunks);
        tokio::spawn(async move {
            while let Some(result) = stream.next().await {
                if let Ok(chunk) = result {
                    tracing::trace!(
                        "{}: Received a filtered chunk of {} bytes",
                        stream_id,
                        chunk.len()
                    );
                    // The task yields if the buffer is full.
                    if let Err(_) = sender.send(Ok(chunk)).await {
                        tracing::debug!("{}: Disconnected by client", stream_id);
                        break;
                    }
                } else {
                    tracing::error!("{}: Error, stop streaming", stream_id);
                    break;
                }

                // Always yield for sending the chunk to the client quickly.
                //
                // The async task never yields voluntarily and can starve other
                // tasks waiting on the same executor.  For avoiding the
                // starvation, the task has to yields within a short term.
                //
                // Theoretically, one 32 KiB chunk comes every 10 ms.  This
                // period is a long enough time in the CPU time point of view.
                // Therefore, the async task simply yields at the end of every
                // iteration.
                tokio::task::yield_now().await;
            }

            drop(pipeline);
        });

        do_streaming(
            user,
            ReceiverStream::new(receiver),
            content_type,
            range,
            stop_triggers,
            config.server.stream_time_limit,
        )
        .await
    }
}

async fn do_streaming<S, D>(
    user: TunerUser,
    stream: S,
    content_type: String,
    range: Option<MpegTsStreamRange>,
    stop_trigger: D,
    time_limit: u64,
) -> Result<Response, Error>
where
    S: Stream<Item = io::Result<Bytes>> + Send + Unpin + 'static,
    D: Send + Unpin + 'static,
{
    let stream = MpegTsStreamTerminator::new(stream, stop_trigger);

    // No data is sent to the client until the first TS packet comes from the
    // streaming pipeline.
    let mut peekable = stream.peekable();
    let fut = Pin::new(&mut peekable).peek();
    match tokio::time::timeout(std::time::Duration::from_millis(time_limit), fut).await {
        Ok(None) => {
            // No packets come from the pipeline, maybe the program has been
            // canceled.
            Err(Error::ProgramNotFound)
        }
        Err(_) => Err(Error::StreamingTimedOut),
        Ok(_) => {
            // Send the response headers and start streaming.
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, header_value!(content_type));
            headers.insert(
                X_MIRAKURUN_TUNER_USER_ID,
                header_value!(&user.get_mirakurun_model().id),
            );
            let body = StreamBody::new(peekable);
            if let Some(range) = range {
                headers.insert(ACCEPT_RANGES, header_value!("bytes"));
                headers.insert(CONTENT_RANGE, header_value!(range.make_content_range()));
                let body = SeekableStreamBody::new(body, range.bytes());
                if range.is_partial() {
                    Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
                } else {
                    Ok((headers, body).into_response())
                }
            } else {
                headers.insert(ACCEPT_RANGES, header_value!("none"));
                Ok((headers, body).into_response())
            }
        }
    }
}

fn do_stream_h(
    config: &Config,
    user: &TunerUser,
    filter_setting: &FilterSetting,
) -> Result<Response, Error> {
    let content_type = determine_stream_content_type(&config, &filter_setting);

    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_RANGES, header_value!("none"));
    headers.insert(CACHE_CONTROL, header_value!("no-store"));
    headers.insert(CONTENT_TYPE, header_value!(content_type));
    headers.insert(
        X_MIRAKURUN_TUNER_USER_ID,
        header_value!(user.get_mirakurun_model().id),
    );
    // axum doesn't add the following header even thought we use a StreamBody.
    headers.insert(TRANSFER_ENCODING, header_value!("chunked"));

    // It's a dirt hack...
    //
    // Create an empty stream in order to prevent a "content-length: 0" header
    // from being added.
    let body = StreamBody::new(futures::stream::empty::<Result<Bytes, Error>>());

    Ok((headers, body).into_response())
}

fn determine_stream_content_type<'a>(
    config: &'a Config,
    filter_setting: &FilterSetting,
) -> &'a str {
    let mut result = "video/MP2T";
    for name in filter_setting.post_filters.iter() {
        if let Some(config) = config.post_filters.get(name) {
            if let Some(ref content_type) = config.content_type {
                result = content_type;
            }
        }
    }
    result
}

fn server_name() -> String {
    format!("mirakc/{}", env!("CARGO_PKG_VERSION"))
}

// state

struct AppState<T, E, R, S> {
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
}

// extractors

#[derive(Deserialize)]
struct ChannelPath {
    channel_type: ChannelType,
    channel: String,
}

#[derive(Deserialize)]
struct ChannelServicePath {
    channel_type: ChannelType,
    channel: String,
    sid: ServiceId,
}

#[derive(Deserialize)]
struct TimeshiftRecordPath {
    recorder: String,
    id: TimeshiftRecordId,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct FilterSetting {
    #[serde(default = "FilterSetting::default_decode")]
    #[serde(deserialize_with = "FilterSetting::deserialize_stream_decode_query")]
    decode: bool, // default: true
    #[serde(default)]
    pre_filters: Vec<String>, // default: empty
    #[serde(default)]
    post_filters: Vec<String>, // default: empty
}

impl FilterSetting {
    fn default_decode() -> bool {
        true
    }

    fn deserialize_stream_decode_query<'de, D>(deserializer: D) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "0" || s == "false" {
            return Ok(false);
        }
        if s == "1" || s == "true" {
            return Ok(true);
        }
        Err(serde::de::Error::custom(
            "The value of the decode query must be 0, 1, false or true",
        ))
    }
}

#[derive(Deserialize)]
struct IptvEpgQuery {
    #[serde(default = "IptvEpgQuery::default_days")]
    days: u8,
}

impl IptvEpgQuery {
    fn default_days() -> u8 {
        3
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for TunerUser
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::ConnectInfo;

        fn ms_since_unix_epoch() -> u128 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|dur| dur.as_millis())
                .unwrap_or(0)
        }

        let id = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.to_string())
            .unwrap_or_else(|| format!("unix:{}", ms_since_unix_epoch()));

        let agent = parts
            .headers
            .get_all(USER_AGENT)
            .iter()
            .last()
            .map(|value| value.to_str().ok().map_or(String::new(), |s| s.to_string()));

        let info = TunerUserInfo::Web { id, agent };

        let priority = parts
            .headers
            .get_all(X_MIRAKURUN_TUNER_USER_ID)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.parse::<i32>().ok())
            .max()
            .map(|value| value.max(0))
            .map(TunerUserPriority::from)
            .unwrap_or_default();

        Ok(TunerUser { info, priority })
    }
}

// models

#[derive(Serialize)]
struct Version {
    current: &'static str,
    latest: &'static str,
}

#[derive(Serialize)]
struct Status {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRecordingSchedule {
    program: MirakurunProgram,
    content_path: PathBuf,
    priority: i32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pre_filters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    post_filters: Vec<String>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    tags: HashSet<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRecordingScheduleInput {
    program_id: MirakurunProgramId,
    content_path: PathBuf,
    #[serde(default)]
    priority: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pre_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    post_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    tags: HashSet<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRecordingRecorder {
    program: MirakurunProgram,
    content_path: PathBuf,
    priority: i32,
    pipeline: Vec<WebProcessModel>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    tags: HashSet<String>,
    #[serde(with = "serde_jst")]
    start_time: DateTime<Jst>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRecordingRecord {
    id: String,
    program: MirakurunProgram,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pre_filters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    post_filters: Vec<String>,
    #[serde(skip_serializing_if = "HashSet::is_empty")]
    tags: HashSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_size: Option<u64>,
}

impl From<Record> for WebRecordingRecord {
    fn from(record: Record) -> Self {
        let content_size = record
            .content_path
            .metadata()
            .ok()
            .map(|metadata| metadata.len());
        let (content_path, content_type) = if record.content_path.exists() {
            (Some(record.content_path), Some(record.content_type))
        } else {
            (None, None)
        };
        WebRecordingRecord {
            id: record.program.record_id(),
            program: record.program.into(),
            pre_filters: record.pre_filters,
            post_filters: record.post_filters,
            tags: record.tags,
            content_path,
            content_type,
            content_size,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTimeshiftRecorder {
    name: String,
    service: MirakurunService,
    #[serde(with = "serde_jst")]
    start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    duration: Duration,
    pipeline: Vec<WebProcessModel>,
    recording: bool,
}

impl From<TimeshiftRecorderModel> for WebTimeshiftRecorder {
    fn from(model: TimeshiftRecorderModel) -> Self {
        Self {
            name: model.name,
            service: model.service.into(),
            start_time: model.start_time.clone(),
            duration: model.end_time - model.start_time,
            pipeline: model
                .pipeline
                .into_iter()
                .map(WebProcessModel::from)
                .collect(),
            recording: model.recording,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebProcessModel {
    command: String,
    pid: Option<u32>,
}

impl From<CommandPipelineProcessModel> for WebProcessModel {
    fn from(model: CommandPipelineProcessModel) -> Self {
        Self {
            command: model.command,
            pid: model.pid,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTimeshiftRecord {
    id: TimeshiftRecordId,
    program: MirakurunProgram,
    #[serde(with = "serde_jst")]
    start_time: DateTime<Jst>,
    #[serde(with = "serde_duration_in_millis")]
    duration: Duration,
    size: u64,
    recording: bool,
}

impl From<TimeshiftRecordModel> for WebTimeshiftRecord {
    fn from(model: TimeshiftRecordModel) -> Self {
        Self {
            id: model.id,
            program: model.program.into(),
            start_time: model.start_time.clone(),
            duration: model.end_time - model.start_time,
            size: model.size,
            recording: model.recording,
        }
    }
}

// errors

#[derive(Serialize)]
struct ErrorBody {
    code: u16,
    reason: Option<&'static str>,
    errors: Vec<u8>,
}

macro_rules! error_response {
    ($status_code:expr) => {
        (
            $status_code,
            Json(ErrorBody {
                code: $status_code.as_u16(),
                reason: None,
                errors: vec![],
            }),
        )
            .into_response()
    };
    ($status_code:expr, $reason:literal) => {
        (
            $status_code,
            Json(ErrorBody {
                code: $status_code.as_u16(),
                reason: Some($reason),
                errors: vec![],
            }),
        )
            .into_response()
    };
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self {
            Error::StreamingTimedOut => error_response!(StatusCode::REQUEST_TIMEOUT),
            Error::TunerUnavailable => error_response!(StatusCode::NOT_FOUND),
            Error::ChannelNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ServiceNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ProgramNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::RecordNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ScheduleNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::RecorderNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::OutOfRange => error_response!(StatusCode::RANGE_NOT_SATISFIABLE),
            Error::NoContent => error_response!(StatusCode::NO_CONTENT),
            Error::NoLogoData => {
                error_response!(StatusCode::SERVICE_UNAVAILABLE, "Logo Data Unavailable")
            }
            Error::AccessDenied => error_response!(StatusCode::FORBIDDEN),
            Error::QuerystringError(_) => error_response!(StatusCode::BAD_REQUEST),
            _ => error_response!(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}
