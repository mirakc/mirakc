use std::io;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::body::HttpBody;
use axum::http::HeaderMap;
use bytes::Bytes;
use futures::stream::Stream;
use http_body::SizeHint;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

// StreamBody enforces the chunk encoding and no content-length header will be
// added.
pub(in crate::web) struct StaticFileBody {
    stream: ReaderStream<File>,
    len: u64,
}

impl StaticFileBody {
    pub(in crate::web) async fn new(
        filepath: impl AsRef<std::path::Path>
    ) -> io::Result<Self> {
        let file = File::open(filepath).await?;
        let len = file.metadata().await?.len();
        Ok(StaticFileBody {
            stream: ReaderStream::new(file),
            len,
        })
    }
}

impl HttpBody for StaticFileBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_data(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.len)
    }
}
