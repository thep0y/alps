# ALPS Architecture and Implementation Notes

This document describes the actual P0 implementation, corresponding to docs/DESIGN.md and docs/IMPLEMENTATION_PLAN.md.

## 1. Data flow

```text
client
  |  GET/HEAD /v1/proxy?url=...
  v
axum Router (routes.rs)
  |  method/auth/body/query
  v
UrlPolicy (policy/mod.rs)
  |  scheme/host/port/userinfo/fragment/IP literal
  v
Limits (limits.rs)
  |  global rate -> global permit
  v
RedirectController (upstream/redirects.rs)
  |-- SecureResolver preflight (upstream/resolver.rs)
  |-- reqwest Client (upstream/client.rs)
  |      custom resolver filters every address
  |-- 3xx? validate/loop/hop -> next
  v
Header filtering (http/headers.rs)
  v
GuardedStream (upstream/stream.rs)
  |  permits + byte counter + observer
  v
axum Body::from_stream -> hyper HTTP/1 -> client
```

## 2. Module responsibilities

| Module | Responsibility |
|---|---|
| `src/main.rs` | Argument parsing, configuration loading, logging init, listener, exit codes |
| `src/config.rs` | TOML deserialization, environment overlays, validation, rule normalization |
| `src/app.rs` | AppState: config, policy, resolver, client, limits, auth, redirect controller |
| `src/server.rs` | TCP accept loop, HTTP/1 header limits, connection cap, graceful shutdown |
| `src/http/routes.rs` | Routing, request orchestration, response construction, 304/204 header fixups |
| `src/http/query.rs` | Raw query parsing; does not convert `+` to space |
| `src/http/headers.rs` | Request/response header allowlists, hop-by-hop, security headers, Content-Disposition |
| `src/http/errors.rs` | ProxyError, status/error codes, JSON error body |
| `src/policy/mod.rs` | ValidatedTarget, scheme/host/port validation |
| `src/policy/allowlist.rs` | Exact/subdomain label-boundary matching |
| `src/policy/ip_policy.rs` | Special-address tables, IPv4/IPv6 decisions |
| `src/upstream/resolver.rs` | DnsBackend, SecureResolver, address filtering and pre-resolution |
| `src/upstream/client.rs` | The single shared reqwest Client |
| `src/upstream/redirects.rs` | Manual redirect state machine, error mapping |
| `src/upstream/stream.rs` | GuardedStream, byte limits, access logging |
| `src/limits.rs` | Global/per-host semaphores, RPS, HostPermit cleanup |
| `src/auth.rs` | Bearer constant-time comparison, auth.mode=none guard |
| `src/telemetry.rs` | tracing initialization, request_id |

Dependency direction: `http -> {policy, upstream, limits, auth, telemetry}`; `upstream::resolver -> policy::ip_policy`; `upstream` never accepts a bare URL string.

## 3. Request lifecycle

1. Connection admission: connection semaphore, `TCP_NODELAY`, HTTP/1 header read timeout and maximum header buffer.
2. Route to `/v1/proxy`; methods other than GET/HEAD return 405.
3. `request_id`: accept a valid `X-Request-Id` or generate one; echo it in the response.
4. Bearer authentication; skipped when `auth.mode=none`.
5. Reject GET/HEAD with a body (400).
6. Parse the `url` parameter (percent-decode, no form `+` conversion), check length/control characters.
7. `UrlPolicy::validate`: scheme, port, userinfo, fragment, IP literals, allowlist.
8. Global RPS -> global concurrency permit.
9. RedirectController: pre-resolve -> send (header timeout) -> validate/loop/hop per 3xx -> final response.
10. Final response: status + header filtering + security headers + Content-Disposition.
11. Known Content-Length over the limit: return 502 before writing headers.
12. HEAD/204/304: empty body, release permits.
13. Otherwise: `Body::from_stream(GuardedStream)`; permits live as long as the Body.
14. Stream end/error/disconnect: access log + permit release.

## 4. Concurrency permits and stream lifetime

