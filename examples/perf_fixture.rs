//! Local-only fixture for resource tests: a streaming upstream plus ALPS with
//! a permissive loopback resolver. Build with:
//!   cargo build --release --features test-util --example perf_fixture
//! Never ship this binary.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use alps::app::{AppHandle, AppState};
use alps::config::{AllowlistRule, AuthMode, Config, LoadedConfig};
use alps::http::routes;
use alps::policy::IpPolicy;
use alps::server;
use alps::telemetry;
use alps::upstream::resolver::{SecureResolver, StaticDnsBackend};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures_util::stream;
use tokio::net::TcpListener;

const CHUNK: usize = 1024 * 1024;
static ZERO: [u8; CHUNK] = [0u8; CHUNK];

fn file_response(size: u64) -> Response {
    let chunk = CHUNK as u64;
    let count = size.div_ceil(chunk);
    let body = stream::iter((0..count).map(move |index| {
        let remaining = size - index * chunk;
        let len = remaining.min(chunk) as usize;
        let bytes = if len == CHUNK {
            Bytes::from_static(&ZERO)
        } else {
            Bytes::copy_from_slice(&ZERO[..len])
        };
        Ok::<Bytes, std::io::Error>(bytes)
    }));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(body))
        .expect("valid response")
}

#[derive(Default)]
struct Stats {
    peak_rss_kb: AtomicU64,
    rss_kb: AtomicU64,
    vm_peak_kb: AtomicU64,
    file_mb: u64,
}

struct StatsState {
    stats: Arc<Stats>,
    app: Arc<AppState>,
}

fn read_status() -> (u64, u64, u64) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return (0, 0, 0);
    };
    let mut rss = 0;
    let mut peak = 0;
    let mut vm_peak = 0;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss = value.trim().trim_end_matches(" kB").parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak = value.trim().trim_end_matches(" kB").parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("VmPeak:") {
            vm_peak = value.trim().trim_end_matches(" kB").parse().unwrap_or(0);
        }
    }
    (rss, peak, vm_peak)
}

async fn stats_handler(State(state): State<Arc<StatsState>>) -> impl IntoResponse {
    let (rss, peak, vm_peak) = read_status();
    state.stats.rss_kb.store(rss, Ordering::Relaxed);
    state.stats.peak_rss_kb.fetch_max(peak, Ordering::Relaxed);
    state.stats.vm_peak_kb.fetch_max(vm_peak, Ordering::Relaxed);
    axum::Json(serde_json::json!({
        "peak_rss_kb": state.stats.peak_rss_kb.load(Ordering::Relaxed),
        "rss_kb": rss,
        "vm_peak_kb": state.stats.vm_peak_kb.load(Ordering::Relaxed),
        "file_mb": state.stats.file_mb,
        "active_streams": state.app.limits.active_streams(),
    }))
}

#[tokio::main(worker_threads = 1)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_mb: u64 = std::env::var("PERF_FILE_MB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(32);
    let size = file_mb.saturating_mul(1024 * 1024);
    let stats = Arc::new(Stats {
        file_mb,
        ..Stats::default()
    });

    let stats_clone = Arc::clone(&stats);
    tokio::spawn(async move {
        loop {
            let (rss, peak, vm_peak) = read_status();
            stats_clone.rss_kb.store(rss, Ordering::Relaxed);
            stats_clone.peak_rss_kb.fetch_max(peak, Ordering::Relaxed);
            stats_clone.vm_peak_kb.fetch_max(vm_peak, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let upstream = Router::new().route(
        "/file",
        get(move || {
            let size = size;
            async move { file_response(size) }
        }),
    );
    let upstream_listener = TcpListener::bind("127.0.0.1:19090").await?;
    tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream).await;
    });

    let mut config: Config = toml::from_str("")?;
    config.server.listen = "0.0.0.0:18080".to_string();
    config.server.max_connections = 512;
    config.auth.mode = AuthMode::None;
    config.security.trusted_network_only = true;
    config.proxy.allow_http = true;
    config.proxy.max_response_bytes = 0;
    config.limits.max_active_streams = 100;
    config.limits.max_active_streams_per_host = 100;
    config.allowlist = vec![AllowlistRule {
        host: "upstream.test".to_string(),
        include_subdomains: false,
        schemes: vec!["http".to_string()],
        ports: vec![19090],
        path_prefixes: Vec::new(),
    }];
    config.logging.level = "warn".to_string();
    config.validate(None)?;
    let _ = telemetry::init(&config);

    let mut map = HashMap::new();
    map.insert(
        "upstream.test".to_string(),
        vec![SocketAddr::new("127.0.0.1".parse()?, 0)],
    );
    let resolver = Arc::new(SecureResolver::new(
        Arc::new(StaticDnsBackend::new(map)),
        IpPolicy::permissive_for_tests(),
        Duration::from_secs(2),
    ));
    let state = AppState::build_with_resolver(
        &LoadedConfig {
            config: config.clone(),
            api_token: None,
            signing_secrets: Vec::new(),
        },
        resolver,
    )?;

    let stats_app = Router::new()
        .route("/stats", get(stats_handler))
        .with_state(Arc::new(StatsState {
            stats: Arc::clone(&stats),
            app: Arc::clone(&state),
        }));
    let stats_listener = TcpListener::bind("0.0.0.0:18081").await?;
    tokio::spawn(async move {
        let _ = axum::serve(stats_listener, stats_app).await;
    });

    let listener = TcpListener::bind(config.listen_addr()).await?;
    eprintln!(
        "perf fixture ready: proxy=:18080 stats=:18081 file_mb={file_mb} upstream=127.0.0.1:19090"
    );

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let terminate = async {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut signal) => {
                        signal.recv().await;
                    }
                    Err(_) => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = ctrl_c => {},
                _ = terminate => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
        }
    };

    server::serve(
        listener,
        routes::router(AppHandle::new(state)),
        Arc::new(config),
        shutdown,
    )
    .await?;

    let (rss, peak, vm_peak) = read_status();
    eprintln!("perf fixture stopped: peak_rss_kb={peak} rss_kb={rss} vm_peak_kb={vm_peak}");
    Ok(())
}
