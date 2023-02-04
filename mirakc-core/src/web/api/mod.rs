use super::*;

use axum::extract::Path;
use axum::extract::Query;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use axum::TypedHeader;
use chrono::Duration;
use itertools::Itertools;
use utoipa::openapi::Server;
use utoipa::OpenApi;

use crate::epg;
use crate::filter::FilterPipelineBuilder;
use crate::models::*;
use crate::recording::RecordingOptions;
use crate::recording::RecordingScheduleState;
use crate::tuner;

use super::body::StaticFileBody;
use super::qs::Qs;
use super::server_name;
use super::AppState;
use super::X_MIRAKURUN_TUNER_USER_ID;

mod channels;
mod iptv;
mod onair;
mod programs;
mod recording;
mod services;
mod status;
mod stream;
mod timeshift;
mod tuners;
mod version;

pub(super) mod models;
use models::*;

pub(super) fn build_api<T, E, R, S, O>(config: &Config) -> Router<Arc<AppState<T, E, R, S, O>>>
where
    T: Clone + Send + Sync + 'static,
    T: Call<crate::tuner::QueryTuner>,
    T: Call<crate::tuner::QueryTuners>,
    T: Call<crate::tuner::StartStreaming>,
    T: TriggerFactory<crate::tuner::StopStreaming>,
    E: Clone + Send + Sync + 'static,
    E: Call<crate::epg::QueryChannel>,
    E: Call<crate::epg::QueryChannels>,
    E: Call<crate::epg::QueryClock>,
    E: Call<crate::epg::QueryProgram>,
    E: Call<crate::epg::QueryPrograms>,
    E: Call<crate::epg::QueryService>,
    E: Call<crate::epg::QueryServices>,
    R: Clone + Send + Sync + 'static,
    R: Call<crate::recording::AddRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingRecorder>,
    R: Call<crate::recording::QueryRecordingRecorders>,
    R: Call<crate::recording::QueryRecordingSchedule>,
    R: Call<crate::recording::QueryRecordingSchedules>,
    R: Call<crate::recording::RemoveRecordingSchedule>,
    R: Call<crate::recording::RemoveRecordingSchedules>,
    R: Call<crate::recording::StartRecording>,
    R: Call<crate::recording::StopRecording>,
    S: Clone + Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
    O: Clone + Send + Sync + 'static,
    O: Call<crate::onair::QueryOnairProgram>,
    O: Call<crate::onair::QueryOnairPrograms>,
    O: Call<crate::onair::SpawnTemporalTracker>,
{
    // As described in the `axum` documentation, a request handler registered
    // by `routing::get()` can be also used for HEAD requests.
    //
    // We implement a HEAD request handler for each streaming endpoint so that
    // we don't allocate a tuner for the request.

    let mut router = Router::new()
        .route("/version", routing::get(version::get))
        .route("/status", routing::get(status::get))
        .route("/tuners", routing::get(tuners::list))
        .route("/tuners/:index", routing::get(tuners::get))
        .route("/channels", routing::get(channels::list))
        .route(
            "/channels/:channel_type/:channel/stream",
            routing::get(channels::stream::get).head(channels::stream::head),
        )
        .route(
            "/channels/:channel_type/:channel/services/:sid/stream",
            routing::get(channels::services::stream::get).head(channels::services::stream::head),
        )
        .route("/services", routing::get(services::list))
        .route("/services/:id", routing::get(services::get))
        .route("/services/:id/logo", routing::get(services::logo))
        .route("/services/:id/programs", routing::get(services::programs))
        .route(
            "/services/:id/stream",
            routing::get(services::stream::get).head(services::stream::head),
        )
        .route("/programs", routing::get(programs::list))
        .route("/programs/:id", routing::get(programs::get))
        .route(
            "/programs/:id/stream",
            routing::get(programs::stream::get).head(programs::stream::head),
        )
        .route("/iptv/playlist", routing::get(iptv::playlist))
        // For compatibility with EPGStation
        .route("/iptv/channel.m3u8", routing::get(iptv::playlist))
        .route("/iptv/epg", routing::get(iptv::epg))
        // For compatibility with Mirakurun
        .route("/iptv/xmltv", routing::get(iptv::xmltv))
        .route("/onair", routing::get(onair::list))
        .route("/onair/:service_id", routing::get(onair::get));

    if config.recording.is_enabled() {
        tracing::info!("Enable endpoints for recording");
        router = router
            .route(
                "/recording/schedules",
                routing::get(recording::schedules::list),
            )
            .route(
                "/recording/schedules",
                routing::post(recording::schedules::create),
            )
            .route(
                "/recording/schedules",
                routing::delete(recording::schedules::clear),
            )
            .route(
                "/recording/schedules/:id",
                routing::get(recording::schedules::get),
            )
            .route(
                "/recording/schedules/:id",
                routing::delete(recording::schedules::delete),
            )
            .route(
                "/recording/recorders",
                routing::get(recording::recorders::list),
            )
            .route(
                "/recording/recorders",
                routing::post(recording::recorders::create),
            )
            .route(
                "/recording/recorders/:id",
                routing::get(recording::recorders::get),
            )
            .route(
                "/recording/recorders/:id",
                routing::delete(recording::recorders::delete),
            );
    };

    if config.timeshift.is_enabled() {
        tracing::info!("Enable endpoints for timeshift recording");
        router = router
            .route("/timeshift", routing::get(timeshift::list))
            .route("/timeshift/:recorder", routing::get(timeshift::get))
            .route(
                "/timeshift/:recorder/records",
                routing::get(timeshift::records::list),
            )
            .route(
                "/timeshift/:recorder/records/:id",
                routing::get(timeshift::records::get),
            )
            // The following two endpoints won't allocate any tuner.
            .route(
                "/timeshift/:recorder/stream",
                routing::get(timeshift::stream),
            )
            .route(
                "/timeshift/:recorder/records/:id/stream",
                routing::get(timeshift::records::stream),
            );
    }

    router
}

