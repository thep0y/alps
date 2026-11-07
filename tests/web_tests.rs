mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use alps::app::AppState;
use alps::config::Config;
use axum::http::{Method, StatusCode, header};

const TOKEN: &str = "0123456789abcdef";

struct WebFixture {
    dir: PathBuf,
}

impl WebFixture {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let index = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "alps-web-router-test-{}-{index}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><html><body><div id=\"root\">ui</div></body></html>",
        )
        .unwrap();
        std::fs::write(dir.join("assets/app.js"), "console.log('ok');").unwrap();
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for WebFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn config_with_web(port: u16, dir: &Path, auth_none: bool, expose_api: bool) -> Config {
    let mut config = common::config_for(port, &[], auth_none);
    config.web.dir = Some(dir.to_path_buf());
    config.web.expose_api = expose_api;
    if auth_none {
        config.validate(None).unwrap();
    } else if expose_api {
        config.validate(Some(TOKEN)).unwrap();
    } else {
        // The API is hidden, so bearer mode does not require a token.
        config.validate(None).unwrap();
    }
    config
}

fn state_for_web(
    port: u16,
    dir: &Path,
    auth_none: bool,
    expose_api: bool,
    token: Option<&str>,
) -> Arc<AppState> {
    common::state_for(
        config_with_web(port, dir, auth_none, expose_api),
        common::resolver_for(&[("upstream.test", port)]),
        token.map(str::to_string),
    )
}

#[tokio::test]
async fn web_ui_and_assets_are_served() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), true, false, None);
    let handle = common::handle_for(state);

    let index = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        "/".to_string(),
        &[],
    )
    .await;
    assert_eq!(index.status(), StatusCode::OK);
    assert_eq!(
        index.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert!(index.headers().contains_key("content-security-policy"));
    assert_eq!(index.headers()["x-content-type-options"], "nosniff");
    assert_eq!(index.headers()[header::CACHE_CONTROL], "no-cache");
    let body = common::body_bytes(index).await;
    assert!(body.windows(9).any(|window| window == b"id=\"root\""));

    let head = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::HEAD,
        "/".to_string(),
        &[],
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH].to_str().unwrap(),
        body.len().to_string()
    );
    assert!(common::body_bytes(head).await.is_empty());

    let asset = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        "/assets/app.js".to_string(),
        &[],
    )
    .await;
    assert_eq!(asset.status(), StatusCode::OK);
    assert!(
        asset.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(
        asset.headers()[header::CACHE_CONTROL],
        "public, max-age=3600"
    );

    let missing = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        "/missing".to_string(),
        &[],
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn web_api_is_hidden_by_default() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), false, false, None);
    let handle = common::handle_for(Arc::clone(&state));
    let target = format!("http://upstream.test:{port}/file");

    let public = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        common::public_proxy_uri(&target),
        &[],
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(public).await, common::FILE_BODY);

    for uri in [
        common::proxy_uri(&target),
        "/p/token".to_string(),
        "/metrics".to_string(),
        "/v1/sign".to_string(),
    ] {
        let response = common::send_web(
            Arc::clone(&handle),
            fixture.path(),
            Method::GET,
            uri.clone(),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn web_public_proxy_bypasses_bearer_auth() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), false, true, Some(TOKEN));
    let handle = common::handle_for(state);
    let target = format!("http://upstream.test:{port}/file");

    let public = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        common::public_proxy_uri(&target),
        &[],
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(public.headers()[header::CACHE_CONTROL], "private, no-store");
    assert_eq!(public.headers()["x-robots-tag"], "noindex, nofollow");
    assert_eq!(public.headers()["x-content-type-options"], "nosniff");
    assert_eq!(public.headers()["referrer-policy"], "no-referrer");
    assert_eq!(common::body_bytes(public).await, common::FILE_BODY);

    let api_without_token = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        common::proxy_uri(&target),
        &[],
    )
    .await;
    assert_eq!(api_without_token.status(), StatusCode::UNAUTHORIZED);

    let authorization = format!("Bearer {TOKEN}");
    let api_with_token = common::send_web(
        Arc::clone(&handle),
        fixture.path(),
        Method::GET,
        common::proxy_uri(&target),
        &[("authorization", authorization.as_str())],
    )
    .await;
    assert_eq!(api_with_token.status(), StatusCode::OK);
}

#[tokio::test]
async fn web_public_proxy_forwards_range_requests() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), true, false, None);
    let handle = common::handle_for(state);
    let target = format!("http://upstream.test:{port}/file");

    let response = common::send_web(
        handle,
        fixture.path(),
        Method::GET,
        common::public_proxy_uri(&target),
        &[("range", "bytes=0-3")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 0-3/16");
    assert_eq!(common::body_bytes(response).await, b"0123");
}

#[tokio::test]
async fn web_public_query_requires_a_single_url() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), true, false, None);
    let handle = common::handle_for(state);

    for uri in [
        "/?foo=bar".to_string(),
        "/?url=".to_string(),
        "/?url=a&url=b".to_string(),
        "/?url=javascript%3Aalert(1)".to_string(),
    ] {
        let response = common::send_web(
            Arc::clone(&handle),
            fixture.path(),
            Method::GET,
            uri.clone(),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert!(response.headers().contains_key("x-error-code"), "{uri}");
    }
}

#[tokio::test]
async fn web_rejects_unsupported_methods_on_the_page() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), true, false, None);
    let handle = common::handle_for(state);

    let response =
        common::send_web(handle, fixture.path(), Method::POST, "/".to_string(), &[]).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
}

#[tokio::test]
async fn web_static_serving_does_not_escape_the_directory() {
    let fixture = WebFixture::new();
    let port = common::start_upstream().await;
    let state = state_for_web(port, fixture.path(), true, false, None);
    let handle = common::handle_for(state);

    let response = common::send_web(
        handle,
        fixture.path(),
        Method::GET,
        "/%2e%2e/%2e%2e/Cargo.toml".to_string(),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn web_directory_cannot_change_on_reload() {
    let first = WebFixture::new();
    let second = WebFixture::new();
    let root = std::env::temp_dir().join(format!("alps-web-reload-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let write_config = |name: &str, dir: &Path| {
        let path = root.join(name);
        std::fs::write(
            &path,
            format!(
                "[server]\nlisten = \"127.0.0.1:0\"\n[auth]\nmode = \"none\"\n[web]\ndir = \"{}\"\n",
                dir.display()
            ),
        )
        .unwrap();
        path
    };

    let first_config = write_config("first.toml", first.path());
    let second_config = write_config("second.toml", second.path());

    let result = alps::app::reload_from_path_with(&first_config, None, Some(first.path()));
    assert!(result.is_ok());

    let result = alps::app::reload_from_path_with(&second_config, None, Some(first.path()));
    assert!(matches!(result, Err(alps::app::ReloadError::WebDirChanged)));

    let _ = std::fs::remove_dir_all(&root);
}
