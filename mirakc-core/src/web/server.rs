use std::path::PathBuf;
use std::sync::Arc;

use axum::http::Request;
use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use hyper_util::server;
use tokio::net::TcpListener;
use tokio::net::UnixListener;
use tower::Service;

use actlet::Spawn;
use actlet::Spawner;

use crate::config::Config;
use crate::error::Error;

use super::peer_info::PeerInfo;

pub(super) async fn serve(config: Arc<Config>, app: Router, spawner: Spawner) -> Result<(), Error> {
    let mut handles = vec![];
    for addr in config.server.http_addrs() {
        let (handle, _) = spawner.spawn_task(http(addr, app.clone(), spawner.clone()));
        handles.push(handle);
    }
    for path in config.server.uds_paths() {
        let (handle, _) = spawner.spawn_task(uds(path.to_owned(), app.clone(), spawner.clone()));
        handles.push(handle);
    }
    for handle in handles.into_iter() {
        let _ = handle.await;
    }
    Ok(())
}

// See //examples/serve-with-hyper in tokio-rs/axum.
// See //examples/unix-domain-socket in tokio-rs/axum.

macro_rules! listen {
    ($listener:expr, $app:expr, $spawner:expr) => {
        let mut make_service = $app.into_make_service_with_connect_info::<PeerInfo>();
        loop {
            let (socket, _remote_addr) = $listener.accept().await.unwrap();
            let tower_service = make_service.call(&socket).await.unwrap();
            $spawner.spawn_task(async move {
                let socket = TokioIo::new(socket);
                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
                        tower_service.clone().call(request)
                    });
                // TODO(refactor): use Spawner (or wrapper) as hyper::rt::Executor.
                // TokioExecutor::execute() calls tokio::spawn().
                let executor = TokioExecutor::new();
                if let Err(err) = server::conn::auto::Builder::new(executor)
                    .serve_connection(socket, hyper_service)
                    .await
                {
                    tracing::debug!(?err, "Failed to serve connection");
                }
            });
        }
    };
}

async fn http(addr: std::net::SocketAddr, app: Router, spawner: Spawner) {
    let listener = TcpListener::bind(&addr).await.unwrap();
    listen!(listener, app, spawner);
}

async fn uds(path: PathBuf, app: Router, spawner: Spawner) {
    // Remove the socket if it exists.
    let _ = tokio::fs::remove_file(&path).await;
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let listener = UnixListener::bind(path).unwrap();
    listen!(listener, app, spawner);
}
