use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use actlet::*;
use axum::body::StreamBody;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CONTENT_RANGE;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::TRANSFER_ENCODING;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing;
use axum::Json;
use axum::Router;
use axum::TypedHeader;
use bytes::Bytes;
use chrono::Duration;
use futures::stream::Stream;
use futures::stream::StreamExt;
use itertools::Itertools;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use crate::airtime_tracker;
use crate::command_util::spawn_pipeline;
use crate::config::Config;
use crate::epg;
use crate::epg::EpgChannel;
use crate::error::Error;
use crate::filter::FilterPipelineBuilder;
use crate::models::*;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamRange;
use crate::mpeg_ts_stream::MpegTsStreamTerminator;
use crate::recording as rec;
use crate::tuner;
use crate::tuner::TunerStreamStopTrigger;

use super::body::SeekableStreamBody;
use super::body::StaticFileBody;
use super::qs::Qs;
use super::server_name;
use super::AppState;
use super::X_MIRAKURUN_TUNER_USER_ID;

mod channels;
mod iptv;
mod programs;
mod recording;
mod services;
mod status;
mod timeshift;
mod tuners;
mod version;

pub(super) mod models;
use models::*;

pub(super) fn build_api<T, E, R, S>(config: &Config) -> Router<Arc<AppState<T, E, R, S>>>
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
    R: Call<rec::AddRecordingSchedule>,
    R: Call<rec::QueryRecordingRecord>,
    R: Call<rec::QueryRecordingRecorder>,
    R: Call<rec::QueryRecordingRecorders>,
    R: Call<rec::QueryRecordingRecords>,
    R: Call<rec::QueryRecordingSchedule>,
    R: Call<rec::QueryRecordingSchedules>,
    R: Call<rec::RemoveRecordingRecord>,
    R: Call<rec::RemoveRecordingSchedule>,
    R: Call<rec::RemoveRecordingSchedules>,
    R: Call<rec::StartRecording>,
    R: Call<rec::StopRecording>,
    S: Send + Sync + 'static,
    S: Call<crate::timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<crate::timeshift::CreateTimeshiftRecordStreamSource>,
    S: Call<crate::timeshift::QueryTimeshiftRecord>,
    S: Call<crate::timeshift::QueryTimeshiftRecords>,
    S: Call<crate::timeshift::QueryTimeshiftRecorder>,
    S: Call<crate::timeshift::QueryTimeshiftRecorders>,
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
        .route("/timeshift", routing::get(timeshift::recorders::list))
        .route(
            "/timeshift/:recorder",
            routing::get(timeshift::recorders::get),
        )
        .route(
            "/timeshift/:recorder/records",
            routing::get(timeshift::recorders::records::list),
        )
        .route(
            "/timeshift/:recorder/records/:id",
            routing::get(timeshift::recorders::records::get),
        )
        // The following two endpoints won't allocate any tuner.
        .route(
            "/timeshift/:recorder/stream",
            routing::get(timeshift::recorders::stream),
        )
        .route(
            "/timeshift/:recorder/records/:id/stream",
            routing::get(timeshift::recorders::records::stream),
        )
        .route("/iptv/playlist", routing::get(iptv::playlist))
        // For compatibility with EPGStation
        .route("/iptv/channel.m3u8", routing::get(iptv::playlist))
        .route("/iptv/epg", routing::get(iptv::epg))
        // For compatibility with Mirakurun
        .route("/iptv/xmltv", routing::get(iptv::xmltv))
        .route("/docs", routing::get(get_docs));

    if config.recording.records_dir.is_some() {
        tracing::info!("Enable endpoints for rec");
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
            )
            .route("/recording/records", routing::get(recording::records::list))
            .route(
                "/recording/records/:id",
                routing::get(recording::records::get),
            )
            .route(
                "/recording/records/:id",
                routing::delete(recording::records::delete),
            )
            .route(
                "/recording/records/:id/stream",
                routing::get(recording::records::stream),
            );
    };

    router
}

// Request Handlers

async fn get_docs<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Response<StaticFileBody>, Error> {
    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/json")
        .body(StaticFileBody::new(&state.config.mirakurun.openapi_json).await?)?)
}

// helpers

async fn do_get_service_stream<T>(
    config: &Config,
    tuner_manager: &T,
    channel: EpgChannel,
    sid: ServiceId,
    user: TunerUser,
    filter_setting: FilterSetting,
) -> Result<Response, Error>
where
    T: Clone,
    T: Call<tuner::StartStreaming>,
    T: Into<Emitter<tuner::StopStreaming>>,
{
    let stream = tuner_manager
        .call(tuner::StartStreaming {
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
                super::X_MIRAKURUN_TUNER_USER_ID,
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

fn do_head_stream(
    config: &Config,
    user: &TunerUser,
    filter_setting: &FilterSetting,
) -> Result<Response, Error> {
    let content_type = determine_stream_content_type(&config, &filter_setting);

    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_RANGES, header_value!("none"));
    headers.insert(CONTENT_TYPE, header_value!(content_type));
    headers.insert(
        super::X_MIRAKURUN_TUNER_USER_ID,
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

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[tokio::test]
    async fn test_do_streaming() {
        let user = user_for_test(0.into());

        let result = do_streaming(
            user.clone(),
            futures::stream::empty(),
            "video/MP2T".to_string(),
            None,
            (),
            1000,
        )
        .await;
        assert_matches!(result, Err(Error::ProgramNotFound));

        let result = do_streaming(
            user.clone(),
            futures::stream::pending(),
            "video/MP2T".to_string(),
            None,
            (),
            1,
        )
        .await;
        assert_matches!(result, Err(Error::StreamingTimedOut));
    }

    fn user_for_test(priority: TunerUserPriority) -> TunerUser {
        TunerUser {
            info: TunerUserInfo::Web {
                id: "".to_string(),
                agent: None,
            },
            priority,
        }
    }
}
// </coverage:exclude>
