use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::response::IntoResponse;
use axum::response::Response;
use bytes::Bytes;
use futures::Stream;
use http_body::Frame;
use http_body::SizeHint;
use http_body_util::StreamBody;

use crate::error::Error;

// StreamBody enforces the chunk encoding and no Content-Length header will be
// added if its stream source has a fixed size such as a file.
//
// This type is just a wrapper of StreamBody and implements
// Body::size_hint() in order to prevent the chunk encoding and add a
// Content-Length header with a specified size.
pub(in crate::web) struct SeekableStreamBody<S> {
    inner: StreamBody<S>,
    size: u64,
}

impl<S> SeekableStreamBody<S> {
    pub(in crate::web) fn new(inner: StreamBody<S>, size: u64) -> Self {
        SeekableStreamBody { inner, size }
    }
}

impl<S> http_body::Body for SeekableStreamBody<S>
where
    S: Stream<Item = Result<Frame<Bytes>, Error>> + Unpin,
{
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.size)
    }
}

impl<S> IntoResponse for SeekableStreamBody<S>
where
    S: 'static + Stream<Item = Result<Frame<Bytes>, Error>> + Send + Unpin,
{
    fn into_response(self) -> Response {
        Response::new(axum::body::Body::new(self))
    }
}
