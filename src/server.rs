//! TCP accept loop, HTTP/1 + optional h2c connection serving and graceful shutdown.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tower::ServiceExt;

use crate::config::Config;

#[derive(Debug, Clone, Copy)]
pub struct ClientAddr(pub SocketAddr);

pub async fn serve<F>(
    listener: TcpListener,
    app: Router,
    config: Arc<Config>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    let mut shutdown = pin!(shutdown);
    let mut connections = JoinSet::new();
    let connection_limit = Arc::new(Semaphore::new(config.server.max_connections));
    let graceful = GracefulShutdown::new();

    loop {
        tokio::select! {
            _ = shutdown.as_mut() => {
                tracing::info!("shutdown signal received; stopping accept loop");
                break;
            }
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::warn!(error = %error, "accept failed");
                        continue;
                    }
                };
                if let Err(error) = stream.set_nodelay(true) {
                    tracing::debug!(error = %error, "failed to set TCP_NODELAY");
                }
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                    tracing::warn!(%peer, "connection limit reached; dropping connection");
                    continue;
                };

                let config = Arc::clone(&config);
                if config.server.http2 {
                    let app = <Router as ServiceExt<Request<Body>>>::map_response(
                        app.clone(),
                        crate::http::routes::strip_bodyless_lengths,
                    );
                    let service = app.map_request(move |mut request: Request<Incoming>| {
                        request.extensions_mut().insert(ClientAddr(peer));
                        request.map(Body::new)
                    });
                    let watcher = graceful.watcher();
                    connections.spawn(async move {
                        let _permit = permit;
                        let mut builder = auto::Builder::new(TokioExecutor::new());
                        builder
                            .http1()
                            .timer(TokioTimer::new())
                            .header_read_timeout(config.server.header_read_timeout())
                            .max_buf_size(config.server.max_header_bytes)
                            .keep_alive(true);
                        builder.http2().max_concurrent_streams(Some(
                            config.server.http2_max_concurrent_streams,
                        ));
                        let connection = builder.serve_connection_with_upgrades(
                            TokioIo::new(stream),
                            TowerToHyperService::new(service),
                        );
                        if let Err(error) = watcher.watch(connection).await {
                            tracing::debug!(error = %error, "connection error");
                        }
                    });
                } else {
                    let app = <Router as ServiceExt<Request<Body>>>::map_response(
                        app.clone(),
                        crate::http::routes::strip_bodyless_lengths,
                    );
                    let service = app.map_request(move |mut request: Request<Incoming>| {
                        request.extensions_mut().insert(ClientAddr(peer));
                        request.map(Body::new)
                    });
                    let connection = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(config.server.header_read_timeout())
                        .max_buf_size(config.server.max_header_bytes)
                        .keep_alive(true)
                        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service));
                    let watched = graceful.watch(connection);
                    connections.spawn(async move {
                        let _permit = permit;
                        if let Err(error) = watched.await {
                            tracing::debug!(error = %error, "connection error");
                        }
                    });
                }

                while connections.try_join_next().is_some() {}
            }
        }
    }

    drop(listener);
    tracing::info!(
        open_connections = graceful.count(),
        "draining open connections"
    );

    let graceful_shutdown = graceful.shutdown();
    tokio::pin!(graceful_shutdown);
    let deadline = tokio::time::sleep(config.server.shutdown_grace());
    tokio::pin!(deadline);

    tokio::select! {
        _ = &mut graceful_shutdown => {
            tracing::info!("all connections closed before the shutdown deadline");
        }
        _ = &mut deadline => {
            tracing::warn!("shutdown grace period elapsed; aborting remaining connections");
            connections.abort_all();
        }
    }

    while connections.join_next().await.is_some() {}
    Ok(())
}
