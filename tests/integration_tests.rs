mod common;

use axum::http::{Method, StatusCode, header};

fn target(port: u16, path: &str) -> String {
    format!("http://upstream.test:{port}{path}")
}

#[tokio::test]
async fn get_proxies_file_with_security_headers() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "16");
    assert!(!response.headers().contains_key("x-content-type-options"));
    assert!(
        !response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"file.bin\""
    );
    assert!(response.headers().contains_key("x-request-id"));
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    assert_eq!(common::body_bytes(response).await, common::FILE_BODY);
}

#[tokio::test]
async fn head_has_metadata_but_no_body() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::HEAD,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "16");
    assert!(common::body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn range_request_returns_partial_content() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[("range", "bytes=0-3")],
    )
    .await;

    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 0-3/16");
    assert_eq!(common::body_bytes(response).await, b"0123");
}

#[tokio::test]
async fn unsatisfiable_range_passes_through() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[("range", "bytes=99-")],
    )
    .await;

    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */16");
}

#[tokio::test]
async fn conditional_request_returns_not_modified() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/not-modified")),
        &[("if-none-match", "\"v1\"")],
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(
        !response.headers().contains_key(header::CONTENT_LENGTH),
        "unexpected headers: {:?}",
        response.headers()
    );
    assert!(common::body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn follows_relative_redirect() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/redirect")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(response).await, common::FILE_BODY);
}

#[tokio::test]
async fn rejects_redirect_to_unlisted_host() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/redirect-external")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = common::body_bytes(response).await;
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("destination_denied"));
}

#[tokio::test]
async fn follows_redirect_to_redirect_allowlisted_host() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    let mut redirect = config.allowlist[0].clone();
    redirect.host = "redirect.test".into();
    config.redirect_allowlist = vec![redirect];
    config.validate(None).unwrap();

    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port), ("redirect.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/redirect-to-redirect-host")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(response).await, common::FILE_BODY);
}

#[tokio::test]
async fn redirect_allowlist_target_is_not_an_initial_target() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    let mut redirect = config.allowlist[0].clone();
    redirect.host = "redirect.test".into();
    config.redirect_allowlist = vec![redirect];
    config.validate(None).unwrap();

    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port), ("redirect.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&format!("http://redirect.test:{port}/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = common::body_bytes(response).await;
    assert!(String::from_utf8_lossy(&body).contains("destination_denied"));
}

#[tokio::test]
async fn rejects_unlisted_host() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&format!("http://evil.test:{port}/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn rejects_unsupported_method() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::POST,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
}

#[tokio::test]
async fn bearer_auth_is_enforced() {
    let port = common::start_upstream().await;
    let resolver = common::resolver_for(&[("upstream.test", port)]);
    let state = common::state_for(
        common::config_for(port, &[], false),
        resolver,
        Some("0123456789abcdef".to_string()),
    );
    let uri = common::proxy_uri(&target(port, "/file"));

    let unauthorized = common::send(state.clone(), Method::GET, uri.clone(), &[]).await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = common::send(
        state,
        Method::GET,
        uri,
        &[("authorization", "Bearer 0123456789abcdef")],
    )
    .await;
    assert_eq!(authorized.status(), StatusCode::OK);
}

#[tokio::test]
async fn strict_policy_rejects_loopback_resolution() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::strict_resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(String::from_utf8_lossy(&common::body_bytes(response).await).contains("ip_denied"));
}

#[tokio::test]
async fn does_not_forward_client_credentials() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/inspect")),
        &[
            ("cookie", "session=secret"),
            ("authorization", "Bearer secret"),
            ("accept-encoding", "gzip"),
        ],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = common::body_bytes(response).await;
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["cookie"].is_null());
    assert!(value["authorization"].is_null());
    assert_eq!(value["accept_encoding"], "identity");
}

#[tokio::test]
async fn enforces_max_response_bytes_for_known_length() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.proxy.max_response_bytes = 1024;
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/large")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await)
            .contains("upstream_response_too_large")
    );
}

