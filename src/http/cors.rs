//! Optional CORS support for explicitly trusted origins.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::app::AppHandle;

const EXPOSE_HEADERS: &str =
    "Accept-Ranges, Content-Length, Content-Range, ETag, Last-Modified, Content-Disposition";

/// Add CORS headers for allowlisted origins and answer proxy preflights.
///
/// Preflight requests are answered before authentication because browsers do
/// not attach credentials to them. Only origins explicitly listed in the
/// configuration receive headers; everything else behaves as before.
pub async fn layer(State(handle): State<Arc<AppHandle>>, request: Request, next: Next) -> Response {
    let state = handle.current();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(canonical_origin);
    let allowed = origin
        .as_deref()
        .filter(|origin| state.config.cors.allows(origin))
        .map(str::to_string);

    let path = request.uri().path();
    let is_preflight = path == "/v1/proxy" || path == "/v1/sign" || path.starts_with("/p/");
    if request.method() == Method::OPTIONS
        && is_preflight
        && let Some(origin) = allowed
    {
        return preflight(&origin, path, state.config.cors.max_age_secs);
    }

    let mut response = next.run(request).await;
    if let Some(origin) = allowed {
        apply(&mut response, &origin);
    }
    response
}

fn canonical_origin(value: &str) -> Option<String> {
    if value == "null" {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn preflight(origin: &str, path: &str, max_age_secs: u64) -> Response {
    let Ok(origin) = HeaderValue::from_str(origin) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let (methods, allow_headers) = if path == "/v1/sign" {
        ("POST", "Authorization, Content-Type")
    } else {
        (
            "GET, HEAD",
            "Authorization, Range, If-Range, If-None-Match, If-Modified-Since",
        )
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(methods),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(allow_headers),
    );
    if let Ok(value) = HeaderValue::from_str(&max_age_secs.to_string()) {
        headers.insert(header::ACCESS_CONTROL_MAX_AGE, value);
    }
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    response
}

fn apply(response: &mut Response, origin: &str) {
    let Ok(origin) = HeaderValue::from_str(origin) else {
        return;
    };
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(EXPOSE_HEADERS),
    );
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
}
