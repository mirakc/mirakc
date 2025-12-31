use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::FORWARDED;
use axum::http::header::HOST;
use axum::http::uri::Authority;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum_extract::Host;

use super::Config;
use super::Error;
use super::peer_info::PeerInfo;

pub async fn access_control(
    State(config): State<Arc<Config>>,
    ConnectInfo(info): ConnectInfo<PeerInfo>,
    headers: HeaderMap,
    Host(host): Host,
    request: Request,
    next: Next,
) -> Response {
    if headers.get_all(FORWARDED).iter().count() > 1 {
        tracing::error!(
            "Multiple 'Forwarded' headers are not allowed for security reasons, disconnect"
        );
        return Error::AccessDenied.into_response();
    }

    if headers.get_all("X-Forwarded-Host").iter().count() > 1 {
        tracing::error!(
            "Multiple 'X-Forwarded-Host' headers are not allowed for security reasons, disconnect"
        );
        return Error::AccessDenied.into_response();
    }

    if headers.get_all(HOST).iter().count() > 1 {
        tracing::error!("Multiple 'Host' headers are not allowed for security reasons, disconnect");
        return Error::AccessDenied.into_response();
    }

    match info {
        PeerInfo::Tcp { addr } => {
            if !is_private_ip_addr(addr.ip()) {
                tracing::error!(tcp.addr = ?addr, "Non-private IP addr, disconnect");
                return Error::AccessDenied.into_response();
            }
            if let Some(allowed_hosts) = config.server.allowed_hosts.as_ref() {
                if !is_allowed_host(&host, allowed_hosts) {
                    tracing::error!(host, "Host not allowed, disconnect");
                    return Error::AccessDenied.into_response();
                }
            } else {
                // Always allowed for backward compatibility if allowed_hosts is not specified.
            }
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

fn is_allowed_host(host: &str, allowed_hosts: &[String]) -> bool {
    const LOCAL_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "[::1]"];
    let authority = Authority::from_str(host).unwrap();
    LOCAL_HOSTS.contains(&authority.host())
        || allowed_hosts
            .iter()
            .any(|allowed_host| allowed_host == host)
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