// openapi docs

#[derive(OpenApi)]
#[openapi(
    paths(
        version::get,
        status::get,
        tuners::list,
        tuners::get,
        channels::list,
        channels::stream::get,
        channels::stream::head,
        channels::services::stream::get,
        channels::services::stream::head,
        services::list,
        services::get,
        services::logo,
        services::programs,
        services::stream::get,
        services::stream::head,
        programs::list,
        programs::get,
        programs::stream::get,
        programs::stream::head,
        iptv::playlist,
        iptv::epg,
        iptv::xmltv,
        onair::list,
        onair::get,
        recording::schedules::list,
        recording::schedules::get,
        recording::schedules::create,
        recording::schedules::delete,
        recording::schedules::clear,
        recording::recorders::list,
        recording::recorders::get,
        recording::recorders::create,
        recording::recorders::delete,
        timeshift::list,
        timeshift::get,
        timeshift::stream,
        timeshift::records::list,
        timeshift::records::get,
        timeshift::records::stream,
    ),
    components(
        schemas(
            models::Status,
            models::Version,
            models::WebOnairProgram,
            models::WebProcessModel,
            models::WebRecordingRecorder,
            models::WebRecordingSchedule,
            models::WebRecordingScheduleInput,
            models::WebTimeshiftRecord,
            models::WebTimeshiftRecorder,
            ChannelType,
            MirakurunChannel,
            MirakurunProgram,
            MirakurunService,
            MirakurunTuner,
            RecordingOptions,
            RecordingScheduleState,
        ),
    ),
    modifiers(
        &BasicAddon,
        &MirakurunCompatAddon,
    ),
)]
pub(super) struct Docs;

