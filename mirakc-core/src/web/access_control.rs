use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::task::Context;
use std::task::Poll;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::response::IntoResponse;
use axum::response::Response;
use futures::future::BoxFuture;
use tower::Layer;
use tower::Service;

use crate::error::Error;

#[derive(Clone)]
pub(super) struct AccessControlLayer;

impl<S> Layer<S> for AccessControlLayer {
    type Service = AccessControlService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AccessControlService(inner)
    }
}

#[derive(Clone)]
pub(super) struct AccessControlService<S>(S);

impl<S> Service<Request<Body>> for AccessControlService<S>
where
    S: Service<Request<Body>, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    // `BoxFuture` is a type alias for `Pin<Box<dyn Future + Send + 'a>>`
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let info = req.extensions().get::<ConnectInfo<SocketAddr>>();

        let allowed = match info {
            Some(ConnectInfo(addr)) => {
                let allowed = is_private_ip_addr(addr.ip());
                tracing::debug!(?addr, allowed);
                allowed
            }
            None => {
                // UNIX domain socket
                tracing::debug!(addr = "uds", allowed = true);
                true
            }
        };

        if allowed {
            let fut = self.0.call(req);
            Box::pin(async move { Ok(fut.await?) })
        } else {
            Box::pin(async { Ok(Error::AccessDenied.into_response()) })
        }
    }
}

fn is_private_ip_addr(ip: IpAddr) -> bool {
    // TODO: IpAddr::is_global() is a nightly-only API at this point.
    match ip {
        IpAddr::V4(ip) => is_private_ipv4_addr(ip),
        IpAddr::V6(ip) => is_private_ipv6_addr(ip),
    }
}

fn is_private_ipv4_addr(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_private_ipv6_addr(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || match ip.to_ipv4() {
            // TODO: Support only IPv4-compatible and IPv4-mapped addresses at this
            //       moment.
            Some(ip) => is_private_ipv4_addr(ip),
            None => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_private_ip_addr() {
        assert!(is_private_ip_addr("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("::1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:7f00:1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:127.0.0.1".parse().unwrap()));

        assert!(is_private_ip_addr("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:a00:1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:10.0.0.1".parse().unwrap()));

        assert!(is_private_ip_addr("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:ac10:1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:172.16.0.1".parse().unwrap()));

        assert!(is_private_ip_addr("192.168.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:c0a8:1".parse().unwrap()));
        assert!(is_private_ip_addr("::ffff:192.168.0.1".parse().unwrap()));

        assert!(!is_private_ip_addr("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip_addr("::ffff:808:808".parse().unwrap()));
        assert!(!is_private_ip_addr("::ffff:8.8.8.8".parse().unwrap()));
    }
}
