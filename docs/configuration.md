# Configuration Reference

ALPS reads a single TOML file, validates it before binding listeners, and refuses to start on any invalid value. The annotated example is [config/alps.example.toml](../config/alps.example.toml); this document is the field-by-field reference.

## 1. Loading, path precedence and secrets

The configuration path is resolved in this order:

1. `--config <PATH>` or `--config=<PATH>` on the command line.
2. The `ALPS_CONFIG` environment variable.
3. The first existing path in this order:
   - `$HOME/.coinfig/alps/alps.toml`
   - `$HOME/.alps.toml`
   - `<directory containing the alps binary>/alps.toml`

If no default file exists, ALPS attempts the binary-directory path and reports that path in the startup error. A path supplied by `--config` or `ALPS_CONFIG` is used as-is and is not subject to this search.

The file must be valid UTF-8 TOML. Unknown keys, wrong types and values outside the documented ranges are startup errors; there is no permissive fallback.

The optional static UI directory has its own precedence: `--static-dir <PATH>` > `ALPS_STATIC_DIR` > `[web] dir`. The directory must exist and contain `index.html`; it is canonicalized at startup.

Secrets are never read from TOML:

| Environment variable | Required when | Rules |
|---|---|---|
| `ALPS_API_TOKEN` | `auth.mode = "bearer"` and a protected API is exposed | At least 16 bytes; compared in constant time. Not required when `web.dir` is set with `web.expose_api = false`. |
| `ALPS_SIGNING_SECRET` | `signing.enabled = true` | Base64 (standard or URL-safe, with or without padding) decoding to at least 32 bytes, or at least 32 raw bytes. |
| `ALPS_SIGNING_SECRET_PREVIOUS` | Optional | Same format as the current secret; only valid when `ALPS_SIGNING_SECRET` is also set. Old tokens remain valid while configured. |
| `ALPS_CONFIG` | Optional | Alternative configuration path. |
| `ALPS_STATIC_DIR` | Optional | Static UI directory; overrides `web.dir`. `--static-dir` overrides this variable. |

Validate without binding a port:

```bash
ALPS_API_TOKEN=0123456789abcdef alps --config config/alps.toml --check-config
```

The command prints a secret-free summary and exits 0 on success, 1 on a configuration error, and 2 on a bind/CLI error. `--help` lists the CLI options.

### Hot reload

`SIGHUP` re-reads the same path, validates the new document, and atomically swaps the runtime snapshot. An invalid reload is logged and the previous configuration remains active. In-flight responses keep the snapshot they started with.

Listener addresses, connection/header limits, the metrics listener and `web.dir` are startup settings and require a restart. `web.expose_api` is evaluated per request, so it can be toggled by SIGHUP while the static directory stays fixed. Use `--check-config` before sending `SIGHUP`.

## 2. Minimal configuration

```toml
[auth]
mode = "bearer"

[[allowlist]]
host = "files.example.org"
schemes = ["https"]
ports = [443]
```

`ALPS_API_TOKEN` must be set. If the allowlist is empty, startup succeeds with a warning but every proxy request is denied.

## 3. Top-level structure

| Section | Required | Purpose |
|---|---|---|
| `[server]` | No | Listener, header limits, HTTP/2, shutdown. |
| `[proxy]` | No | URL/redirect/timeout/response policy. |
| `[limits]` | No | Concurrency, rate limiting, egress shaping, trusted proxies. |
| `[security]` | No | Fixed SSRF safety switches and trusted-network declaration. |
| `[auth]` | No | Bearer token or explicit no-auth mode. |
| `[signing]` | No | HMAC-signed links and TTLs. |
| `[metrics]` | No | Prometheus admin listener. |
| `[cors]` | No | Exact browser origins and preflight cache. |
| `[web]` | No | Optional public UI and `/?url=` proxy route. |
| `[[allowlist]]` | Yes in practice | Destination domain rules; empty denies all. |
| `[logging]` | No | Filter, format, redaction. |

Omitted sections use the defaults listed below. `serde(deny_unknown_fields)` is enabled at every level.

## 4. `[server]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `listen` | string | `0.0.0.0:8080` | Must parse as `SocketAddr`. Restart required. |
| `shutdown_grace_secs` | integer | `20` | 1-300. Deadline for in-flight requests before connections are aborted. |
| `header_read_timeout_secs` | integer | `10` | 1-60. Slowloris defense for HTTP/1.1 request headers. |
| `max_header_bytes` | integer | `16384` | 8192-65536. Inbound header block limit. |
| `max_connections` | integer | `512` | 1-65535. Excess accepted connections are closed, not queued. |
| `http2` | boolean | `false` | Enables h2c prior knowledge on the proxy listener. HTTP/1.1 remains available. |
| `http2_max_concurrent_streams` | integer | `100` | 1-1000. Only used when `http2 = true`. |

