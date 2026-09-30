use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::command_util::spawn_pipeline;

use super::*;

pub struct ContentSource {
    id: RecordId,
    kind: ContentSourceKind,
}

impl ContentSource {
    pub async fn new<C: Spawn>(
        config: &Config,
        record: &Record,
        range: Option<&ContentRange>,
        ctx: &C,
    ) -> Result<Self, Error> {
        let content_path = make_content_path(config, record).unwrap();
        if !content_path.exists() {
            tracing::warn!(?content_path, "No such file, maybe it has been removed");
            return Err(Error::NoContent);
        }

        let id = record.id.clone();
        let content_path_str = content_path.to_str().unwrap();
        let kind = match (range, &record.recording_status) {
            (None, RecordingStatus::Recording) => {
                // We use `tail -f` for streaming during recording in order to send data to be
                // appended to the content file in the future after the stream reaches EOF at that
                // point.
                //
                // NOTE: `tail` in macOS doesn't support `-s` option.  The default value of the
                // sleep interval of `tail` in GNU coreutils is 1.0 second.
                let cmd = format!("tail -f -c +0 '{content_path_str}'");
                ContentSourceKind::Pipeline(spawn_pipeline(vec![cmd], id.clone(), "content", ctx)?)
            }
            // Both a range request and a whole-content request are a seek followed by a limited
            // read.  A request without a range is simply the degenerate case of that: start at the
            // beginning and take everything.
            _ => {
                let (first, count) = match range {
                    Some(range) => {
                        debug_assert!(range.is_partial());
                        (range.first(), range.bytes())
                    }
                    None => (0, u64::MAX),
                };
                let mut file = tokio::fs::File::open(&content_path).await.map_err(|e| {
                    tracing::warn!(?content_path, %e, "Failed to open content file");
                    content_file_error(e)
                })?;
                // Seeking past the end of the file is not an error, and reading from there yields
                // no bytes.  That matches what `dd ibs=1 skip=N` did.
                //
                // The HTTP layer normalizes ranges against the known content length before we get
                // here, so a range past the end doesn't arrive through it.  We keep the `dd`
                // behaviour anyway, for ranges built directly from `ContentRange::without_size`
                // (which checks `first <= last` but holds no content length to check against) and
                // for a file that shrinks after its length was read.
                tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(first))
                    .await
                    .map_err(|e| {
                        tracing::warn!(?content_path, %e, "Failed to seek content file");
                        content_file_error(e)
                    })?;
                ContentSourceKind::File(Some(tokio::io::AsyncReadExt::take(file, count)))
            }
        };

        Ok(Self { id, kind })
    }

    pub fn create_stream(&mut self, time_limit: u64) -> ContentStream {
        // 32 KiB, large enough for 10 ms buffering.
        const CHUNK_SIZE: usize = 4096 * 8;

        match &mut self.kind {
            ContentSourceKind::Pipeline(pipeline) => {
                let (_, output) = pipeline.take_endpoints();
                let stream = ReaderStream::with_capacity(output, CHUNK_SIZE)
                    // We set a time limit in order to stop streaming when the stream reaches the
                    // *true* EOF.  Because `tail -f` doesn't terminate when the stream reaches an
                    // EOF at that point.
                    //
                    // We cannot use a RecordingStopped emitter for this purpose.  Because the
                    // streaming has to continue in order to send remaining data until the *true*
                    // EOF reaches.
                    .timeout(std::time::Duration::from_millis(time_limit))
                    .map_while(Result::ok);
                MpegTsStream::new(self.id.clone(), Box::pin(stream))
            }
            ContentSourceKind::File(reader) => {
                let reader = reader.take().expect("create_stream called twice");
                // No time limit here, unlike the pipeline above.  The time limit exists solely
                // because `tail -f` never terminates on its own; reading a regular file ends at
                // EOF.  The previous implementation streamed every case through a pipeline and so
                // applied the limit to `cat` and `dd` as well, which meant a storage stall longer
                // than the limit silently truncated the response instead of delaying it.
                let stream = ReaderStream::with_capacity(reader, CHUNK_SIZE);
                MpegTsStream::new(self.id.clone(), Box::pin(stream))
            }
        }
    }

    // TODO: remove
    #[cfg(test)]
    pub fn kind(&self) -> &ContentSourceKind {
        &self.kind
    }
}

#[async_trait]
impl Actor for ContentSource {
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(content_source.id = %self.id, "Started");
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(content_source.id = %self.id, "Stopping...");
        if let ContentSourceKind::Pipeline(pipeline) = &mut self.kind {
            pipeline.kill();
        }
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!(content_source.id = %self.id, "Stopped");
    }
}

pub enum ContentSourceKind {
    Pipeline(CommandPipeline<RecordId>),
    // `None` once `create_stream()` has taken the reader.
    File(Option<tokio::io::Take<tokio::fs::File>>),
}

impl std::fmt::Debug for ContentSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(_) => f.debug_tuple("Pipeline").finish(),
            Self::File(reader) => f.debug_tuple("File").field(&reader.is_some()).finish(),
        }
    }
}

// The content can be removed between the existence check and opening or seeking it.
fn content_file_error(err: std::io::Error) -> Error {
    match err.kind() {
        std::io::ErrorKind::NotFound => Error::NoContent,
        _ => Error::IoError(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_content_file_error() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_matches!(content_file_error(err), Error::NoContent);

        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_matches!(content_file_error(err), Error::IoError(err) => {
            assert_matches!(err.kind(), std::io::ErrorKind::PermissionDenied);
        });
    }
}
