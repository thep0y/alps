//! HTTP routes: health checks, the proxy endpoint and signed links.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use serde::Deserialize;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::app::{AppHandle, AppState};
use crate::auth::AuthError;
use crate::http::errors::{ProxyError, error_response};
use crate::http::headers::{self, ForwardHeaders};
use crate::http::query;
use crate::http::web::{self, StaticRoot};
use crate::limits::StreamPermits;
use crate::policy::ValidatedTarget;
use crate::server::ClientAddr;
use crate::signing::{SigningError, unix_now};
use crate::telemetry;
use crate::upstream::redirects::UpstreamResponse;
use crate::upstream::stream::{self, StreamObserver};

pub fn router(handle: Arc<AppHandle>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/proxy", any(proxy))
        .route("/v1/sign", any(sign))
        .route("/p/{token}", any(signed_proxy))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            handle.clone(),
            crate::http::cors::layer,
        ))
        .with_state(handle)
}

/// Public web router: the static UI plus the unauthenticated `/?url=` route.
///
/// The JSON API handlers stay mounted but are hidden by the CORS/API guard
/// while `web.expose_api = false`; hidden paths return 404 before auth or a
/// preflight. The static directory is a startup setting; `expose_api` follows
/// configuration reloads.
pub fn web_router(handle: Arc<AppHandle>, static_dir: PathBuf) -> Router {
    let root = Arc::new(static_dir);

    // Keep the JSON API handlers mounted even when hidden. The CORS layer
    // returns 404 for them while web.expose_api is false, so a SIGHUP toggle
    // cannot leave an unauthenticated route behind after auth is disabled.
    let router = Router::new()
        .route("/", any(web_index))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/proxy", any(proxy))
        .route("/v1/sign", any(sign))
        .route("/p/{token}", any(signed_proxy));

    let serve_dir = ServeDir::new(root.as_path());
    let static_service = tower::service_fn(move |request: axum::http::Request<Body>| {
        let serve_dir = serve_dir.clone();
        async move {
            let response = serve_dir
                .oneshot(request)
                .await
                .expect("static file service is infallible");
            Ok::<_, std::convert::Infallible>(web::harden_static_response(response))
        }
    });

    router
        .fallback_service(static_service)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            handle.clone(),
            crate::http::cors::layer,
        ))
        .layer(Extension(StaticRoot(root)))
        .with_state(handle)
}

/// Admin listener router: metrics plus the health endpoints.
pub fn metrics_router(handle: Arc<AppHandle>) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(handle)
}

async fn metrics(State(handle): State<Arc<AppHandle>>) -> Response {
    let state = handle.current();
    let metrics = crate::metrics::global();
    metrics.set_active_streams(state.limits.active_streams());
    match metrics.encode() {
        Ok(body) => {
            let mut response = (
                [(
                    axum::http::header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
                )],
                body,
            )
                .into_response();
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "metrics unavailable").into_response(),
    }
}

