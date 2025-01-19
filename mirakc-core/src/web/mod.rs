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
use tower_http::trace::TraceLayer;
use utoipa_swagger_ui::SwaggerUi;

use crate::config::Config;
use crate::error::Error;
use crate::string_table::StringTable;

use access_control::AccessControlLayer;
use default_headers::DefaultHeadersLayer;

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
pub mod api;
mod body;
mod default_headers;
mod error;
mod escape;
mod mount;
mod peer_info;
mod qs;
mod server;
mod sse;

#[cfg(test)]
mod tests;

pub(crate) use api::models::WebOnairProgram;

#[allow(clippy::too_many_arguments)]
pub async fn serve<T, E, R, S, O, W>(
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
    onair_manager: O,
    spawner: W,
) -> Result<(), Error>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::QueryTuner>,
    T: Call<crate::tuner::QueryTuners>,
    T: Call<crate::tuner::RegisterEmitter>,
    T: Call<crate::tuner::StartStreaming>,
    T: TriggerFactory<crate::tuner::StopStreaming>,
    T: TriggerFactory<crate::tuner::UnregisterEmitter>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryChannel>,
    E: Call<crate::epg::QueryChannels>,
    E: Call<crate::epg::QueryClock>,
    E: Call<crate::epg::QueryProgram>,
    E: Call<crate::epg::QueryPrograms>,
    E: Call<crate::epg::QueryService>,
    E: Call<crate::epg::QueryServices>,
    E: Call<crate::epg::RegisterEmitter>,
    E: TriggerFactory<crate::epg::UnregisterEmitter>,
    R: Clone + Send + Sync + 'static,
    R: Call<crate::recording::AddRecordingSchedule>,
    R: Call<crate::recording::OpenContent>,
    R: Call<crate::recording::QueryRecord>,
    R: Call<crate::recording::QueryRecordingRecorder>,
    R: Call<crate::recording::QueryRecordingRecorders>,
    R: Call<crate::recording::QueryRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingSchedules>,
    R: Call<crate::recording::QueryRecords>,
    R: Call<crate::recording::RegisterEmitter>,
    R: Call<crate::recording::RemoveRecord>,
    R: Call<crate::recording::RemoveRecordingSchedule>,
    R: Call<crate::recording::RemoveRecordingSchedules>,
    R: Call<crate::recording::StartRecording>,
    R: Call<crate::recording::StopRecording>,
    R: TriggerFactory<crate::recording::UnregisterEmitter>,
    S: Clone + Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
    S: Call<crate::timeshift::RegisterEmitter>,
    S: TriggerFactory<crate::timeshift::UnregisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair::QueryOnairProgram>,
    O: Call<crate::onair::QueryOnairPrograms>,
    O: Call<crate::onair::RegisterEmitter>,
    O: Call<crate::onair::SpawnTemporalTracker>,
    O: TriggerFactory<crate::onair::UnregisterEmitter>,
    W: Clone + Send + Sync + 'static,
    W: Spawn,
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
            onair_manager,
            spawner: spawner.clone(),
        }));

    server::serve(config, app, spawner).await
}

// headers

const X_MIRAKURUN_TUNER_USER_ID: &str = "x-mirakurun-tuner-user-id";
const X_MIRAKURUN_PRIORITY: &str = "x-mirakurun-priority";

// endpoints

#[allow(clippy::type_complexity)]
fn build_app<T, E, R, S, O, W>(config: &Config) -> Router<Arc<AppState<T, E, R, S, O, W>>>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::QueryTuner>,
    T: Call<crate::tuner::QueryTuners>,
    T: Call<crate::tuner::RegisterEmitter>,
    T: Call<crate::tuner::StartStreaming>,
    T: TriggerFactory<crate::tuner::StopStreaming>,
    T: TriggerFactory<crate::tuner::UnregisterEmitter>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryChannel>,
    E: Call<crate::epg::QueryChannels>,
    E: Call<crate::epg::QueryClock>,
    E: Call<crate::epg::QueryProgram>,
    E: Call<crate::epg::QueryPrograms>,
    E: Call<crate::epg::QueryService>,
    E: Call<crate::epg::QueryServices>,
    E: Call<crate::epg::RegisterEmitter>,
    E: TriggerFactory<crate::epg::UnregisterEmitter>,
    R: Clone + Send + Sync + 'static,
    R: Call<crate::recording::AddRecordingSchedule>,
    R: Call<crate::recording::OpenContent>,
    R: Call<crate::recording::QueryRecord>,
    R: Call<crate::recording::QueryRecordingRecorder>,
    R: Call<crate::recording::QueryRecordingRecorders>,
    R: Call<crate::recording::QueryRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingSchedules>,
    R: Call<crate::recording::QueryRecords>,
    R: Call<crate::recording::RegisterEmitter>,
    R: Call<crate::recording::RemoveRecord>,
    R: Call<crate::recording::RemoveRecordingSchedule>,
    R: Call<crate::recording::RemoveRecordingSchedules>,
    R: Call<crate::recording::StartRecording>,
    R: Call<crate::recording::StopRecording>,
    R: TriggerFactory<crate::recording::UnregisterEmitter>,
    S: Clone + Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
    S: Call<crate::timeshift::RegisterEmitter>,
    S: TriggerFactory<crate::timeshift::UnregisterEmitter>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair::QueryOnairProgram>,
    O: Call<crate::onair::QueryOnairPrograms>,
    O: Call<crate::onair::RegisterEmitter>,
    O: Call<crate::onair::SpawnTemporalTracker>,
    O: TriggerFactory<crate::onair::UnregisterEmitter>,
    W: Clone + Send + Sync + 'static,
    W: Spawn,
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

struct AppState<T, E, R, S, O, W> {
    config: Arc<Config>,
    string_table: Arc<StringTable>,
    tuner_manager: T,
    epg: E,
    recording_manager: R,
    timeshift_manager: S,
    onair_manager: O,
    spawner: W,
}

struct ConfigExtractor(Arc<Config>);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for ConfigExtractor {
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.config.clone())
    }
}

struct StringTableExtractor(Arc<StringTable>);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for StringTableExtractor {
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.string_table.clone())
    }
}

struct TunerManagerExtractor<T>(T);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for TunerManagerExtractor<T>
where
    T: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.tuner_manager.clone())
    }
}

struct EpgExtractor<E>(E);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for EpgExtractor<E>
where
    E: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.epg.clone())
    }
}

struct RecordingManagerExtractor<R>(R);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for RecordingManagerExtractor<R>
where
    R: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.recording_manager.clone())
    }
}

struct TimeshiftManagerExtractor<S>(S);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for TimeshiftManagerExtractor<S>
where
    S: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.timeshift_manager.clone())
    }
}

struct OnairProgramManagerExtractor<S>(S);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for OnairProgramManagerExtractor<O>
where
    O: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.onair_manager.clone())
    }
}

struct SpawnerExtractor<W>(W);

impl<T, E, R, S, O, W> FromRef<Arc<AppState<T, E, R, S, O, W>>> for SpawnerExtractor<W>
where
    W: Clone,
{
    fn from_ref(state: &Arc<AppState<T, E, R, S, O, W>>) -> Self {
        Self(state.spawner.clone())
    }
}

// helpers

fn server_name() -> String {
    format!("mirakc/{}", env!("CARGO_PKG_VERSION"))
}
