use std::net::SocketAddr;
use std::sync::Arc;

use actlet::prelude::*;
use axum::extract::FromRef;
use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONNECTION;
use axum::http::header::SERVER;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::routing;
use axum::Router;
use axum::Server;
use futures::future::join_all;
use futures::future::FutureExt;
use tower_http::trace::TraceLayer;
use utoipa_swagger_ui::SwaggerUi;

use crate::config::Config;
use crate::error::Error;
use crate::string_table::StringTable;
use crate::web::access_control::AccessControlLayer;
use crate::web::default_headers::DefaultHeadersLayer;
use crate::web::uds::UdsListener;

// macros

macro_rules! header_value {
    ($v:literal) => {
        HeaderValue::from_static($v)
    };
    ($v:expr) => {
        HeaderValue::from_str(&$v).unwrap()
    };
}

mod access_control;
mod api;
mod body;
mod default_headers;
mod error;
mod escape;
mod mount;
mod qs;
mod sse;
mod uds;

#[cfg(test)]
mod tests;

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
    T: Call<crate::tuner::QueryTuners>,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryChannel>,
    E: Call<crate::epg::QueryChannels>,
    E: Call<crate::epg::QueryClock>,
    E: Call<crate::epg::QueryProgram>,
    E: Call<crate::epg::QueryPrograms>,
    E: Call<crate::epg::QueryService>,
    E: Call<crate::epg::QueryServices>,
    E: Call<crate::epg::RegisterEmitter>,
    E: Call<crate::epg::RemoveAirtime>,
    E: Call<crate::epg::UpdateAirtime>,
    R: Clone + Send + Sync + 'static,
    R: Call<crate::recording::AddRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingRecorder>,
    R: Call<crate::recording::QueryRecordingRecorders>,
    R: Call<crate::recording::QueryRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingSchedules>,
    R: Call<crate::recording::RegisterEmitter>,
    R: Call<crate::recording::RemoveRecordingSchedule>,
    R: Call<crate::recording::RemoveRecordingSchedules>,
    R: Call<crate::recording::StartRecording>,
    S: Clone + Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
{
    // Disable caching.
    let mut default_headers = HeaderMap::new();
    default_headers.append(CACHE_CONTROL, header_value!("no-store"));

    let app = build_app(&config)
        .layer(DefaultHeadersLayer::new(default_headers))
        .with_state(Arc::new(AppState {
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

// headers

const X_MIRAKURUN_TUNER_USER_ID: &'static str = "x-mirakurun-tuner-user-id";

// endpoints

fn build_app<T, E, R, S>(config: &Config) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::QueryTuners>,
    T: Call<crate::tuner::StartStreaming>,
    T: Into<Emitter<crate::tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryChannel>,
    E: Call<crate::epg::QueryChannels>,
    E: Call<crate::epg::QueryClock>,
    E: Call<crate::epg::QueryProgram>,
    E: Call<crate::epg::QueryPrograms>,
    E: Call<crate::epg::QueryService>,
    E: Call<crate::epg::QueryServices>,
    E: Call<crate::epg::RegisterEmitter>,
    E: Call<crate::epg::RemoveAirtime>,
    E: Call<crate::epg::UpdateAirtime>,
    R: Clone + Send + Sync + 'static,
    R: Call<crate::recording::AddRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingRecorder>,
    R: Call<crate::recording::QueryRecordingRecorders>,
    R: Call<crate::recording::QueryRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingSchedules>,
    R: Call<crate::recording::RegisterEmitter>,
    R: Call<crate::recording::RemoveRecordingSchedule>,
    R: Call<crate::recording::RemoveRecordingSchedules>,
    R: Call<crate::recording::StartRecording>,
    S: Clone + Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
{
    let mut router = Router::new()
        .nest("/api", api::build_api(config))
        .merge(SwaggerUi::new("/api/debug").url("/api/docs", api::Docs::generate(config)));

    router = router.route("/events", routing::get(sse::events));
    router = mount::mount_entries(config, router);

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

// state and extractors

struct AppState<T, E, R, S> {
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
}

struct ConfigExtractor(Arc<Config>);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for ConfigExtractor {
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.config.clone())
    }
}

struct StringTableExtractor(Arc<StringTable>);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for StringTableExtractor {
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.string_table.clone())
    }
}

struct TunerManagerExtractor<T>(T);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for TunerManagerExtractor<T>
where
    T: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.tuner_manager.clone())
    }
}

struct EpgExtractor<E>(E);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for EpgExtractor<E>
where
    E: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.epg.clone())
    }
}

struct RecordingManagerExtractor<R>(R);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for RecordingManagerExtractor<R>
where
    R: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.recording_manager.clone())
    }
}

struct TimeshiftManagerExtractor<S>(S);

impl<T, E, R, S> FromRef<Arc<AppState<T, E, R, S>>> for TimeshiftManagerExtractor<S>
where
    S: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S>>) -> Self {
        Self(state.timeshift_manager.clone())
    }
}

// helpers

fn server_name() -> String {
    format!("mirakc/{}", env!("CARGO_PKG_VERSION"))
}
