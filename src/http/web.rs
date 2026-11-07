//! Optional static frontend serving for the public download page.
//!
//! The static tree is administrator-controlled. `ServeDir` performs path
//! resolution and range/conditional handling; this module only adds the
//! security and cache headers that the proxy response path intentionally does
//! not inject.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Response, StatusCode, header};

const DOCUMENT_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=()";

/// Canonical static directory captured when the web router is built.
#[derive(Clone)]
pub struct StaticRoot(pub Arc<PathBuf>);

/// Apply browser security headers and a small cache policy to static files.
pub fn harden_static_response<B>(mut response: Response<B>) -> Response<B> {
    let status = response.status();
    let headers = response.headers_mut();

    let is_html = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
        });

    insert(headers, "x-content-type-options", "nosniff");
    insert(headers, "referrer-policy", "no-referrer");
    insert(headers, "x-frame-options", "DENY");
    insert(headers, "cross-origin-opener-policy", "same-origin");
    insert(headers, "cross-origin-resource-policy", "same-origin");
    insert(headers, "x-robots-tag", "noindex, nofollow");

    if is_html {
        insert(headers, "content-security-policy", DOCUMENT_CSP);
        insert(headers, "permissions-policy", PERMISSIONS_POLICY);
    }

    if status.is_success() {
        let cache = if is_html {
            "no-cache"
        } else {
            "public, max-age=3600"
        };
        insert(headers, "cache-control", cache);
    }

    response
}

/// Serve the SPA entry point from the canonical static directory.
pub async fn serve_index(root: &Path, is_head: bool) -> Response<Body> {
    let path = root.join("index.html");
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, bytes.len())
                .body(Body::from(bytes))
                .expect("valid response");
            if is_head {
                *response.body_mut() = Body::empty();
            }
            harden_static_response(response)
        }
        Err(error) => {
            tracing::error!(error = %error, path = %path.display(), "failed to read web index");
            harden_static_response(
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(Body::from("web UI unavailable"))
                    .expect("valid response"),
            )
        }
    }
}

fn insert(headers: &mut axum::http::HeaderMap, name: &'static str, value: &'static str) {
    headers.insert(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_gets_csp_and_no_cache() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::empty())
            .unwrap();
        let response = harden_static_response(response);
        assert_eq!(response.headers()["content-security-policy"], DOCUMENT_CSP);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }

    #[test]
    fn assets_are_cacheable_but_not_html() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/javascript")
            .body(Body::empty())
            .unwrap();
        let response = harden_static_response(response);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=3600"
        );
        assert!(!response.headers().contains_key("content-security-policy"));
    }

    #[test]
    fn errors_do_not_get_a_cache_policy() {
        let response = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::empty())
            .unwrap();
        let response = harden_static_response(response);
        assert!(!response.headers().contains_key(header::CACHE_CONTROL));
        assert_eq!(response.headers()["x-frame-options"], "DENY");
    }
}
