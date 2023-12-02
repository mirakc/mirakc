use std::convert::Infallible;
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
use tokio::task::JoinSet;
use tower::Service;

use crate::config::Config;
use crate::error::Error;

use super::peer_info::PeerInfo;

pub(super) async fn serve(config: Arc<Config>, app: Router) -> Result<(), Error> {
    let mut set = JoinSet::new();
    for addr in config.server.http_addrs() {
        set.spawn(http(addr, app.clone()));
    }
    for path in config.server.uds_paths() {
        set.spawn(uds(path.to_owned(), app.clone()));
    }

    while let Some(_) = set.join_next().await {}
    Ok(())
}

// UDS support based on //examples/unix-domain-socket in tokio-rs/axum.

async fn http(addr: std::net::SocketAddr, app: Router) -> Result<(), Error> {
    let listener = TcpListener::bind(&addr).await?;

    let handle = tokio::spawn(async move {
        let mut make_service = app.into_make_service_with_connect_info::<PeerInfo>();
        loop {
            let (socket, _remote_addr) = listener.accept().await.unwrap();
            let tower_service = unwrap_infallible(make_service.call(&socket).await);
            tokio::spawn(async move {
                let socket = TokioIo::new(socket);
                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
                        tower_service.clone().call(request)
                    });
                if let Err(err) = server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(socket, hyper_service)
                    .await
                {
                    eprintln!("failed to serve connection: {err:#}");
                }
            });
        }
    });

    let _ = handle.await;
    Ok(())
}

async fn uds(path: PathBuf, app: Router) -> Result<(), Error> {
    // Cleanup the previous socket if it exists.
    let _ = tokio::fs::remove_file(&path).await;
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;

    let listener = UnixListener::bind(path)?;

    let handle = tokio::spawn(async move {
        let mut make_service = app.into_make_service_with_connect_info::<PeerInfo>();
        loop {
            let (socket, _remote_addr) = listener.accept().await.unwrap();
            let tower_service = unwrap_infallible(make_service.call(&socket).await);
            tokio::spawn(async move {
                let socket = TokioIo::new(socket);
                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
                        tower_service.clone().call(request)
                    });
                if let Err(err) = server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(socket, hyper_service)
                    .await
                {
                    eprintln!("failed to serve connection: {err:#}");
                }
            });
        }
    });

    let _ = handle.await;
    Ok(())
}

fn unwrap_infallible<T>(result: Result<T, Infallible>) -> T {
    match result {
        Ok(value) => value,
        Err(err) => match err {},
    }
}
