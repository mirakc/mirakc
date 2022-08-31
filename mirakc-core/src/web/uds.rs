use std::path::Path;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures::ready;
use hyper::server::accept::Accept;
use tokio::net::UnixListener;
use tokio::net::UnixStream;

use crate::error::Error;

// UDS support based on //examples/unix-domain-socket in tokio-rs/axum.

pub(super) struct UdsListener {
    uds: UnixListener,
}

impl UdsListener {
    pub(super) fn new<P: AsRef<Path>>(path: P) -> Self {
        UdsListener {
            uds: UnixListener::bind(path).unwrap(),
        }
    }
}

impl Accept for UdsListener {
    type Conn = UnixStream;
    type Error = Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let (stream, _addr) = ready!(Pin::new(&mut self.uds).poll_accept(cx))?;
        Poll::Ready(Some(Ok(stream)))
    }
}