impl Docs {
    // Currently, utoipa doesn't support conditional endpoints.  So, we define
    // all endpoints in the openapi proc macro, and then remove disabled
    // endpoints in this function.
    pub(super) fn generate(config: &Config) -> utoipa::openapi::OpenApi {
        let mut openapi = Self::openapi();

        // Remove endpoints for recording if it's disabled.
        if !config.recording.is_enabled() {
            openapi
                .paths
                .paths
                .retain(|path, _| !path.starts_with("/recording"))
        }

        // Remove endpoints for timeshift recording if it's disabled.
        if !config.timeshift.is_enabled() {
            openapi
                .paths
                .paths
                .retain(|path, _| !path.starts_with("/timeshift"))
        }

        openapi
    }
}

struct BasicAddon;

impl utoipa::Modify for BasicAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi.info.title = "mirakc Web API".to_string();
        openapi.servers = Some(vec![Server::new("/api")]);
    }
}

struct MirakurunCompatAddon;

impl utoipa::Modify for MirakurunCompatAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for path in openapi.paths.paths.values_mut() {
            // mirakurun.Client assumes that every path item has the
            // `parameters` properties even if it's empty.  See
            // mirakurun.Client.call().
            if path.parameters.is_none() {
                path.parameters = Some(vec![]);
            }

            // mirakurun.Client assumes that every streaming endpoints has a
            // `stream` tag.  See mirakurun.Client.call().
            for op in path.operations.values_mut() {
                let tags = match op.tags.as_mut() {
                    Some(tags) => tags,
                    None => continue,
                };
                if tags.iter().any(|tag| tag.ends_with("::stream")) {
                    tags.push("stream".to_string());
                }
            }
        }
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_openapi_docs() {
        use utoipa::openapi::PathItemType::*;

        let openapi = Docs::openapi();
        let paths = &openapi.paths;

        // Validate compatibility with one generated by Mirakurun.

        // NOTE
        // ----
        // utoipa::openapi::path::Operation doesn't implement std::fmt::Debug at
        // this point.  So, we cannot write like below:
        //
        // ```rust
        // assert_matches!(paths.get_path_operation("/version", Get), Some(op) => {
        //     ...
        // })
        // ```

        let op = paths.get_path_operation("/version", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "checkVersion"));

        let op = paths.get_path_operation("/status", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getStatus"));

        let op = paths.get_path_operation("/tuners", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getTuners"));

        let op = paths.get_path_operation("/channels", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getChannels"));

        let op = paths
            .get_path_operation("/channels/{type}/{channel}/stream", Get)
            .unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getChannelStream"));
        assert_matches!(op.tags, Some(ref tags) => assert!(tags.iter().any(|s| s == "stream")));

        let op = paths
            .get_path_operation("/channels/{type}/{channel}/services/{sid}/stream", Get)
            .unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getServiceStreamByChannel"));
        assert_matches!(op.tags, Some(ref tags) => assert!(tags.iter().any(|s| s == "stream")));

        let op = paths.get_path_operation("/services", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getServices"));

        let op = paths.get_path_operation("/services/{id}", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getService"));

        let op = paths
            .get_path_operation("/services/{id}/logo", Get)
            .unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getLogoImage"));

        let op = paths
            .get_path_operation("/services/{id}/stream", Get)
            .unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getServiceStream"));
        assert_matches!(op.tags, Some(ref tags) => assert!(tags.iter().any(|s| s == "stream")));

        let op = paths.get_path_operation("/programs", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getPrograms"));

        let op = paths.get_path_operation("/programs/{id}", Get).unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getProgram"));

        let op = paths
            .get_path_operation("/programs/{id}/stream", Get)
            .unwrap();
        assert_matches!(op.operation_id, Some(ref id) => assert_eq!(id, "getProgramStream"));
        assert_matches!(op.tags, Some(ref tags) => assert!(tags.iter().any(|s| s == "stream")));
    }
}
// </coverage:exclude>
