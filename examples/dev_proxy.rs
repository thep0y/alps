//! Local-only proxy harness for manual load testing.
//!
//! Build with: cargo run --features test-util --example dev_proxy
//! It maps DEV_UPSTREAM_HOST (default upstream.test) to 127.0.0.1 and allows
//! loopback only. Never ship this binary; release builds do not enable the
//! test-util feature.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use alps::app::{AppHandle, AppState};
use alps::config::{AllowlistRule, AuthMode, Config, LoadedConfig};
use alps::http::routes;
use alps::policy::IpPolicy;
use alps::server;
use alps::upstream::resolver::{SecureResolver, StaticDnsBackend};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::var("DEV_UPSTREAM_HOST").unwrap_or_else(|_| "upstream.test".to_string());
    let upstream_port: u16 = std::env::var("DEV_UPSTREAM_PORT")
        .unwrap_or_else(|_| "18000".to_string())
        .parse()?;
    let proxy_port: u16 = std::env::var("DEV_PROXY_PORT")
        .unwrap_or_else(|_| "18080".to_string())
        .parse()?;

    let mut config: Config = toml::from_str("")?;
    config.server.listen = format!("127.0.0.1:{proxy_port}");
    config.auth.mode = AuthMode::None;
    config.proxy.allow_http = true;
    config.allowlist = vec![AllowlistRule {
        host: host.clone(),
        include_subdomains: false,
        schemes: vec!["http".to_string()],
        ports: vec![upstream_port],
        path_prefixes: Vec::new(),
    }];
    config.validate(None)?;

    let mut map = HashMap::new();
    map.insert(host.clone(), vec![SocketAddr::new("127.0.0.1".parse()?, 0)]);
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

    let listener = TcpListener::bind(config.listen_addr()).await?;
    println!("dev proxy listening on http://127.0.0.1:{proxy_port}");
    println!("target host {host} -> 127.0.0.1:{upstream_port}");
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    server::serve(
        listener,
        routes::router(AppHandle::new(state)),
        Arc::new(config),
        shutdown,
    )
    .await?;
    Ok(())
}
