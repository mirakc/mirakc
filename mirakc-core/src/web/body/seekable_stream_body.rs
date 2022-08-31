use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::BoxError;
use axum::body::HttpBody;
use axum::body::StreamBody;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::response::Response;
use bytes::Bytes;
use futures::stream::TryStream;
use http_body::SizeHint;

// StreamBody enforces the chunk encoding and no Content-Length header will be
// added if its stream source has a fixed size such as a file.
//
// This type is just a wrapper of StreamBody and implements
// HttpBody::size_hint() in order to prevent the chunk encoding and add a
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

impl<S> HttpBody for SeekableStreamBody<S>
where
    S: TryStream + Unpin,
    S::Ok: Into<Bytes>,
    S::Error: Into<BoxError>,
{
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_data(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        Pin::new(&mut self.inner).poll_data(cx)
    }

    fn poll_trailers(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Pin::new(&mut self.inner).poll_trailers(cx)
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.size)
    }
}

impl<S> IntoResponse for SeekableStreamBody<S>
where
    S: 'static + Send + TryStream + Unpin,
    S::Ok: Into<Bytes>,
    S::Error: Into<BoxError>,
{
    fn into_response(self) -> Response {
        Response::new(axum::body::boxed(self))
    }
}
