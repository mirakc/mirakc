use std::sync::Arc;

use axum::extract::connect_info::Connected;
use tokio::net::TcpStream;
use tokio::net::UnixStream;

#[derive(Clone, Debug)]
pub(crate) enum PeerInfo {
    Tcp {
        addr: std::net::SocketAddr,
    },
    Unix {
        // tokio::net::unix::SocketAddr doesn't implement Clone.
        addr: Arc<tokio::net::unix::SocketAddr>,
        cred: tokio::net::unix::UCred,
    },
    #[cfg(test)]
    Test,
}

impl Connected<&TcpStream> for PeerInfo {
    fn connect_info(target: &TcpStream) -> Self {
        Self::Tcp {
            addr: target.peer_addr().unwrap(),
        }
    }
}

impl Connected<&UnixStream> for PeerInfo {
    fn connect_info(target: &UnixStream) -> Self {
        Self::Unix {
            addr: Arc::new(target.peer_addr().unwrap()),
            cred: target.peer_cred().unwrap(),
        }
    }
}