#[tokio::test]
async fn upstream_404_is_passed_through() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/missing")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn end_to_end_over_tcp_and_graceful_shutdown() {
    let port = common::start_upstream().await;
    let config = common::config_for(port, &[], true);
    let state = common::state_for(
        config.clone(),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(alps::server::serve(
        listener,
        alps::http::routes::router(common::handle_for(state)),
        std::sync::Arc::new(config),
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let target = format!("http://upstream.test:{port}/file");
    let url = format!("http://{address}{}", common::proxy_uri(&target));

    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "16");
    assert_eq!(response.bytes().await.unwrap().as_ref(), common::FILE_BODY);

    let head = client.head(&url).send().await.unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "16");
    assert!(head.bytes().await.unwrap().is_empty());

    let range = client
        .get(&url)
        .header("range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(range.status(), 206);
    assert_eq!(range.headers()[header::CONTENT_RANGE], "bytes 0-3/16");
    assert_eq!(range.bytes().await.unwrap().as_ref(), b"0123");

    let conditional = format!(
        "http://{address}{}",
        common::proxy_uri(&format!("http://upstream.test:{port}/not-modified"))
    );
    let not_modified = client
        .get(conditional)
        .header("if-none-match", "\"v1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(not_modified.status(), 304);
    assert!(
        !not_modified.headers().contains_key(header::CONTENT_LENGTH),
        "unexpected headers: {:?}",
        not_modified.headers()
    );
    assert!(not_modified.bytes().await.unwrap().is_empty());

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn passes_content_encoding_through_without_decompression() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/encoded")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    assert_eq!(common::body_bytes(response).await, b"abc");
}

#[tokio::test]
async fn detects_redirect_loops() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/loop")),
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await)
            .contains("upstream_redirect_error")
    );
}

const SIGNING_SECRET: [u8; 32] = [7u8; 32];

fn signing_config(port: u16) -> alps::config::Config {
    let mut config = common::config_for(port, &[], false);
    config.signing.enabled = true;
    config.signing.default_ttl_secs = 60;
    config.signing.max_ttl_secs = 3600;
    config
}

fn signing_state(port: u16) -> std::sync::Arc<alps::app::AppState> {
    common::state_for_signing(
        signing_config(port),
        common::resolver_for(&[("upstream.test", port)]),
        Some("0123456789abcdef".to_string()),
        vec![SIGNING_SECRET.to_vec()],
    )
}

const AUTH: (&str, &str) = ("authorization", "Bearer 0123456789abcdef");

#[tokio::test]
async fn sign_then_fetch_signed_link() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = target(port, "/file");
    let body = serde_json::json!({"url": target, "expires_in_secs": 60}).to_string();
    let response = common::send_json(state.clone(), "/v1/sign".to_string(), &[AUTH], &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value =
        serde_json::from_slice(&common::body_bytes(response).await).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();
    assert!(payload["expires_at"].as_u64().unwrap() > 0);

    let signed = common::send(state, Method::GET, format!("/p/{token}"), &[]).await;
    assert_eq!(signed.status(), StatusCode::OK);
    assert_eq!(common::body_bytes(signed).await, common::FILE_BODY);
}

#[tokio::test]
async fn sign_endpoint_requires_bearer() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = target(port, "/file");
    let body = serde_json::json!({"url": target}).to_string();
    let response = common::send_json(state, "/v1/sign".to_string(), &[], &body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tampered_signed_token_is_rejected() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = format!("http://upstream.test:{port}/file");
    let token = state.signer.sign(&target, 60, None).unwrap();
    let mut chars: Vec<char> = token.chars().collect();
    let last = chars.len() - 1;
    chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
    let tampered: String = chars.into_iter().collect();
    let response = common::send(state, Method::GET, format!("/p/{tampered}"), &[]).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(String::from_utf8_lossy(&common::body_bytes(response).await).contains("invalid_token"));
}

#[tokio::test]
async fn expired_signed_token_is_rejected() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = format!("http://upstream.test:{port}/file");
    let token = state.signer.sign_at(&target, 60, None, 1_000).unwrap();
    let response = common::send(state, Method::GET, format!("/p/{token}"), &[]).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(String::from_utf8_lossy(&common::body_bytes(response).await).contains("token_expired"));
}

#[tokio::test]
async fn signed_link_still_enforces_allowlist() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = format!("http://evil.test:{port}/file");
    let token = state.signer.sign(&target, 60, None).unwrap();
    let response = common::send(state, Method::GET, format!("/p/{token}"), &[]).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await).contains("destination_denied")
    );
}

