use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;

use super::Error;
use super::peer_info::PeerInfo;

pub async fn access_control(
    ConnectInfo(info): ConnectInfo<PeerInfo>,
    request: Request,
    next: Next,
) -> Response {
    match info {
        PeerInfo::Tcp { addr } if !is_private_ip_addr(addr.ip()) => {
            tracing::error!(tcp.addr = ?addr, "Non-private IP addr, disconnect");
            return Error::AccessDenied.into_response();
        }
        PeerInfo::Tcp { .. } => {
            // TCP connections from remote clients in private networks are allowed.
        }
        PeerInfo::Unix { .. } => {
            // Connections via UNIX sockets are always allowed.
        }
        #[cfg(test)]
        PeerInfo::Test => {
            // Always allowed.
        }
    }

    next.run(request).await
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
    use test_log::test;

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