## 5. `[proxy]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `allow_http` | boolean | `false` | Required for any `http` scheme in an allowlist rule. |
| `max_redirects` | integer | `5` | 0-10. 0 disables following; a 3xx response becomes `upstream_redirect_error`. |
| `max_url_length` | integer | `8192` | 128-65536. Maximum decoded target URL length in bytes, checked again after a missing `https://` scheme is added. |
| `connect_timeout_secs` | integer | `5` | 1-300. DNS resolution plus TCP connect. |
| `response_header_timeout_secs` | integer | `15` | 1-300. Time allowed for upstream response headers. |
| `read_idle_timeout_secs` | integer | `30` | 1-300. Per-read upstream body idle timeout; resets after each read. |
| `max_response_bytes` | integer | `0` | 0 means unlimited; otherwise at least 1024. Known oversized lengths fail before headers; unknown lengths are truncated mid-stream. |
| `forward_accept` | boolean | `false` | Forward only the client `Accept` header upstream. |
| `user_agent` | string | `alps/<version>` | 1-256 bytes, no control characters. |

## 6. `[limits]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `max_active_streams` | integer | `100` | 1-1000000. Global active response streams; over-limit requests get 503 `capacity_exceeded` without queueing. |
| `max_active_streams_per_host` | integer | `20` | 1-1000000 and <= `max_active_streams`. Per target host. |
| `max_idle_connections_per_host` | integer | `3` | 1-64. Upstream idle pool size. |
| `idle_connection_timeout_secs` | integer | `30` | 1-600. Idle upstream connection lifetime. |
| `max_requests_per_second` | integer | `0` | 0 disables; otherwise 1-1000000. Global ingress rate; over-limit gets 429 `rate_limited`. |
| `max_requests_per_second_per_ip` | integer | `0` | 0 disables; otherwise 1-1000000. Per-client-IP rate; the peer is used unless `trusted_proxies` is set. |
| `max_egress_bytes_per_second` | integer | `0` | 0 disables; otherwise 1-10000000000. Global response-body shaping; does not throttle upstream ingress. |
| `trusted_proxies` | array of strings | `[]` | CIDRs whose `X-Forwarded-For` is trusted for per-IP limiting. Empty means use the TCP peer. Never include untrusted peers. |

## 7. `[security]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `block_non_global_ips` | boolean | `true` | Fixed to true; false is a startup error. |
| `block_ip_literal_hosts` | boolean | `true` | Fixed to true; false is a startup error. |
| `no_system_proxy` | boolean | `true` | Fixed to true; false is a startup error. |
| `default_content_disposition` | string | `attachment` | Only `attachment` is accepted. Safe text types may still be served `inline`; active content always falls back to this value. |
| `trusted_network_only` | boolean | `false` | Explicit trust declaration. Required for `auth.mode = "none"` on a non-loopback listener; it does not relax allowlist, DNS or IP checks. |

These are defense-in-depth invariants, not tuning knobs. Keep them enabled.

## 8. `[auth]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `mode` | string | `bearer` | `bearer` requires `ALPS_API_TOKEN` of at least 16 bytes unless `web.dir` is set with `web.expose_api = false` (no protected API is exposed). `none` is allowed on a loopback listener, with `security.trusted_network_only = true`, or when `web.dir` is set with `web.expose_api = false`. |

`none` does not bypass destination policy. Use it only when the listener is unreachable from untrusted clients, or when the public `/?url=` route is intentionally the only entry point and the allowlist is the content boundary.

## 9. `[signing]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `enabled` | boolean | `false` | Enables `/p/{token}` and `POST /v1/sign`; requires `ALPS_SIGNING_SECRET`. |
| `default_ttl_secs` | integer | `300` | 1-31536000. Default lifetime for issued links. |
| `max_ttl_secs` | integer | `86400` | 1-31536000 and >= `default_ttl_secs`. Maximum accepted/issued lifetime. |
| `max_token_bytes` | integer | `4096` | 512-16384. Maximum signed-token size. |

Secret rotation: set `ALPS_SIGNING_SECRET_PREVIOUS` to the old key and `ALPS_SIGNING_SECRET` to the new key. New links use the current key; tokens signed with the previous key remain valid until that variable is removed. Setting only `ALPS_SIGNING_SECRET_PREVIOUS` is an error.

## 10. `[metrics]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `enabled` | boolean | `false` | Starts the separate Prometheus admin listener. |
| `listen` | string | `127.0.0.1:9090` | Must parse as `SocketAddr` and differ from `server.listen` when enabled. Restart required. |

The admin listener has no authentication. Bind it to loopback or a private interface; do not expose `/metrics` publicly.

## 11. `[cors]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `allowed_origins` | array of strings | `[]` | At most 64 exact origins of the form `scheme://host[:port]`. No wildcard, path, query, userinfo or fragment. Empty disables CORS; entries are normalized, sorted and deduplicated. |
| `max_age_secs` | integer | `600` | 0-86400. `Access-Control-Max-Age` for preflight responses. |

CORS responses never include credentials. Preflight is handled only for `/v1/proxy`, `/v1/sign` and `/p/` paths.

