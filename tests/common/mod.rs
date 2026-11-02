#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::net::TcpListener;
use tower::ServiceExt;

use alps::app::{AppHandle, AppState};
use alps::config::{AllowlistRule, Config, LoadedConfig};
use alps::http::routes;
use alps::policy::IpPolicy;
use alps::server::ClientAddr;
use alps::upstream::resolver::{SecureResolver, StaticDnsBackend};

pub const FILE_BODY: &[u8] = b"0123456789abcdef";

/// Start a controlled upstream and return its ephemeral port.
pub async fn start_upstream() -> u16 {
    let app = Router::new()
        .route("/file", get(file).head(file_head))
        .route("/inspect", get(inspect))
        .route("/redirect", get(redirect_relative))
        .route("/redirect-external", get(redirect_external))
        .route("/not-modified", get(not_modified))
        .route("/large", get(large))
        .route("/large-128k", get(large_128k))
        .route("/status-300", get(status_300))
        .route("/chain/{n}", get(chain))
        .route("/bad-location", get(bad_location))
        .route("/encoded", get(encoded))
        .route("/loop", get(loop_redirect))
        .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
        .route("/inspect-body", get(|| async { (StatusCode::OK, "ok") }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

async fn file(headers: HeaderMap) -> Response {
    file_response(headers, true)
}

async fn file_head(headers: HeaderMap) -> Response {
    file_response(headers, false)
}

fn file_response(headers: HeaderMap, with_body: bool) -> Response {
    let total = FILE_BODY.len();
    if let Some(range) = headers
        .get(axum::http::header::RANGE)
        .and_then(|value| value.to_str().ok())
    {
        if let Some((start, end)) = parse_range(range, total) {
            let slice = FILE_BODY[start..=end].to_vec();
            let body = if with_body {
                Body::from(slice)
            } else {
                Body::empty()
            };
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    axum::http::header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{total}"),
                )
                .header(axum::http::header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap();
        }
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(
                axum::http::header::CONTENT_RANGE,
                format!("bytes */{total}"),
            )
            .body(Body::empty())
            .unwrap();
    }

    let body = if with_body {
        Body::from(FILE_BODY)
    } else {
        Body::empty()
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_LENGTH, total)
        .header(axum::http::header::ACCEPT_RANGES, "bytes")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            "attachment; filename=\"file.bin\"",
        )
        .body(body)
        .unwrap()
}

fn parse_range(value: &str, total: usize) -> Option<(usize, usize)> {
    let value = value.strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    let start: usize = start.parse().ok()?;
    let end = if end.is_empty() {
        total.saturating_sub(1)
    } else {
        end.parse().ok()?
    };
    if start >= total || start > end {
        return None;
    }
    Some((start, end.min(total - 1)))
}

async fn inspect(headers: HeaderMap) -> Response {
    let value = serde_json::json!({
        "cookie": headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()),
        "authorization": headers.get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok()),
        "accept_encoding": headers.get(axum::http::header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()),
    });
    axum::Json(value).into_response()
}

async fn redirect_relative() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(axum::http::header::LOCATION, "/file")
        .body(Body::empty())
        .unwrap()
}

async fn redirect_external() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(axum::http::header::LOCATION, "http://evil.test/file")
        .body(Body::empty())
        .unwrap()
}

async fn not_modified(headers: HeaderMap) -> Response {
    if headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some("\"v1\"")
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(axum::http::header::ETAG, "\"v1\"")
            .body(Body::empty())
            .unwrap();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::ETAG, "\"v1\"")
        .header(axum::http::header::CONTENT_TYPE, "text/plain")
        .body(Body::from("fresh"))
        .unwrap()
}

async fn encoded() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_ENCODING, "gzip")
        .header(axum::http::header::CONTENT_LENGTH, 3)
        .body(Body::from("abc"))
        .unwrap()
}

async fn loop_redirect() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(axum::http::header::LOCATION, "/loop")
        .body(Body::empty())
        .unwrap()
}

async fn large() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_LENGTH, 4096)
        .body(Body::from(vec![b'x'; 4096]))
        .unwrap()
}

async fn large_128k() -> Response {
    const SIZE: usize = 128 * 1024;
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_LENGTH, SIZE)
        .body(Body::from(vec![b'x'; SIZE]))
        .unwrap()
}

async fn status_300() -> Response {
    Response::builder()
        .status(StatusCode::MULTIPLE_CHOICES)
        .header(axum::http::header::LOCATION, "/file")
        .body(Body::empty())
        .unwrap()
}

async fn chain(Path(n): Path<u32>) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(axum::http::header::LOCATION, format!("/chain/{}", n + 1))
        .body(Body::empty())
        .unwrap()
}

async fn bad_location() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(axum::http::header::LOCATION, "http://[")
        .body(Body::empty())
        .unwrap()
}