/// axum inserts Content-Length: 0 for exact-size empty bodies, including 204
/// and 304. Those statuses must not carry a zero length, so strip it again.
/// Apply this outside the Router service; the route future sets the header
/// after any Router-level middleware has run.
pub fn strip_bodyless_lengths(mut response: Response) -> Response {
    if matches!(
        response.status(),
        StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED
    ) {
        response
            .headers_mut()
            .remove(axum::http::header::CONTENT_LENGTH);
    }
    response
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn readyz(State(handle): State<Arc<AppHandle>>) -> Response {
    let state = handle.current();
    Json(serde_json::json!({
        "status": "ready",
        "allowlist_rules": state.policy.allowlist_len(),
        "redirect_allowlist_rules": state.policy.redirect_allowlist_len(),
    }))
    .into_response()
}

async fn proxy(
    State(handle): State<Arc<AppHandle>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    peer: Option<Extension<ClientAddr>>,
) -> Response {
    proxy_inner(handle, method, uri, headers, peer, true).await
}

/// Public UI entry point. With a query string the request is handled by the
/// unauthenticated proxy path; otherwise the SPA entry point is returned.
async fn web_index(
    State(handle): State<Arc<AppHandle>>,
    Extension(root): Extension<StaticRoot>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    peer: Option<Extension<ClientAddr>>,
) -> Response {
    if uri.query().is_some() {
        let mut response = proxy_inner(handle, method, uri, headers, peer, false).await;
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
        for (name, value) in [
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("x-frame-options", "DENY"),
            ("cross-origin-resource-policy", "same-origin"),
            ("x-robots-tag", "noindex, nofollow"),
        ] {
            response.headers_mut().insert(
                axum::http::header::HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        return response;
    }

    if method != Method::GET && method != Method::HEAD {
        let request_id = telemetry::request_id(&headers);
        let started = Instant::now();
        return reject(
            ProxyError::MethodNotAllowed,
            &request_id,
            &method,
            started,
            false,
        );
    }

    web::serve_index(root.0.as_path(), method == Method::HEAD).await
}

async fn proxy_inner(
    handle: Arc<AppHandle>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    peer: Option<Extension<ClientAddr>>,
    require_auth: bool,
) -> Response {
    let state = handle.current();
    let request_id = telemetry::request_id(&headers);
    let started = Instant::now();
    let is_head = method == Method::HEAD;
    let client_ip = resolve_client_ip(&state, &headers, peer);

    if method != Method::GET && method != Method::HEAD {
        return reject(
            ProxyError::MethodNotAllowed,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    if require_auth && let Err(error) = state.auth.check(&headers) {
        let error = match error {
            AuthError::Misconfigured => ProxyError::Internal,
            AuthError::Missing | AuthError::Invalid => ProxyError::Unauthorized,
        };
        return reject(error, &request_id, &method, started, is_head);
    }
    if headers::has_request_body(&headers) {
        return reject(
            ProxyError::RequestBodyNotAllowed,
            &request_id,
            &method,
            started,
            is_head,
        );
    }

    let raw_url = match query::extract_target_url(&uri, state.config.proxy.max_url_length) {
        Ok(url) => url,
        Err(error) => return reject(error, &request_id, &method, started, is_head),
    };
    let target = match state.policy.validate(&raw_url) {
        Ok(target) => target,
        Err(error) => return reject(error.into(), &request_id, &method, started, is_head),
    };
    let forward = headers::forward_request(&headers, state.config.proxy.forward_accept);

    serve_target(ServeContext {
        state: state.as_ref(),
        request_id: &request_id,
        method,
        target,
        forward,
        limit_override: None,
        started,
        client_ip,
    })
    .await
}

async fn signed_proxy(
    State(handle): State<Arc<AppHandle>>,
    method: Method,
    headers: HeaderMap,
    Path(token): Path<String>,
    peer: Option<Extension<ClientAddr>>,
) -> Response {
    let state = handle.current();
    let request_id = telemetry::request_id(&headers);
    let started = Instant::now();
    let is_head = method == Method::HEAD;
    let client_ip = resolve_client_ip(&state, &headers, peer);

    if method != Method::GET && method != Method::HEAD {
        return reject(
            ProxyError::MethodNotAllowed,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    if headers::has_request_body(&headers) {
        return reject(
            ProxyError::RequestBodyNotAllowed,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    let verified = match state.signer.verify(&token) {
        Ok(verified) => verified,
        Err(error) => {
            return reject(verify_error(error), &request_id, &method, started, is_head);
        }
    };
    let target = match state.policy.validate(&verified.url) {
        Ok(target) => target,
        Err(error) => return reject(error.into(), &request_id, &method, started, is_head),
    };
    let forward = headers::forward_request(&headers, state.config.proxy.forward_accept);

    serve_target(ServeContext {
        state: state.as_ref(),
        request_id: &request_id,
        method,
        target,
        forward,
        limit_override: verified.max_bytes,
        started,
        client_ip,
    })
    .await
}

#[derive(Debug, Deserialize)]
struct SignRequest {
    url: String,
    #[serde(default)]
    expires_in_secs: Option<u64>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

async fn sign(
    State(handle): State<Arc<AppHandle>>,
    method: Method,
    headers: HeaderMap,
    peer: Option<Extension<ClientAddr>>,
    payload: Result<Json<SignRequest>, JsonRejection>,
) -> Response {
    let state = handle.current();
    let request_id = telemetry::request_id(&headers);
    let started = Instant::now();
    let is_head = false;
    let client_ip = resolve_client_ip(&state, &headers, peer);

    if method != Method::POST {
        return reject(
            ProxyError::MethodNotAllowed,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    if let Err(error) = state.auth.check(&headers) {
        let error = match error {
            AuthError::Misconfigured => ProxyError::Internal,
            AuthError::Missing | AuthError::Invalid => ProxyError::Unauthorized,
        };
        return reject(error, &request_id, &method, started, is_head);
    }
    if !state.limits.check_rate_for(client_ip) {
        return reject(
            ProxyError::RateLimited,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    if !state.signer.enabled() {
        return reject(
            ProxyError::SigningDisabled,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return reject(
                ProxyError::InvalidRequest,
                &request_id,
                &method,
                started,
                is_head,
            );
        }
    };
    let target = match state.policy.validate(&request.url) {
        Ok(target) => target,
        Err(error) => return reject(error.into(), &request_id, &method, started, is_head),
    };
    let ttl = request
        .expires_in_secs
        .unwrap_or_else(|| state.signer.default_ttl());
    if ttl == 0 || ttl > state.signer.max_ttl() {
        return reject(
            ProxyError::InvalidRequest,
            &request_id,
            &method,
            started,
            is_head,
        );
    }
    let max_bytes = match request.max_bytes {
        Some(value) if value > 0 => {
            let configured = state.config.proxy.max_response_bytes;
            if configured > 0 && value > configured {
                return reject(
                    ProxyError::InvalidRequest,
                    &request_id,
                    &method,
                    started,
                    is_head,
                );
            }
            Some(value)
        }
        _ => None,
    };
    let token = match state.signer.sign(target.url().as_str(), ttl, max_bytes) {
        Ok(token) => token,
        Err(error) => {
            return reject(sign_error(error), &request_id, &method, started, is_head);
        }
    };
    let expires_at = state
        .signer
        .verify(&token)
        .map(|verified| verified.expires_at)
        .unwrap_or_else(|_| unix_now().saturating_add(ttl));

    tracing::info!(
        request_id = %request_id,
        host = target.host(),
        ttl_secs = ttl,
        "signed link issued"
    );

    crate::metrics::global().record_request(StatusCode::OK.as_u16());
    let mut response = Json(serde_json::json!({
        "token": token,
        "expires_at": expires_at,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(telemetry::REQUEST_ID_HEADER, value);
    }
    response
}

struct ServeContext<'a> {
    state: &'a AppState,
    request_id: &'a str,
    method: Method,
    target: ValidatedTarget,
    forward: ForwardHeaders,
    limit_override: Option<u64>,
    started: Instant,
    client_ip: IpAddr,
}

async fn serve_target(context: ServeContext<'_>) -> Response {
    let ServeContext {
        state,
        request_id,
        method,
        target,
        forward,
        limit_override,
        started,
        client_ip,
    } = context;
    let is_head = method == Method::HEAD;

    if !state.limits.check_rate_for(client_ip) {
        return reject(
            ProxyError::RateLimited,
            request_id,
            &method,
            started,
            is_head,
        );
    }
    let global = match state.limits.acquire_global() {
        Ok(permit) => permit,
        Err(_) => {
            return reject(
                ProxyError::CapacityExceeded,
                request_id,
                &method,
                started,
                is_head,
            );
        }
    };

    let fetch_started = Instant::now();
    let fetched = state.redirects.fetch(target, method.clone(), forward).await;
    crate::metrics::global().observe_upstream(fetch_started.elapsed().as_secs_f64());
    let upstream = match fetched {
        Ok(upstream) => upstream,
        Err(error) => {
            drop(global);
            return reject(error, request_id, &method, started, is_head);
        }
    };

    let UpstreamResponse {
        status,
        headers: upstream_headers,
        body: upstream_body,
        hops,
        host,
        host_permit,
    } = upstream;

    let config_limit = state.config.proxy.max_response_bytes;
    let limit = match (config_limit, limit_override) {
        (0, None) => 0,
        (0, Some(value)) => value,
        (configured, None) => configured,
        (configured, Some(value)) => configured.min(value),
    };
    let too_large = limit > 0
        && upstream_body
            .content_length()
            .is_some_and(|length| length > limit);
    if too_large {
        drop(host_permit);
        return reject(
            ProxyError::UpstreamResponseTooLarge,
            request_id,
            &method,
            started,
            is_head,
        );
    }

    let mut response_headers = headers::filter_response(&upstream_headers, status);
    let observer = StreamObserver::new(
        request_id.to_string(),
        method.as_str().to_string(),
        host,
        status.as_u16(),
        hops,
        started,
    );

    let body = if is_head || status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED
    {
        let mut observer = observer;
        observer.finish("complete");
        drop(upstream_body);
        drop(host_permit);
        drop(global);
        Body::empty()
    } else {
        let permits = StreamPermits::new(global, host_permit);
        stream::body_from_upstream(
            upstream_body,
            permits,
            limit,
            observer,
            state.limits.egress(),
        )
    };

    let mut response = Response::builder()
        .status(status)
        .body(body)
        .expect("valid response");
    *response.headers_mut() = std::mem::take(&mut response_headers);
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response
            .headers_mut()
            .insert(telemetry::REQUEST_ID_HEADER, value);
    }
    response
}

fn verify_error(error: SigningError) -> ProxyError {
    match error {
        SigningError::Disabled => ProxyError::SigningDisabled,
        SigningError::Expired => ProxyError::TokenExpired,
        _ => ProxyError::InvalidToken,
    }
}

fn sign_error(error: SigningError) -> ProxyError {
    match error {
        SigningError::Disabled => ProxyError::SigningDisabled,
        SigningError::TooLong | SigningError::InvalidTtl => ProxyError::InvalidRequest,
        _ => ProxyError::Internal,
    }
}

fn resolve_client_ip(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<Extension<ClientAddr>>,
) -> IpAddr {
    let peer_ip = peer
        .map(|Extension(addr)| addr.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    state.limits.client_ip(peer_ip, forwarded)
}

fn reject(
    error: ProxyError,
    request_id: &str,
    method: &Method,
    started: Instant,
    is_head: bool,
) -> Response {
    let metrics = crate::metrics::global();
    metrics.record_request(error.status().as_u16());
    if error.status() == StatusCode::FORBIDDEN {
        metrics.record_policy_denied(error.code());
    }
    tracing::info!(
        request_id = %request_id,
        method = %method,
        status = error.status().as_u16(),
        error_code = error.code(),
        duration_ms = started.elapsed().as_millis() as u64,
        outcome = "rejected",
        "request rejected"
    );
    error_response(&error, request_id, is_head)
}
