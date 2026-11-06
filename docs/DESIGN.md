# ALPS — AllowList Proxy Service

> **Engineering Design RFC**
> Version: 1.0 (design draft) | Date: 2026-11-02 | Implementation language: Rust
> Project name: **ALPS** (**A**llow**L**ist **P**roxy **S**ervice)

## 1. Project overview

ALPS is a lightweight, stateless file proxy for low-resource servers, based on a domain allowlist. Clients hand ALPS an HTTP(S) URL; ALPS checks that the destination is authorized, follows redirects safely, and streams the upstream response to the client. Typical uses include software packages, installers, images, audio/video, backups, and short-lived signed object-storage links.

**Positioning:** ALPS is a restricted URL-fetch/file-forwarding gateway. It is not a general-purpose HTTP forward proxy for arbitrary destinations, nor a transparent reverse proxy for arbitrary APIs.

### 1.1 Design principles

- Deny by default: an empty allowlist forwards nothing; only HTTPS, standard ports and publicly routable IPs are allowed by default.
- Per-hop validation: ALPS owns redirects; every new URL must pass the rules again.
- Streaming and backpressure: never read a whole file into memory, never use unbounded queues or buffers.
- Low footprint: share the outbound connection pool and bound concurrency, connection count and file descriptors.
- Protocol correctness: preserve status codes and headers needed for downloads, especially Range/206/416.
- Auditability: consistent access decisions, structured error codes, redacted logs and a clear test matrix.

### 1.2 Goals and non-goals

| Category | Scope |
|---|---|
| P0 required | Domain allowlist, HTTPS, manual redirects, DNS/IP defenses, GET/HEAD, streaming, Range, connect timeouts, rate/concurrency limits, health checks, TOML configuration |
| P1 priority | Configuration hot reload, HMAC short-lived signed links, per-host concurrency, Prometheus metrics, Docker/ARM releases |
| P2 optional | Fine-grained path rules, total egress bandwidth shaping, optional disk cache, OpenTelemetry, client HTTP/2/HTTP/3 |
| Non-goals | Forwarding arbitrary HTTP methods, HTTP CONNECT, WebSocket, upload proxying, transcoding, persistent files, default web-content rendering |

## 2. Use cases and interface contract

### 2.1 HTTP API

| Endpoint | Function | Security requirement |
|---|---|---|
| `GET /v1/proxy?url=<encoded_url>` | Proxy a target file and follow redirects server-side | Authentication or trusted-network-only access |
| `HEAD /v1/proxy?url=<encoded_url>` | Fetch file metadata without a body | Same destination checks as GET |
| `GET /p/{token}` | Shareable time-limited download link (P1) | HMAC validation, expiry, signed-content constraints |
| `HEAD /p/{token}` | HEAD for a signed link (P1) | Same authorization rules as signed GET |
| `GET /healthz` | Process liveness | Simple status only; no configuration leakage |
| `GET /readyz` | Configuration/startup status | Local or trusted networks only |
| `GET /metrics` | Monitoring metrics (P1) | Local or trusted networks only |

Example:

```http
GET /v1/proxy?url=https%3A%2F%2Ffiles.example.org%2Freleases%2Fapp.tar.gz HTTP/1.1
Host: proxy.example.net
Authorization: Bearer <private-api-token>
Range: bytes=1048576-
```

For Range-capable upstreams, ALPS returns `206 Partial Content`, `Content-Range` and the file body chunks. If the upstream does not support Range, ALPS does not fabricate a partial response; upstream behavior is preserved.