#[tokio::test]
async fn signed_link_max_bytes_is_enforced() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let target = format!("http://upstream.test:{port}/file");
    let token = state.signer.sign(&target, 60, Some(8)).unwrap();
    let response = common::send(state, Method::GET, format!("/p/{token}"), &[]).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await)
            .contains("upstream_response_too_large")
    );
}

#[tokio::test]
async fn sign_is_disabled_when_not_configured() {
    let port = common::start_upstream().await;
    let config = common::config_for(port, &[], false);
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        Some("0123456789abcdef".to_string()),
    );
    let target = target(port, "/file");
    let body = serde_json::json!({"url": target}).to_string();
    let response = common::send_json(state, "/v1/sign".to_string(), &[AUTH], &body).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await).contains("signing_disabled")
    );
}

#[tokio::test]
async fn sign_rejects_unlisted_url_and_bad_ttl() {
    let port = common::start_upstream().await;
    let state = signing_state(port);
    let payload = serde_json::json!({"url": format!("http://evil.test:{port}/file")}).to_string();
    let response =
        common::send_json(state.clone(), "/v1/sign".to_string(), &[AUTH], &payload).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let payload = serde_json::json!({
        "url": target(port, "/file"),
        "expires_in_secs": 999_999u64,
    })
    .to_string();
    let response = common::send_json(state, "/v1/sign".to_string(), &[AUTH], &payload).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn per_ip_rate_limit_uses_trusted_forwarded_for() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.limits.max_requests_per_second_per_ip = 1;
    config.limits.trusted_proxies = vec!["127.0.0.1/32".to_string()];
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let url = common::proxy_uri(&target(port, "/file"));
    let forwarded = [("x-forwarded-for", "203.0.113.9")];

    let first = common::send(state.clone(), Method::GET, url.clone(), &forwarded).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = common::send(state.clone(), Method::GET, url.clone(), &forwarded).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);

    let other = [("x-forwarded-for", "203.0.113.10")];
    let third = common::send(state, Method::GET, url, &other).await;
    assert_eq!(third.status(), StatusCode::OK);
}

#[tokio::test]
async fn metrics_endpoint_exposes_bounded_metrics() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let _ = common::send(
        state.clone(),
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;
    let response = common::send_admin(state, "/metrics".to_string()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.starts_with("text/plain"));
    let body = String::from_utf8(common::body_bytes(response).await).unwrap();
    assert!(body.contains("alps_requests_total"));
    assert!(body.contains("alps_active_streams"));
    assert!(body.contains("alps_upstream_duration_seconds"));
}

