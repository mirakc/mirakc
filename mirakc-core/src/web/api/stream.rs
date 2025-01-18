use super::*;

use std::fmt;
use std::io;
use std::ops::Bound;
use std::pin::Pin;

use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CONTENT_LENGTH;
use axum::http::header::CONTENT_RANGE;
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use futures::TryStreamExt;
use http_body::Frame;
use http_body_util::StreamBody;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use crate::command_util::spawn_pipeline;
use crate::models::ContentRange;
use crate::mpeg_ts_stream::MpegTsStream;
use crate::mpeg_ts_stream::MpegTsStreamTerminator;
use crate::web::body::SeekableStreamBody;

#[derive(Debug)]
pub(in crate::web::api) struct StreamingHeaderParams {
    pub seekable: bool,
    pub content_type: String,
    pub length: Option<u64>,
    pub range: Option<ContentRange>,
    pub user: TunerUser,
}

pub(in crate::web::api) fn compute_content_range(
    ranges: &Option<TypedHeader<axum_extra::headers::Range>>,
    content_length: u64,
    incomplete: bool,
    seekable: bool,
) -> Result<Option<ContentRange>, Error> {
    match ranges {
        Some(TypedHeader(ranges)) => {
            let mut range = None;
            for (start, end) in ranges.satisfiable_ranges(content_length) {
                if range.is_some() {
                    return Err(Error::InvalidRequest("Multiple ranges are not supported"));
                }
                let first = match start {
                    Bound::Included(pos) => pos,
                    _ => unreachable!(),
                };
                let last = match end {
                    Bound::Included(pos) => pos,
                    Bound::Unbounded => content_length - 1, // 0-based index
                    _ => unreachable!(),
                };
                if incomplete {
                    range = Some(ContentRange::without_size(first, last)?);
                } else {
                    range = Some(ContentRange::with_size(first, last, content_length)?);
                }
            }
            if range.is_none() {
                return Err(Error::OutOfRange);
            }
            if !seekable {
                tracing::warn!("Not seekable, ignore the range and return the entire stream");
                range = None;
            }
            if let Some(ref r) = range {
                if !r.is_partial() {
                    // The range covers the entire content.
                    range = None;
                }
            }
            Ok(range)
        }
        None => Ok(None),
    }
}

pub(in crate::web::api) fn compute_content_length(
    size: u64,
    incomplete: bool,
    range: Option<&ContentRange>,
) -> Option<u64> {
    match (range, incomplete) {
        (Some(range), _) => Some(range.bytes()),
        (None, true) => None,
        (None, false) => Some(size),
    }
}

pub(in crate::web::api) async fn streaming<W, T, S, D>(
    config: &Config,
    spawner: &W,
    stream: MpegTsStream<T, S>,
    filters: Vec<String>,
    params: &StreamingHeaderParams,
    stop_triggers: D,
) -> Result<Response, Error>
where
    W: Spawn,
    T: fmt::Display + Clone + Send + Unpin + 'static,
    S: Stream<Item = io::Result<Bytes>> + Send + Unpin + 'static,
    D: Send + Unpin + 'static,
{
    let time_limit = config.server.stream_time_limit;

    if filters.is_empty() {
        do_streaming(
            stream,
            params,
            stop_triggers,
            config.server.stream_time_limit,
        )
        .await
    } else {
        tracing::debug!(?filters, "Streaming with filters");

        let mut pipeline = spawn_pipeline(filters, stream.id(), "web.stream")?;

        let (input, output) = pipeline.take_endpoints();

        let stream_id = stream.id();
        spawner.spawn_task(async move {
            let _ = stream.pipe(input).await;
        });

        // Use an MPSC channel as a buffer.
        //
        // The command pipeline often breaks when the client stops reading for a
        // few seconds.
        let mut stream = ReaderStream::with_capacity(output, config.server.stream_chunk_size);
        let (sender, receiver) = mpsc::channel(config.server.stream_max_chunks);
        spawner.spawn_task(async move {
            while let Some(result) = stream.next().await {
                if let Ok(chunk) = result {
                    tracing::trace!(
                        stream.id = %stream_id,
                        chunk.len = chunk.len(),
                        "Received a filtered chunk"
                    );
                    // The task yields if the buffer is full.
                    if sender.send(Ok(chunk)).await.is_err() {
                        tracing::debug!(stream.id = %stream_id, "Disconnected by client");
                        break;
                    }
                } else {
                    tracing::error!(stream.id = %stream_id, "Error, stop streaming");
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

        let stream = ReceiverStream::new(receiver);
        do_streaming(stream, params, stop_triggers, time_limit).await
    }
}

async fn do_streaming<S, D>(
    stream: S,
    params: &StreamingHeaderParams,
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
            let headers = build_headers(params);
            let body = StreamBody::new(peekable.map_ok(Frame::data).map_err(Error::from));
            if let Some(ref range) = params.range {
                let body = SeekableStreamBody::new(body, range.bytes());
                Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
            } else {
                Ok((headers, axum::body::Body::new(body)).into_response())
            }
        }
    }
}

fn build_headers(params: &StreamingHeaderParams) -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.insert(
        ACCEPT_RANGES,
        if params.seekable {
            header_value!("bytes")
        } else {
            header_value!("none")
        },
    );

    match params.length {
        Some(ref length) if params.seekable => {
            headers.insert(CONTENT_LENGTH, header_value!(length.to_string()));
        }
        _ => (),
    }

    headers.insert(CONTENT_TYPE, header_value!(params.content_type));

    headers.insert(
        super::X_MIRAKURUN_TUNER_USER_ID,
        header_value!(params.user.get_mirakurun_model().id),
    );

    if let Some(ref range) = params.range {
        debug_assert!(range.is_partial());
        headers.insert(CONTENT_RANGE, header_value!(range.make_http_header()));
    }

    headers
}

pub(in crate::web::api) fn do_head_stream(
    params: &StreamingHeaderParams,
) -> Result<Response, Error> {
    let headers = build_headers(params);

    // It's a dirt hack...
    //
    // Create an empty stream in order to prevent a "content-length: 0" header
    // from being added.
    let body = axum::body::Body::empty().into_data_stream();
    let body = axum::body::Body::from_stream(body);

    if params.range.is_some() {
        Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
    } else {
        Ok((headers, body).into_response())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_do_streaming() {
        let params = StreamingHeaderParams {
            seekable: false,
            content_type: "video/MP2T".to_string(),
            length: None,
            range: None,
            user: user_for_test(0.into()),
        };

        let result = do_streaming(futures::stream::empty(), &params, (), 1000).await;
        assert_matches!(result, Err(Error::ProgramNotFound));

        let result = do_streaming(futures::stream::pending(), &params, (), 1).await;
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