- `GlobalPermit` is acquired at request start and lives with the response Body until the stream ends.
- `HostPermit` is held for the current hop; when following a redirect, acquire the next hop's permit before replacing the previous one to avoid a permit gap.
- `StreamPermits` holds both and moves into `GuardedStream`; both are released when the Body drops.
- `HostPermit::drop` removes the host slot once the last permit is released and all permits are available, preventing unbounded map growth.
- Client disconnect -> Body drop -> reqwest Response drop -> unread connection closed -> permits released -> log outcome=client_disconnect.

## 5. Redirect state machine

- Follow only 301/302/303/307/308; 304 is a final response; 300/305/306 are upstream protocol errors.
- Missing/non-UTF-8/unparseable `Location` -> 502 `upstream_redirect_error`.
- The next hop goes through `UrlPolicy::validate_url`: invalid URL -> 502; policy rejection -> 403 `destination_denied`.
- Loop detection uses a set of normalized URLs (including query); default hop limit is 5.
- GET/HEAD keeps its method across 301/302/303/307/308 (no body-replay issue).

## 6. Timeouts

| Stage | Configuration | Implementation |
|---|---|---|
| DNS | `connect_timeout_secs` | `tokio::time::timeout` around DnsBackend |
| TCP connect | `connect_timeout_secs` | reqwest `connect_timeout` |
| Response headers | `response_header_timeout_secs` | `timeout(client.execute())` |
| Body read idle | `read_idle_timeout_secs` | reqwest `read_timeout` (reset per read) |
| No whole-request timeout | - | `ClientBuilder::timeout` is not called |

## 7. Error codes

| Scenario | HTTP | code |
|---|---|---|
| Invalid parameter/URL, body present | 400 | `invalid_url` / `request_body_not_allowed` |
| Authentication failure | 401 | `unauthorized` |
| Allowlist/scheme/port/userinfo/fragment/IP literal | 403 | `destination_denied` |
| All resolved IPs violate policy | 403 | `ip_denied` |
| Unsupported method | 405 | `method_not_allowed` |
| Rate limit | 429 | `rate_limited` |
| Redirect error/loop/hop limit | 502 | `upstream_redirect_error` |
| Connection failure | 502 | `upstream_connect_error` |
| DNS failure (not policy) | 502 | `upstream_dns_error` |
| Response exceeds limit (known length) | 502 | `upstream_response_too_large` |
| Upstream protocol error | 502 | `upstream_protocol_error` |
| Concurrency limit | 503 | `capacity_exceeded` |
| Connect/header/read timeout | 504 | `upstream_timeout` |
| Other | 500 | `internal_error` |

Upstream business statuses (200/206/304/404/416, etc.) are passed through unchanged.

## 8. Header handling

Requests forward only `Range`, `If-Range`, `If-None-Match`, `If-Modified-Since` (optionally `Accept`), and force `Accept-Encoding: identity`; Host/Cookie/Authorization/Proxy-*/X-Forwarded-* are not forwarded.

Responses pass through only Content-Type/Length/Range/Accept-Ranges/ETag/Last-Modified/Cache-Control/Expires/Content-Disposition/Content-Encoding; Set-Cookie/Location/Transfer-Encoding/Connection and their extension fields are stripped; security headers are enforced. 204/304 carry no Content-Length.

## 9. Design vs. implementation differences

- A custom accept loop (hyper HTTP/1 builder + hyper-util GracefulShutdown) is used because `axum::serve` does not expose inbound header limits.
- Raw query parsing is manual to avoid form decoding rewriting `+`.
- Each request performs one DNS pre-resolution before sending so `ip_denied` is stable; the actual connection is still filtered by the resolver.
- 304 is not followed; the `Content-Length: 0` that axum adds is removed outside the service layer.
- Global RPS plus optional per-client-IP limiting; X-Forwarded-For is trusted only behind configured proxies.

## 10. Signed links

