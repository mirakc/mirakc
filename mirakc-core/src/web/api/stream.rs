use super::*;

use std::fmt;
use std::io;
use std::pin::Pin;

use axum::body::StreamBody;
use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CONTENT_RANGE;
use axum::http::header::TRANSFER_ENCODING;
use bytes::Bytes;
use futures::stream::Stream;
use futures::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use crate::command_util::spawn_pipeline;
use crate::epg::EpgChannel;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamRange;
use crate::mpeg_ts_stream::MpegTsStreamTerminator;
use crate::web::body::SeekableStreamBody;

pub(in crate::web::api) async fn do_get_service_stream<T>(
    config: &Config,
    tuner_manager: &T,
    channel: EpgChannel,
    sid: Sid,
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
            stream_id: None,
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

pub(in crate::web::api) async fn streaming<T, S, D>(
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

        let mut pipeline = spawn_pipeline(filters, stream.id(), "web.stream")?;

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

pub(in crate::web::api) fn do_head_stream(
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

pub(in crate::web::api) fn determine_stream_content_type<'a>(
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
