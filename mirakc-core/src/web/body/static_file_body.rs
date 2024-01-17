use std::io;
use std::pin::Pin;
use std::task::ready;
use std::task::Context;
use std::task::Poll;

use bytes::Bytes;
use futures::stream::Stream;
use http_body::Frame;
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
    pub(in crate::web) async fn new(filepath: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let file = File::open(filepath).await?;
        let len = file.metadata().await?.len();
        Ok(StaticFileBody {
            stream: ReaderStream::new(file),
            len,
        })
    }
}

impl http_body::Body for StaticFileBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let data = ready!(Pin::new(&mut self.stream).poll_next(cx));
        Poll::Ready(data.map(|data| data.map(Frame::data)))
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.len)
    }
}
