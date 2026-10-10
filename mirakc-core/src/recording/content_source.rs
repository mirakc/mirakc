use bytes::BytesMut;
use tokio::sync::watch;

use super::*;

// Wakes the followers each time data may have reached the file.
//
// `tokio::fs::File::poll_write()` returns before the data is written to the file, so notifying only
// on writes, as `tokio_util::io::InspectWriter` would, delays the latest data until the next write.
// `poll_flush()` returns after the data is written, and `tokio::io::copy()` flushes whenever the
// input has nothing to read.
pub struct ProgressWriter<'a, W> {
    pub writer: W,
    pub progress: &'a watch::Sender<bool>,
}

impl<W> tokio::io::AsyncWrite for ProgressWriter<'_, W>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let result = std::task::ready!(Pin::new(&mut self.writer).poll_write(cx, buf));
        self.progress.send_replace(false);
        std::task::Poll::Ready(result)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let result = std::task::ready!(Pin::new(&mut self.writer).poll_flush(cx));
        self.progress.send_replace(false);
        std::task::Poll::Ready(result)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

// Writes the output of the recording pipeline to the content file.  `progress` wakes the followers
// while writing and becomes `true` once everything has been flushed.
pub async fn write_content<R>(
    mut output: R,
    content_path: &Path,
    progress: watch::Sender<bool>,
) -> std::io::Result<u64>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let file = tokio::fs::File::create(content_path).await?;
    let mut writer = BufWriter::new(ProgressWriter {
        writer: file,
        progress: &progress,
    });
    // TODO: use Stdio
    let n = tokio::io::copy(&mut output, &mut writer).await?;
    // `copy()` flushes the writer before returning.  `progress` is dropped without this on an
    // error, which also ends the followers.
    progress.send_replace(true);
    Ok(n)
}

// Every request is a seek followed by a limited read.  A request without a range is simply the
// degenerate case of that: start at the beginning and take everything.
pub async fn open_content_stream(
    config: &Config,
    record: &Record,
    range: Option<&ContentRange>,
    progress: Option<watch::Receiver<bool>>,
) -> Result<ContentStream, Error> {
    let content_path = make_content_path(config, record).unwrap();
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
    // Seeking past the end of the file is not an error, and reading from there yields no bytes.
    // That matches what `dd ibs=1 skip=N` did.
    //
    // The HTTP layer normalizes ranges against the known content length before we get here, so a
    // range past the end doesn't arrive through it.  We keep the `dd` behaviour anyway, for ranges
    // built directly from `ContentRange::without_size` (which checks `first <= last` but holds no
    // content length to check against) and for a file that shrinks after its length was read.
    tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(first))
        .await
        .map_err(|e| {
            tracing::warn!(?content_path, %e, "Failed to seek content file");
            content_file_error(e)
        })?;
    let stream = content_stream(tokio::io::AsyncReadExt::take(file, count), progress);
    Ok(MpegTsStream::new(record.id.clone(), Box::pin(stream)))
}

// Streams `reader` until EOF.  With `progress`, EOF only pauses the stream until the recorder
// writes more or reports that it has finished.
//
// There is no time limit.  A storage stall delays the stream instead of truncating it.
pub fn content_stream<R>(
    reader: R,
    progress: Option<watch::Receiver<bool>>,
) -> impl Stream<Item = std::io::Result<Bytes>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // 32 KiB, large enough for 10 ms buffering.
    const CHUNK_SIZE: usize = 4096 * 8;

    futures::stream::try_unfold(
        (reader, progress, BytesMut::new()),
        |(mut reader, mut progress, mut buf)| async move {
            loop {
                // Read the flag *before* reading the file so that a write in between wakes
                // `changed()` below.
                let done = progress.as_mut().is_none_or(|p| *p.borrow_and_update());
                buf.reserve(CHUNK_SIZE);
                if tokio::io::AsyncReadExt::read_buf(&mut reader, &mut buf).await? > 0 {
                    return Ok(Some((buf.split().freeze(), (reader, progress, buf))));
                }
                if done {
                    return Ok(None);
                }
                if progress.as_mut().unwrap().changed().await.is_err() {
                    // The recorder has gone away on an error.  Send what is on disk and stop.
                    progress = None;
                }
            }
        },
    )
}

// The content can be removed between loading the record and opening or seeking the content file.
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