## 12. `[web]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `dir` | string | unset | Directory containing the built UI. Must exist, be a directory, and contain `index.html`; canonicalized at startup. Relative paths use the process working directory. Can be overridden by `ALPS_STATIC_DIR` and `--static-dir`. Restart required. |
| `expose_api` | boolean | `false` | Expose `/v1/proxy`, `/v1/sign` and `/p/{token}` on the public listener too. Requires `dir`. Applied on reload; hidden paths return 404 before auth or CORS preflight. |

When `dir` is set:

- `GET /` serves `index.html`; other paths are served from the directory as static files with directory listings disabled.
- `GET/HEAD /?url=<encoded_url>` runs the same proxy pipeline as `/v1/proxy` (allowlist, IP policy, redirects, limits, streaming) but does **not** require a bearer token. It is a public route.
- The JSON API routes are not reachable unless `expose_api = true`; hidden paths return `404` before auth or CORS preflight. Even with `expose_api = true`, `/?url=` remains public.
- `auth.mode = "bearer"` does not require `ALPS_API_TOKEN` when `expose_api = false`; `auth.mode = "none"` is allowed on a non-loopback listener in that mode.
- Public proxy responses use `Cache-Control: private, no-store` and add `X-Robots-Tag: noindex, nofollow`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` and `Cross-Origin-Resource-Policy: same-origin`. Static HTML gets a strict CSP; all static responses get the same hardening headers.

This mode is for a deliberately limited public download page. The allowlist is the content boundary; the bearer token is not sent to browsers.

## 13. `[[allowlist]]`

There may be zero or more rules. A target is allowed only when at least one rule matches **the same rule's** scheme, host and port, and any configured path prefix.

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `host` | string | required | DNS domain only. Lowercased, one trailing dot removed, IDNA normalized to ASCII. Max 253 bytes, labels max 63 bytes. Rejects IP literals, wildcards, slashes, userinfo and paths. |
| `include_subdomains` | boolean | `false` | When true, matches the host and labels below it using label boundaries (not raw string suffix matching). |
| `schemes` | array of strings | `["https"]` | Non-empty subset of `https` and `http`. `http` additionally requires `proxy.allow_http = true`. Sorted and deduplicated. |
| `ports` | array of integers | `[443]` | Non-empty, 1-65535; 0 is rejected. Sorted and deduplicated. |
| `path_prefixes` | array of strings | `[]` | At most 16 literal prefixes. Empty means all paths. Each must start with `/`, be at most 1024 bytes, contain no query/fragment/control/whitespace, and contain no `.` or `..` segments. Matching is literal `starts_with` on the normalized URL path, so trailing slashes matter. |

Examples:

```toml
[[allowlist]]
host = "files.example.org"
include_subdomains = false
schemes = ["https"]
ports = [443]
path_prefixes = []

[[allowlist]]
host = "downloads.example.net"
schemes = ["https"]
ports = [443]
path_prefixes = ["/releases/", "/pub/"]
```

```toml
# An empty allowlist is valid configuration but denies every request.
# allowlist = []
```

## 14. `[logging]`

| Field | Type | Default | Constraints and behavior |
|---|---|---|---|
| `level` | string | `info` | `tracing_subscriber::EnvFilter` syntax, e.g. `info`, `debug` or `alps=debug,tower=warn`. Invalid filters fail startup. |
| `format` | string | `json` | `json` or `text`. |
| `redact_url_query` | boolean | `true` | Fixed to true; false fails startup. |

Logs omit full URLs, query strings, signed tokens, authorization headers and cookies.

## 15. Validation and troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `unknown field` | Typo or unsupported key | Remove it or consult this reference; unknown keys are never ignored. |
| `server.listen` / `metrics.listen` error | Not `host:port` | Use a parseable `SocketAddr`, e.g. `127.0.0.1:8080`. |
| `auth.mode` requires token | Bearer mode with an exposed API but without `ALPS_API_TOKEN` | Export a token of at least 16 bytes, or set `web.dir` with `web.expose_api = false`. |
| `auth.mode = 'none'` error | Non-loopback bind without declaration | Set `security.trusted_network_only = true` only if the network is genuinely trusted, or use `web.dir` with `web.expose_api = false` for a public web-only listener. |
| `web.dir` error | Missing path, not a directory, or no `index.html` | Build `web/` and point `web.dir` at the `dist` directory. |
| `web.expose_api` error | `expose_api = true` without `dir` | Set `web.dir` or remove `expose_api`. |
| `signing.enabled` requires secret | Signing enabled without `ALPS_SIGNING_SECRET` | Provide a valid secret or disable signing. |
| `http requires proxy.allow_http` | `http` scheme with `allow_http = false` | Enable only for trusted networks, or use HTTPS. |
| `cors.allowed_origins` error | Wildcard, path, query or invalid origin | Use an exact origin such as `https://app.example.org`. |
| `path_prefixes` error | Missing `/`, `..` segment, query/fragment or too long | Use a literal normalized path prefix. |
| `metrics.listen` error | Same address as `server.listen` | Use a different admin address/port. |

## 16. Complete example

The complete annotated file is [config/alps.example.toml](../config/alps.example.toml). It includes every field with its default value and the constraints above.