- Issue: `POST /v1/sign` (Bearer-protected) with `{"url","expires_in_secs","max_bytes"}`. The URL first passes UrlPolicy, so signing cannot bypass the allowlist.
- Token format: `base64url(payload_json).base64url(HMAC-SHA256(payload))`; payload contains version, URL, expiry and optional max bytes.
- Verification: `GET/HEAD /p/{token}` does not use Bearer auth; HMAC is checked in constant time; expiry/tampering returns 401/403. After verification the URL is revalidated by UrlPolicy and SecureResolver.
- Rotation: `ALPS_SIGNING_SECRET` signs and verifies; `ALPS_SIGNING_SECRET_PREVIOUS` only verifies old tokens.
- No server-side state: the token is a bearer capability and expiry is the only revocation mechanism; rotate keys after a leak.

## 11. Metrics

The admin listener (`[metrics] enabled = true`, default `127.0.0.1:9090`) exposes:

- `alps_requests_total{status_class}`
- `alps_policy_denied_total{reason}`
- `alps_redirects_total`
- `alps_bytes_sent_total`
- `alps_stream_failures_total{reason}`
- `alps_active_streams` (computed from Limits at scrape time)
- `alps_upstream_duration_seconds` (histogram)

Label values come from fixed enums or status classes; no host/URL high-cardinality labels. Metrics are process-global and are not reset by hot reload.

## 12. Hot reload

- `AppHandle` uses `ArcSwap<AppState>`. Each request takes one snapshot at start and uses it for the whole request path.
- On SIGHUP: `Config::load` -> validate -> build a new AppState -> atomic swap. Failures keep the old state and log an error.
- In-flight requests/streams keep the old snapshot and old Limits; new requests use the new snapshot. Metrics counters persist.
- Server-level settings (listen address, connection caps, inbound header limits) stay with the old `Arc<Config>` and require a restart.
- The metrics listener is determined at startup; hot reload does not rebind it.
- The log filter is initialized at startup; hot reload does not change the log level.

## 13. Per-IP rate limiting

- `limits.max_requests_per_second_per_ip` = 0 disables it; otherwise a token bucket per client IP is used, with periodic cleanup of expired entries.
- The client IP comes from the TCP peer; `X-Forwarded-For` is parsed right-to-left only when the peer belongs to `limits.trusted_proxies`, skipping trusted proxies and taking the first untrusted address.
- Global RPS and per-IP RPS both apply; either limit returning false yields 429.

## 14. Path rules, CORS and inbound HTTP/2

- `path_prefixes`: optional literal path prefixes on an allowlist entry, matched against the URL parser's normalized `url.path()` (dot segments folded, percent-encoding preserved). Empty means all paths for that host.
- CORS: `cors.allowed_origins` is an exact origin list; middleware adds `Access-Control-Allow-Origin` / `Expose-Headers` / `Vary: Origin` only for matching origins, and answers `OPTIONS` on `/v1/proxy` and `/p/{token}` with a preflight (no Bearer required). Non-matching origins receive no CORS headers.
- Inbound HTTP/2: `server.http2 = true` uses the hyper-util auto builder to accept both HTTP/1.1 and h2c (prior knowledge); h1 header limits still apply, and `server.http2_max_concurrent_streams` caps h2 streams. Disabled by default; production usually terminates TLS at Caddy/Nginx and connects back over h2c.
- All new policy is read from the per-request AppState snapshot and applies to new requests after a hot reload.

## 15. Egress bandwidth shaping

- `limits.max_egress_bytes_per_second` is a process-wide response-body token bucket shared by all streams; 0 disables it.
- Bucket capacity is max(rate, 64 KiB), allowing short bursts; a chunk larger than capacity is split into multiple acquisitions to avoid permanent waiting.
- Throttling happens in `GuardedStream::poll_next`: check the byte limit first, then asynchronously acquire tokens; Pending while waiting propagates backpressure to the upstream connection.
- Token waits are cancelled by client disconnect; streams/permits still follow the Body lifetime.
- Shaping is approximately fair (multiple streams share one bucket); it does not guarantee an exact per-stream rate.

## 16. Later (P2)

- Disk cache, OpenTelemetry, upstream HTTP/3; see [decisions.md](./decisions.md) for the rationale.
- One-time/revocable signed links would require persistence or short-lived state storage, beyond the stateless goal.
