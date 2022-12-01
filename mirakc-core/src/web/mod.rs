use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use actlet::*;
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
use tower_http::services::ServeDir;
use tower_http::services::ServeFile;
use tower_http::trace::TraceLayer;
use utoipa_swagger_ui::SwaggerUi;

use crate::config::Config;
use crate::epg;
use crate::error::Error;
use crate::recording;
use crate::string_table::StringTable;
use crate::timeshift;
use crate::tuner;
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
mod qs;
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
    T: Call<tuner::QueryTuners>,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryChannel>,
    E: Call<epg::QueryChannels>,
    E: Call<epg::QueryClock>,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RemoveAirtime>,
    E: Call<epg::UpdateAirtime>,
    R: Send + Sync + 'static,
    R: Call<recording::AddRecordingSchedule>,
    R: Call<recording::QueryRecordingRecord>,
    R: Call<recording::QueryRecordingRecorder>,
    R: Call<recording::QueryRecordingRecorders>,
    R: Call<recording::QueryRecordingRecords>,
    R: Call<recording::QueryRecordingSchedule>,
    R: Call<recording::QueryRecordingSchedules>,
    R: Call<recording::RemoveRecordingRecord>,
    R: Call<recording::RemoveRecordingSchedule>,
    R: Call<recording::RemoveRecordingSchedules>,
    R: Call<recording::StartRecording>,
    R: Call<recording::StopRecording>,
    S: Send + Sync + 'static,
    S: Call<timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecord>,
    S: Call<timeshift::QueryTimeshiftRecords>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
    S: Call<timeshift::QueryTimeshiftRecorders>,
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
    T: Call<tuner::QueryTuners>,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
    E: Clone + Send + Sync + 'static,
    E: Call<epg::QueryChannel>,
    E: Call<epg::QueryChannels>,
    E: Call<epg::QueryClock>,
    E: Call<epg::QueryProgram>,
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryService>,
    E: Call<epg::QueryServices>,
    E: Call<epg::RemoveAirtime>,
    E: Call<epg::UpdateAirtime>,
    R: Send + Sync + 'static,
    R: Call<recording::AddRecordingSchedule>,
    R: Call<recording::QueryRecordingRecord>,
    R: Call<recording::QueryRecordingRecorder>,
    R: Call<recording::QueryRecordingRecorders>,
    R: Call<recording::QueryRecordingRecords>,
    R: Call<recording::QueryRecordingSchedule>,
    R: Call<recording::QueryRecordingSchedules>,
    R: Call<recording::RemoveRecordingRecord>,
    R: Call<recording::RemoveRecordingSchedule>,
    R: Call<recording::RemoveRecordingSchedules>,
    R: Call<recording::StartRecording>,
    R: Call<recording::StopRecording>,
    S: Send + Sync + 'static,
    S: Call<timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecord>,
    S: Call<timeshift::QueryTimeshiftRecords>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
    S: Call<timeshift::QueryTimeshiftRecorders>,
{
    let mut router = Router::new()
        .nest("/api", api::build_api(config))
        .merge(SwaggerUi::new("/api/debug").url("/api/docs", api::Docs::generate(config)));

    async fn convert_error(err: io::Error) -> Error {
        err.into()
    }

    for (mount_point, mount) in config.server.mounts.iter() {
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

// state

struct AppState<T, E, R, S> {
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
}

fn server_name() -> String {
    format!("mirakc/{}", env!("CARGO_PKG_VERSION"))
}