#[tokio::test]
async fn app_handle_swaps_the_snapshot_for_new_requests() {
    let port = common::start_upstream().await;
    let resolver = common::resolver_for(&[("upstream.test", port)]);
    let initial = common::state_for(common::config_for(port, &[], true), resolver.clone(), None);
    let handle = common::handle_for(initial);
    let url = common::proxy_uri(&target(port, "/file"));

    let allowed = common::send_handle(handle.clone(), Method::GET, url.clone(), &[]).await;
    assert_eq!(allowed.status(), StatusCode::OK);

    let mut empty = common::config_for(port, &[], true);
    empty.allowlist.clear();
    let replacement = common::state_for(empty, resolver, None);
    handle.replace(replacement);

    let denied = common::send_handle(handle, Method::GET, url, &[]).await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reload_from_path_builds_state_and_reports_bad_config() {
    let dir = std::env::temp_dir().join(format!("alps-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.toml");
    std::fs::write(
        &good,
        r#"
[server]
listen = "127.0.0.1:0"
[auth]
mode = "none"
[[allowlist]]
host = "files.example.org"
schemes = ["https"]
ports = [443]
"#,
    )
    .unwrap();
    let state = alps::app::reload_from_path(&good).unwrap();
    assert_eq!(state.policy.allowlist_len(), 1);

    let bad = dir.join("bad.toml");
    std::fs::write(&bad, "[server\nlisten = ").unwrap();
    assert!(alps::app::reload_from_path(&bad).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn cors_allows_configured_origin_and_answers_preflight() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], false);
    config.cors.allowed_origins = vec!["https://app.example.org".to_string()];
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        Some("0123456789abcdef".to_string()),
    );
    let url = common::proxy_uri(&target(port, "/file"));
    let auth = [
        ("authorization", "Bearer 0123456789abcdef"),
        ("origin", "https://app.example.org"),
    ];

    let allowed = common::send(state.clone(), Method::GET, url.clone(), &auth).await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
        allowed.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://app.example.org"
    );
    assert!(
        allowed.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS]
            .to_str()
            .unwrap()
            .contains("Content-Range")
    );
    assert!(
        allowed
            .headers()
            .get_all(header::VARY)
            .iter()
            .any(|value| value.to_str().unwrap().contains("Origin"))
    );

    let denied = common::send(
        state.clone(),
        Method::GET,
        url,
        &[
            ("authorization", "Bearer 0123456789abcdef"),
            ("origin", "https://evil.example"),
        ],
    )
    .await;
    assert_eq!(denied.status(), StatusCode::OK);
    assert!(
        !denied
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );

    let preflight = common::send(
        state.clone(),
        Method::OPTIONS,
        "/v1/proxy".to_string(),
        &[("origin", "https://app.example.org")],
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        preflight.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://app.example.org"
    );
    assert!(
        preflight.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .unwrap()
            .contains("Range")
    );

    let sign_preflight = common::send(
        state.clone(),
        Method::OPTIONS,
        "/v1/sign".to_string(),
        &[("origin", "https://app.example.org")],
    )
    .await;
    assert_eq!(sign_preflight.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        sign_preflight.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
        "POST"
    );
    assert!(
        sign_preflight.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .unwrap()
            .contains("Authorization")
    );

    let untrusted = common::send(
        state,
        Method::OPTIONS,
        "/v1/proxy".to_string(),
        &[("origin", "https://evil.example")],
    )
    .await;
    assert_eq!(untrusted.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn serves_h2c_when_enabled() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.server.http2 = true;
    let state = common::state_for(
        config.clone(),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(alps::server::serve(
        listener,
        alps::http::routes::router(common::handle_for(state)),
        std::sync::Arc::new(config),
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    let client = reqwest::Client::builder()
        .http2_prior_knowledge()
        .no_proxy()
        .build()
        .unwrap();
    let health = client
        .get(format!("http://{address}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.version(), axum::http::Version::HTTP_2);

    let proxy = client
        .get(format!(
            "http://{address}{}",
            common::proxy_uri(&target(port, "/file"))
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(proxy.status(), StatusCode::OK);
    assert_eq!(proxy.version(), axum::http::Version::HTTP_2);
    assert_eq!(proxy.bytes().await.unwrap().as_ref(), common::FILE_BODY);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn egress_limit_shapes_proxy_body() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.limits.max_egress_bytes_per_second = 64 * 1024;
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );

    let started = std::time::Instant::now();
    let response = common::send(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/large-128k")),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::body_bytes(response).await;
    assert_eq!(body.len(), 128 * 1024);
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(800),
        "elapsed {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn readyz_reports_allowlist_count() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(state, Method::GET, "/readyz".to_string(), &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(common::body_bytes(response).await).unwrap();
    assert!(body.contains("allowlist_rules"));
}

#[tokio::test]
async fn proxy_rejects_bodies_and_bad_queries() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );

    let with_body = common::send(
        state.clone(),
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[("content-length", "1")],
    )
    .await;
    assert_eq!(with_body.status(), StatusCode::BAD_REQUEST);

    let missing = common::send(state.clone(), Method::GET, "/v1/proxy".to_string(), &[]).await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let malformed = common::send(state, Method::GET, "/v1/proxy?url=%ZZ".to_string(), &[]).await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn signed_link_rejects_wrong_method_and_body() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.signing.enabled = true;
    let state = common::state_for_signing(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
        vec![vec![7u8; 32]],
    );
    let token = state.signer.sign(&target(port, "/file"), 60, None).unwrap();

    let post = common::send(state.clone(), Method::POST, format!("/p/{token}"), &[]).await;
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);

    let with_body = common::send(
        state,
        Method::GET,
        format!("/p/{token}"),
        &[("content-length", "1")],
    )
    .await;
    assert_eq!(with_body.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sign_endpoint_rejects_invalid_payloads() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.signing.enabled = true;
    config.signing.default_ttl_secs = 60;
    config.signing.max_ttl_secs = 60;
    config.proxy.max_response_bytes = 1024;
    let state = common::state_for_signing(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
        vec![vec![7u8; 32]],
    );

    let get = common::send(state.clone(), Method::GET, "/v1/sign".to_string(), &[]).await;
    assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);

    let bad_json = common::send_json(state.clone(), "/v1/sign".to_string(), &[], "{").await;
    assert_eq!(bad_json.status(), StatusCode::BAD_REQUEST);

    let ttl_zero = common::send_json(
        state.clone(),
        "/v1/sign".to_string(),
        &[],
        &format!(r#"{{"url":"http://upstream.test:{port}/file","expires_in_secs":0}}"#),
    )
    .await;
    assert_eq!(ttl_zero.status(), StatusCode::BAD_REQUEST);

    let ttl_too_large = common::send_json(
        state.clone(),
        "/v1/sign".to_string(),
        &[],
        &format!(r#"{{"url":"http://upstream.test:{port}/file","expires_in_secs":61}}"#),
    )
    .await;
    assert_eq!(ttl_too_large.status(), StatusCode::BAD_REQUEST);

    let max_bytes_too_large = common::send_json(
        state.clone(),
        "/v1/sign".to_string(),
        &[],
        &format!(r#"{{"url":"http://upstream.test:{port}/file","max_bytes":2048}}"#),
    )
    .await;
    assert_eq!(max_bytes_too_large.status(), StatusCode::BAD_REQUEST);

    let long_url = format!(
        r#"{{"url":"http://upstream.test:{port}/{}"}}"#,
        "a".repeat(5000)
    );
    let token_too_long = common::send_json(state, "/v1/sign".to_string(), &[], &long_url).await;
    assert_eq!(token_too_long.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sign_endpoint_is_rate_limited_per_ip() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.signing.enabled = true;
    config.limits.max_requests_per_second_per_ip = 1;
    let state = common::state_for_signing(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
        vec![vec![7u8; 32]],
    );
    let body = format!(r#"{{"url":"http://upstream.test:{port}/file"}}"#);
    let first = common::send_json(state.clone(), "/v1/sign".to_string(), &[], &body).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = common::send_json(state, "/v1/sign".to_string(), &[], &body).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn global_stream_capacity_is_reported() {
    let port = common::start_upstream().await;
    let mut config = common::config_for(port, &[], true);
    config.limits.max_active_streams = 1;
    config.limits.max_active_streams_per_host = 1;
    let state = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let url = common::proxy_uri(&target(port, "/file"));
    let first = common::send(state.clone(), Method::GET, url.clone(), &[]).await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = common::send(state, Method::GET, url, &[]).await;
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);

    drop(first);
}

#[tokio::test]
async fn proxy_without_peer_extension_uses_loopback() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send_no_peer(
        state,
        Method::GET,
        common::proxy_uri(&target(port, "/file")),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::body_bytes(response).await;
    assert_eq!(body.as_slice(), common::FILE_BODY);
}

#[tokio::test]
async fn redirect_edge_cases_are_mapped() {
    let port = common::start_upstream().await;
    let state = common::state_for(
        common::config_for(port, &[], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );

    for path in ["/status-300", "/bad-location"] {
        let response = common::send(
            state.clone(),
            Method::GET,
            common::proxy_uri(&target(port, path)),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "{path}");
        assert!(
            String::from_utf8_lossy(&common::body_bytes(response).await)
                .contains("upstream_redirect_error")
        );
    }

    let mut config = common::config_for(port, &[], true);
    config.proxy.max_redirects = 2;
    let limited = common::state_for(
        config,
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        limited,
        Method::GET,
        common::proxy_uri(&target(port, "/chain/0")),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn dns_and_connect_failures_are_mapped() {
    let port = common::start_upstream().await;
    let dns_state = common::state_for(
        common::config_for(port, &["missing.test"], true),
        common::resolver_for(&[("upstream.test", port)]),
        None,
    );
    let response = common::send(
        dns_state,
        Method::GET,
        common::proxy_uri(&format!("http://missing.test:{port}/file")),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await).contains("upstream_dns_error")
    );

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let connect_state = common::state_for(
        common::config_for(closed_port, &[], true),
        common::resolver_for(&[("upstream.test", closed_port)]),
        None,
    );
    let response = common::send(
        connect_state,
        Method::GET,
        common::proxy_uri(&format!("http://upstream.test:{closed_port}/file")),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        String::from_utf8_lossy(&common::body_bytes(response).await)
            .contains("upstream_connect_error")
    );
}