pub fn resolver_for(hosts: &[(&str, u16)]) -> Arc<SecureResolver> {
    let mut map = HashMap::new();
    for (host, _port) in hosts {
        map.insert(
            (*host).to_string(),
            vec![SocketAddr::new("127.0.0.1".parse().unwrap(), 0)],
        );
    }
    Arc::new(SecureResolver::new(
        Arc::new(StaticDnsBackend::new(map)),
        IpPolicy::permissive_for_tests(),
        Duration::from_secs(2),
    ))
}

pub fn strict_resolver_for(hosts: &[(&str, u16)]) -> Arc<SecureResolver> {
    let mut map = HashMap::new();
    for (host, _port) in hosts {
        map.insert(
            (*host).to_string(),
            vec![SocketAddr::new("127.0.0.1".parse().unwrap(), 0)],
        );
    }
    Arc::new(SecureResolver::new(
        Arc::new(StaticDnsBackend::new(map)),
        IpPolicy::strict(),
        Duration::from_secs(2),
    ))
}

pub fn config_for(port: u16, extra_hosts: &[&str], auth_none: bool) -> Config {
    let mut config: Config = toml::from_str(
        r#"
[server]
listen = "127.0.0.1:0"
[auth]
mode = "none"
[proxy]
allow_http = true
[limits]
max_active_streams = 16
max_active_streams_per_host = 8
[[allowlist]]
host = "upstream.test"
schemes = ["http"]
ports = [443]
"#,
    )
    .unwrap();
    config.allowlist.clear();
    config.allowlist.push(AllowlistRule {
        host: "upstream.test".to_string(),
        include_subdomains: false,
        schemes: vec!["http".to_string()],
        ports: vec![port],
        path_prefixes: Vec::new(),
    });
    for host in extra_hosts {
        config.allowlist.push(AllowlistRule {
            host: (*host).to_string(),
            include_subdomains: false,
            schemes: vec!["http".to_string()],
            ports: vec![port],
            path_prefixes: Vec::new(),
        });
    }
    if !auth_none {
        config.auth.mode = alps::config::AuthMode::Bearer;
    }
    if auth_none {
        config.validate(None).unwrap();
    } else {
        config.validate(Some("0123456789abcdef")).unwrap();
    }
    config
}

pub fn state_for(
    config: Config,
    resolver: Arc<SecureResolver>,
    token: Option<String>,
) -> Arc<AppState> {
    state_for_signing(config, resolver, token, Vec::new())
}

pub fn state_for_signing(
    config: Config,
    resolver: Arc<SecureResolver>,
    token: Option<String>,
    signing_secrets: Vec<Vec<u8>>,
) -> Arc<AppState> {
    let loaded = LoadedConfig {
        config,
        api_token: token,
        signing_secrets,
    };
    AppState::build_with_resolver(&loaded, resolver).unwrap()
}

pub fn proxy_uri(target: &str) -> String {
    format!(
        "/v1/proxy?url={}",
        utf8_percent_encode(target, NON_ALPHANUMERIC)
    )
}

pub fn handle_for(state: Arc<AppState>) -> Arc<AppHandle> {
    AppHandle::new(state)
}

pub async fn send(
    state: Arc<AppState>,
    method: Method,
    uri: String,
    headers: &[(&str, &str)],
) -> Response {
    send_handle(handle_for(state), method, uri, headers).await
}

pub async fn send_handle(
    handle: Arc<AppHandle>,
    method: Method,
    uri: String,
    headers: &[(&str, &str)],
) -> Response {
    send_body(handle, method, uri, headers, Body::empty(), true).await
}

pub async fn send_no_peer(
    state: Arc<AppState>,
    method: Method,
    uri: String,
    headers: &[(&str, &str)],
) -> Response {
    send_body(
        handle_for(state),
        method,
        uri,
        headers,
        Body::empty(),
        false,
    )
    .await
}

pub async fn send_json(
    state: Arc<AppState>,
    uri: String,
    headers: &[(&str, &str)],
    json: &str,
) -> Response {
    let mut all = headers.to_vec();
    all.push(("content-type", "application/json"));
    send_body(
        handle_for(state),
        Method::POST,
        uri,
        &all,
        Body::from(json.to_string()),
        true,
    )
    .await
}

async fn send_body(
    handle: Arc<AppHandle>,
    method: Method,
    uri: String,
    headers: &[(&str, &str)],
    body: Body,
    with_peer: bool,
) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, HeaderValue::from_str(value).unwrap());
    }
    let service = <Router as ServiceExt<Request<Body>>>::map_response(
        routes::router(handle),
        routes::strip_bodyless_lengths,
    );
    let mut request = builder.body(body).unwrap();
    if with_peer {
        request
            .extensions_mut()
            .insert(ClientAddr(SocketAddr::from(([127, 0, 0, 1], 12345))));
    }
    service.oneshot(request).await.unwrap()
}

pub async fn send_admin(state: Arc<AppState>, uri: String) -> Response {
    let service = <Router as ServiceExt<Request<Body>>>::map_response(
        routes::metrics_router(handle_for(state)),
        routes::strip_bodyless_lengths,
    );
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ClientAddr(SocketAddr::from(([127, 0, 0, 1], 12345))));
    service.oneshot(request).await.unwrap()
}

pub async fn body_bytes(response: Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}