URL constraints: a single target URL defaults to 8 KiB maximum; after decoding only HTTP(S) URLs are allowed, and a scheme-less target is normalized to HTTPS; usernames, passwords, fragments (@#...@) and control characters are rejected. Preserve the original query semantics of signed URLs: do not reorder query parameters or arbitrarily modify percent-encoding. Compatibility-test signed links from different object stores. Never log full URLs or query strings at normal levels.

### 2.2 Request/response behavior

Only GET and HEAD are accepted; other methods return `405` and are not forwarded upstream.

Explicitly forwardable request headers: `Range`, `If-Range`, `If-None-Match`, `If-Modified-Since`, and optionally `Accept`. Client `Host`, `Cookie`, `Authorization`, `Proxy-*` and `X-Forwarded-*` are not forwarded.

Send `Accept-Encoding: identity` upstream by default and disable client-library auto-decompression, avoiding mismatches between `Content-Length`/`Content-Range` and the byte stream.

Preserve upstream response headers: `Content-Type`, `Content-Length`, `Content-Range`, `Accept-Ranges`, `ETag`, `Last-Modified`, `Cache-Control`, `Expires`, validated `Content-Disposition`, etc. Do not forward `Set-Cookie`, `Location` (redirects are consumed server-side), `Transfer-Encoding` or other hop-by-hop headers; also parse extension hop-by-hop fields listed by `Connection`.

Preserve file-serving upstream statuses: `200`, `206`, `304`, `404`, `416`, etc. Internal proxy errors return ALPS-owned status/error codes.

Forward only file-semantics headers and do not inject additional response headers. Active content and unknown types default to `Content-Disposition: attachment`; safe text types (`text/plain`, CSV, Markdown, JSON, YAML, PAC) may be served `inline` so browsers can display them. Browser-embedded images/video require a separate trusted origin, strict types and explicit configuration.

CORS is closed by default; enable it only for trusted origins.

## 3. Overall architecture

```text
  Browser / curl / downloader
             |
             v
  +-----------------------------+
  |   Axum HTTP entry           |
  |   routing/auth/limits/tokens|
  +--------------+--------------+
                 |
                 v
  +-----------------------------+
  |   URL Parser + Policy Engine|
  |   scheme/host/port/path     |
  +--------------+--------------+
                 |
                 v
  +-----------------------------+
  | Redirect Controller         |<---------+
  | max N hops, loop, recheck   |          |
  +--------------+--------------+          |
                 |                         |
                 v                         |
  +-----------------------------+          |
  | Secure Resolver + Connector |          |
  | DNS filter/verified IP/TLS  |          |
  +--------------+--------------+          |
                 |                         |
                 v                         |
  +-----------------------------+          |
  | Reqwest Client / Conn Pool  |--3xx-----+
  | GET/HEAD HTTP(S)            |
  +--------------+--------------+
                 | 200/206/304/4xx...
                 v
  +-----------------------------+
  | Header Filter + Body Stream |
  | backpressure/byte cap/cancel|
  +--------------+--------------+
                 |
                 v
              Client
```

No database, no disk file cache, no centralized queue. Every proxy request runs as an independent async task; resource budgets are enforced by global and per-upstream semaphores. All outbound requests go through one protected component; no other module may bypass checks and issue network requests.

## 4. Core module responsibilities

Prefer one Cargo package split by module; avoid an early Workspace and complex plugin machinery.

```text
alps/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── config/
│   └── alps.example.toml
├── src/
│   ├── main.rs                  # startup, signals, graceful shutdown
│   ├── config.rs                # TOML/env, validation
│   ├── app.rs                   # AppState and shared resources
│   ├── http/
│   │   ├── mod.rs
│   │   ├── routes.rs            # GET/HEAD/health/metrics
│   │   ├── headers.rs           # request/response header filtering
│   │   └── errors.rs            # status/error mapping
│   ├── policy/
│   │   ├── mod.rs
│   │   ├── allowlist.rs         # host, port, scheme rules
│   │   └── ip_policy.rs         # address ranges and IP classes
│   ├── upstream/
│   │   ├── mod.rs
│   │   ├── client.rs            # shared reqwest client
│   │   ├── resolver.rs          # secure DNS resolver
│   │   ├── redirects.rs         # manual redirect state machine
│   │   └── stream.rs            # bounded stream, limits, cancellation
│   ├── auth.rs                  # API token / signed links
│   ├── limits.rs                # semaphores/rate limits
│   └── telemetry.rs             # tracing/metrics
├── tests/
│   ├── policy_tests.rs
│   ├── redirect_tests.rs
│   ├── streaming_tests.rs
│   └── integration_tests.rs
├── deploy/
│   ├── Dockerfile
│   └── alps.service
└── docs/
    ├── architecture.md
    └── security.md
```

Suggested module interfaces:

- `UrlPolicy::validate(&Url) -> Result<ValidatedTarget, PolicyError>`: returns normalized valid target information; never expose raw unchecked URLs to outbound modules.
- `SecureResolver`: filters DNS answers before they reach the connector and only supplies permitted addresses; the connection must use the same validated set.
- `RedirectController::fetch(ValidatedTarget, Method, ForwardHeaders) -> Result<UpstreamResponse, ProxyError>`: centralizes per-hop authorization, loop detection, hop limits and the final response.
- `StreamBridge`: wraps upstream `bytes_stream()` in a backpressured response Body and holds concurrency permits until the stream ends or the client disconnects.

## 5. Allowlist and security model (P0 core)

### 5.1 Allowlist matching

Each rule contains `host`, `include_subdomains`, `schemes` and `ports`; `path_prefixes` may be added later.

Exact match takes priority: `files.example.org` does not allow `files.example.org.evil.example`.

For subdomain rules use DNS label boundaries instead of naive `contains/ends_with`; `sub.files.example.org` matches but `notfiles.example.org` does not.

Normalize with lowercasing, IDNA ASCII normalization and trailing-dot handling; rules and targets use the same normalization.

Reject IP literals, URL userinfo, non-standard ports and unsupported schemes by default. An empty allowlist always denies.

An upstream A may redirect to B only if both A and B are allowed by their own rules; there is no "trust any redirect after the initial domain" mode.

### 5.2 SSRF / DNS rebinding defense

Validating the domain alone is insufficient. Check the real outbound destination IP and prevent re-resolution between validation and connection.

- Validate the input URL's scheme, host and port against the allowlist.
- Feed DNS answers through `SecureResolver` and reject loopback, private, link-local, multicast, reserved, unspecified, special-purpose addresses and IPv4-mapped IPv6, etc. Use a deny-by-default, maintainable address classification covering IPv4 and IPv6.
- Only supply validated IPs to the actual connector and preserve/recheck that constraint before connect. Never "check DNS, then let the default client resolve again". Keep the original domain for TLS SNI, certificate validation and `Host`.
- Reapply rules on every redirect, reconnect and DNS refresh; on failure, deny without falling back to an unchecked address.
- Prevent environment/system proxy injection (e.g. `HTTP_PROXY`); default to `.no_proxy()`. Add deployment-level egress restrictions to private networks and cloud metadata as a second line of defense.
- Never bypass checks based on arbitrary headers, retries or error handling.
- P0 provides no global switch to bypass private-IP rejection for public domains pointing at private hosts. Future support, if any, must be a strictly isolated privileged mode with explicit IP ranges.

### 5.3 Redirect state machine

Handle `301`, `302`, `303`, `307` and `308` explicitly; since only GET/HEAD are allowed, there is no upload-body replay. Policy:

- Disable reqwest's default redirects: `redirect(Policy::none())`.
- Check the current URL, then issue the request.
- On an accepted 3xx, build the next hop from `Location`; relative addresses are supported; invalid targets are errors.
- Recheck scheme, port and domain for the next hop; revalidate through the secure resolver on the actual connection.
- Detect loops with a set of normalized URLs; allow at most `max_redirects` hops (default 5).
- Redirects must not forward client credentials, cookies or sensitive request headers across origins; ALPS authentication only applies to this service.
- On reaching the final upstream response, stream it to the client; do not expose intermediate `Location` or signed query parameters by default.

## 6. High-concurrency, low-footprint design

### 6.1 Resource model

Baseline server target: a 1 vCPU / 256 MiB Linux VPS; resource figures are acceptance goals, not unmeasured performance promises.

- Tokio async I/O: connections do not own dedicated OS threads; create tasks only for necessary work.
- Shared global `reqwest::Client`: share DNS/TLS/connection pooling; never build a client per request.
- Streaming: `reqwest::Response::bytes_stream()` → `axum::body::Body::from_stream()`; never `collect()` a whole file.
- Backpressure: stop reading upstream when the downstream consumer is slow; no unbounded `mpsc` channels.
- Limits: global active streams, per-destination active streams, client rate, maximum response bytes, connection/read timeouts, URL length, redirect count, inbound header size.
- Semaphore permits held during a download must live with the Body and must not be released when the HTTP handler returns.
- On client disconnect, cancel the upstream stream and release resources; graceful shutdown stops accepting new requests and lets existing connections finish before the deadline.

### 6.2 Default limits (initial values; tune with benchmarks)

| Item | Suggested default | Rationale |
|---|---|---|
| Global concurrent active response streams | 100 | Reject excess with 503 rather than unbounded queueing |
| Concurrent streams per upstream host | 20 | Prevent one host from consuming all egress |
| Global connect timeout | 5 s | Connect phase only |
| Time to first response header | 15 s | Independent of long download duration |
| Upstream read-idle timeout | 30 s | Reset after each successful read |
| Maximum redirects | 5 | Includes cross-origin redirects |
| Target URL length | 8 KiB | Bound memory/log amplification |
| Idle connections per upstream | 2–4 | Tune with TLS/H2 connection reuse benchmarks |
| Upstream idle connection reaping | 30 s | Avoid excess idle sockets in low-memory deployments |
| Maximum bytes per response | Unlimited by default, configurable | Supports large files; when enabled, count bytes in-stream |
| API request rate | Deployer-configured | Public deployments should enable it to prevent shared-egress abuse |

Important: do not set a short whole-request timeout for large files; configure connect, response-header and per-read timeouts separately. If a stream exceeds a limit or fails after headers are sent, the response cannot be rewritten as `4xx/5xx`; terminate the stream and record the reason.

### 6.3 Memory and throughput expectations

Memory consists roughly of baseline runtime RSS, per-connection TLS/protocol state, socket buffers, pending Bytes and monitoring/logging. Streaming avoids linear growth with file size but does not imply constant memory. Test with 100 concurrent downloads, a 1 GiB file and mixed fast/slow clients: target idle RSS <= 40 MiB and concurrent RSS <= 128 MiB. If targets are missed, tune concurrency limits and the connection pool first. These are proposed acceptance thresholds and must be validated on the specified system and build settings.

## 7. Configuration

Use `TOML + environment variables for secrets`. Validate at startup and refuse to start on failure. P1 hot reload atomically replaces `Arc<Config>`: existing downloads keep a consistent configuration snapshot while new requests use new rules (explicitly define whether revocation aborts existing streams).

```toml
# config/alps.toml
[server]
listen = "0.0.0.0:8080"
shutdown_grace_secs = 20

[proxy]
allow_http = false
max_redirects = 5
max_url_length = 8192
connect_timeout_secs = 5
response_header_timeout_secs = 15
read_idle_timeout_secs = 30
max_response_bytes = 0           # 0 = unlimited

[limits]
max_active_streams = 100
max_active_streams_per_host = 20
max_idle_connections_per_host = 3
idle_connection_timeout_secs = 30

[security]
block_non_global_ips = true       # fixed on in P0
block_ip_literal_hosts = true
no_system_proxy = true           # fixed on in P0
default_content_disposition = "attachment"

[auth]
mode = "bearer"                 # recommended for public deployments; token from ALPS_API_TOKEN

[[allowlist]]
host = "files.example.org"
include_subdomains = false
schemes = ["https"]
ports = [443]

[[allowlist]]
host = "downloads.example.net"
include_subdomains = false
schemes = ["https"]
ports = [443]

[logging]
level = "info"
format = "json"
redact_url_query = true
```

Notes: this is a configuration-contract draft; field names must stay consistent across `serde(deny_unknown_fields)`, startup validation and documentation, and safety switches must not be accidentally disabled. `auth.mode="none"` must explicitly declare trusted-network-only operation and is not a public default.

## 8. Rust stack and dependencies

As of 2026-11-02, the recommended first version is below; released builds pin patch versions in `Cargo.lock`.

| Purpose | Crate | Rationale |
|---|---|---|
| Async runtime | `tokio` 1.x | Mature async networking and synchronization |
| HTTP service | `axum` 0.8.x | Lightweight routing, integrates with hyper/tower |
| Upstream HTTP | `reqwest` 0.13.x | Pooling, HTTP/2, Rustls, redirects, streaming |
| HTTP/TLS | `rustls` (integrated by reqwest) | Avoids a direct runtime dependency on system OpenSSL |
| Configuration | `serde`, `toml` | Deserialization and validation |
| URL | `url` | Normalized parsing and relative URL joining |
| Streaming | `futures-util`, `bytes` | Streaming bytes and combinators |
| Logging | `tracing`, `tracing-subscriber` | Structured logs and traces |
| Errors | `thiserror` | Typed business errors |
| Rules/ranges | `ipnet` | IP/CIDR helpers; still needs a complete special-address policy |
| Signing | `hmac`, `sha2`, `base64` (P1) | Signed links |

Minimal dependency example:

```toml
[package]
name = "alps"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = { version = "0.8", default-features = false, features = ["http1", "tokio", "json"] }
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "macros", "net", "time", "signal", "sync"] }
reqwest = { version = "0.13", default-features = false, features = ["rustls", "http2", "stream"] }
serde = { version = "1", default-features = false, features = ["std", "derive"] }
toml = { version = "1.1", default-features = false, features = ["parse", "serde", "std"] }
url = { version = "2", default-features = false, features = ["std"] }
bytes = { version = "1", default-features = false, features = ["std"] }
futures-util = { version = "0.3", default-features = false, features = ["std", "async-await"] }
tracing = { version = "0.1", default-features = false, features = ["std"] }
tracing-subscriber = { version = "0.3", default-features = false, features = ["std", "fmt", "env-filter", "json"] }
thiserror = { version = "2", default-features = false, features = ["std"] }
ipnet = { version = "2", default-features = false, features = ["std"] }
```

Note: reqwest 0.13 differs from 0.12 in TLS feature names; use the verified `rustls` feature for 0.13. `reqwest::Response::bytes_stream()` requires the `stream` feature. Every direct dependency disables default features and enables only what the code uses; `Cargo.toml` is authoritative. The project's Rust version floor is 1.99; this is a project policy, not merely the highest MSRV of the dependency graph.

Suggested client construction:

```rust
let client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .no_proxy()
    .no_gzip()
    .no_brotli()
    .no_zstd()
    .no_deflate()
    .referer(false)
    .connect_timeout(std::time::Duration::from_secs(5))
    .read_timeout(std::time::Duration::from_secs(30))
    .pool_max_idle_per_host(3)
    .pool_idle_timeout(std::time::Duration::from_secs(30))
    // .dns_resolver(secure_resolver)  // project implementation; binds DNS-IP validation to connections
    .build()?;
```

If implementing a custom secure resolver via `ClientBuilder::dns_resolver`, test retry paths, multi-address fallback and connection reuse, and verify that no real TCP connection can land on an unvetted address.

## 9. Error model, observability and deployment

### 9.1 HTTP error mapping

| Scenario | Suggested status | Internal error code |
|---|---|---|
| Invalid URL / missing parameter | 400 | `invalid_url` |
| Authentication failure | 401 | `unauthorized` |
| Target or redirect target not allowlisted | 403 | `destination_denied` |
| Resolved IP violates policy | 403 | `ip_denied` |
| Unsupported HTTP method | 405 | `method_not_allowed` |
| Rate limited | 429 | `rate_limited` |
| Redirect limit/loop or upstream protocol error | 502 | `upstream_redirect_error` |
| Upstream connection failure | 502 | `upstream_connect_error` |
| Global concurrency exceeded | 503 | `capacity_exceeded` |
| Upstream timeout | 504 | `upstream_timeout` |

Return a JSON error object before response headers are written; after a download starts, errors can only truncate the stream and record metrics. Preserve upstream business statuses such as `404/416` according to explicit passthrough rules; do not treat them as internal proxy errors.

### 9.2 Logging and metrics

Recommended log fields: `request_id`, latency, allow/deny decision, upstream host (without private query), status, redirect count, response bytes and error type. Never log auth headers, full URLs, signing parameters, cookies or sensitive filenames.

Suggested P1 metrics: `alps_requests_total{status_class}`, `alps_active_streams`, `alps_upstream_duration_seconds`, `alps_bytes_sent_total`, `alps_redirects_total`, `alps_policy_denied_total{reason}`, `alps_stream_failures_total`. Never put arbitrary user URLs or high-cardinality hosts in Prometheus labels.

### 9.3 Deployment

- Linux `x86_64` / `aarch64`, single binary; containers run as non-root.
- `0.0.0.0:8080` should only be exposed on a trusted network or behind a reverse proxy; use Caddy/Nginx for TLS and set request/connection limits.
- Separate configuration and credentials: TOML file + environment variables/container secrets; never commit credentials.
- Read-only rootfs, minimal Linux capabilities, `nofile` and memory limits, private-network egress firewall, systemd `Restart=on-failure`.
- Independent download bandwidth budget and egress quota; public instances should enable authentication and rate limits by default.

## 10. Testing and acceptance

### 10.1 Functional matrix

| Test | Expected |
|---|---|
| Allowlist hit / miss / empty | Allow / 403 / 403 |
| Case, trailing dot, IDNA, lookalike domains | Correct normalization, no suffix false positives |
| HTTPS→HTTPS same-domain/cross-allowlisted redirect | Correct hop-by-hop following |
| Redirect to unauthorized domain/private IP/non-HTTP(S) | Immediate denial |
| Relative `Location`, redirect loop, hop overflow | Correct resolution or controlled error |
| DNS multi-answer, rebinding, IPv4-mapped IPv6 | Never connect to a violating address |
| `GET` 1 GiB file with slow clients | Streaming, bounded RSS, effective backpressure |
| `HEAD`, Range, `206`, `416`, conditional `304` | HTTP-correct status codes and headers |
| Client abort and unexpected upstream close | Permits, sockets and tasks released |
| Upstream chunked transfer / unknown Content-Length | Bounded memory and clean completion |
| TLS certificate failure, handshake failure, read timeout | Understandable internal error |
| Upstream Content-Encoding with Range | No incorrect decompression, byte ranges intact |
| Upstream HTML/SVG/malicious filenames | Security headers, no same-origin script execution |
| 100 concurrent, connection reuse, limit hits | Stable within limits, fast rejection beyond them |

### 10.2 Performance acceptance

Use a uniform 1 vCPU, 256 MiB Linux test environment, a local controlled upstream and a separate load generator:

- 100 active streams continuously transferring: no OOM, no unbounded task growth, target RSS <= 128 MiB.
- A single 1 GiB download: RSS must not grow linearly with file size.
- After disconnecting 100 downloads: active-stream count and resource usage fall back.
- Allowlist denial, redirect errors and DNS-rebinding simulations all pass.
- Quantify extra processing overhead (excluding network RTT and upstream response time) with p50/p95; do not promise a throughput number in advance.

Load tools may include `wrk`, `oha`, `curl` and a controlled HTTP upstream; record CPU, RSS, sockets, download throughput, p95 latency and failure rate per run.

## 11. Roadmap

| Milestone | Deliverables | Completion criteria |
|---|---|---|
| M0: foundation | Rust package, configuration, logging, health checks | Starts, parses config, rejects invalid config |
| M1: secure proxy | URL/host/IP validation, GET/HEAD, manual redirects | SSRF test suite passes |
| M2: file compatibility | Streaming Body, Range, conditional requests, response header policy | Large-file, resume and abort tests pass |
| M3: resource governance | Global/per-host concurrency, timeouts, rate limits, graceful shutdown | Stable in a 256 MiB environment |
| M4: delivery | CI, integration tests, Docker, Linux ARM, docs | Reproducible build and basic operations |
| M5: optional enhancements | HMAC links, metrics, hot reload, cache | Independent feature and compatibility tests |

### 11.1 Decision records (ADR)

- ADR-001: no file cache in the first version. Disk space and Range cache policy add state complexity, contrary to the low-resource goal.
- ADR-002: manual redirect handling. Every hop must recheck the allowlist and DNS/IP safety.
- ADR-003: shared reqwest Client. Reuse TCP/TLS connections and avoid rebuilding pools under high concurrency.
- ADR-004: Axum at the server entry instead of raw Hyper. Easier routing, errors and middleware early on; refactor benchmark-driven if performance is insufficient.
- ADR-005: prioritize GET/HEAD file semantics over a general forwarding proxy. Clearer attack surface, resource model and semantics.

## 12. Design boundaries to decide early

- The allowlist is a security boundary: if a deployment allows a public domain controlled by a third party, even a publicly routable IP can create egress abuse or content risk. Administrators must review domain control.
- Short-lived signed URLs usually contain sensitive query parameters and must not leak into logs, metrics or traces. Prefer time-limited HMAC tokens for shareable links.
- Media playback: Range support satisfies many players, but not every segment, CORS and browser MIME scenario; test it.
- Upstream hotlink protection: do not forge `Referer`, cookies or user identity by default; some upstreams will reject proxied requests, which is an expected compatibility boundary.
- Cross-origin CDN downloads: explicitly add every legitimate redirect target to the allowlist; never open all hosts for compatibility.
- Bandwidth and system resources: low memory does not mean unlimited throughput; network bandwidth, TCP/TLS, upstream limits and file descriptors all constrain real performance.

## References

- reqwest 0.13.5: https://docs.rs/reqwest/0.13.5/reqwest/
- reqwest custom DNS: https://docs.rs/reqwest/0.13.5/reqwest/dns/trait.Resolve.html
- reqwest RedirectPolicy: https://docs.rs/reqwest/0.13.5/reqwest/redirect/struct.Policy.html
- reqwest Response::bytes_stream: https://docs.rs/reqwest/0.13.5/reqwest/struct.Response.html
- axum Body::from_stream: https://docs.rs/axum/0.8.9/axum/body/struct.Body.html
- OWASP SSRF Prevention Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html
