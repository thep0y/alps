# ALPS Security Notes

## 1. Threat model

ALPS is treated as a "controlled URL fetch gateway". It primarily defends against:

- **SSRF**: attackers inducing ALPS to reach loopback, private networks, cloud metadata, Unix sockets or other internal targets.
- **DNS rebinding**: validation resolves a public IP, then the connection resolves a private IP.
- **Redirect bypass**: the first hop is allowed, then a later hop targets an unauthorized domain or private network.
- **Credential/header leakage**: forwarding client Cookie/Authorization upstream, or passing upstream Set-Cookie back to clients.
- **Log leakage**: recording signed URLs, query strings or tokens in logs/errors.
- **Resource exhaustion**: many concurrent streams, slow clients, slow headers, oversized URLs, unbounded responses.

Out of scope for P0: arbitrary HTTP methods, CONNECT, WebSocket, uploads; upstream business authentication; transport encryption between clients and ALPS (TLS is terminated by a reverse proxy).

## 2. Security invariants (cannot be disabled)

1. An empty allowlist denies everything.
2. http(s) URLs are accepted; a URL without a scheme is normalized to `https://`, and HTTPS is the default. Explicit `http://` still requires `proxy.allow_http`.
3. userinfo, fragments, control characters and oversized URLs are rejected.
4. IP-literal targets are rejected (IPv4/IPv6/bracketed forms).
5. Every hop revalidates scheme/host/port and reapplies DNS/IP filtering.
6. Outbound connections only use addresses returned by SecureResolver; the repository forbids `.resolve()` / `.resolve_to_addrs()`.
7. `.no_proxy()` disables environment proxies.
8. Request/response headers use strict allowlists; hop-by-hop and `Connection` extension fields are stripped.
9. Only file-semantics headers are forwarded; active content stays `attachment`, while safe text types (`text/plain`, CSV, Markdown, JSON, PAC) may be served `inline`. No extra security headers are injected into proxied responses.
10. Logs never contain full URLs, query strings, tokens, Authorization or Cookie.
11. Concurrency permits live with the Body and are released when the client disconnects.
12. Global/per-host concurrency, RPS, connection/header/read timeouts, URL length, header size and connection count all have limits.

## 3. IP address policy

Rejected IPv4 ranges:

```text
0.0.0.0/8          10.0.0.0/8         100.64.0.0/10      127.0.0.0/8
169.254.0.0/16     172.16.0.0/12      192.0.0.0/24       192.0.2.0/24
192.88.99.0/24     192.168.0.0/16     198.18.0.0/15      198.51.100.0/24
203.0.113.0/24     224.0.0.0/4        240.0.0.0/4
```

IPv6 is limited to `2000::/3`, with these ranges additionally rejected:

```text
::/128             ::1/128            ::ffff:0:0/96      64:ff9b::/96
64:ff9b:1::/48     100::/64           2001::/32          2001:2::/48
2001:10::/28       2001:20::/28       2001:db8::/32      2002::/16
3fff::/20          5f00::/16          fc00::/7           fe80::/10
fec0::/10          ff00::/8
```

Notes:

- `IpAddr::is_global` is still unavailable on stable, so explicit tables are used; adding a special range requires updating `src/policy/ip_policy.rs` and this document.
- Rejecting all of `192.0.0.0/24` is conservative; a few addresses (e.g. 192.0.0.9/32) are globally reachable per IANA. Allow them explicitly and with review if required.
- Cloud metadata that uses public ranges (e.g. 168.63.129.16) cannot be distinguished at the application layer; block it with an egress firewall.
- Mixed DNS answers are filtered; if all answers are disallowed, the request is rejected with `ip_denied`.

## 4. DNS rebinding and TOCTOU

- `SecureResolver::resolve()` is called by reqwest for every new connection; filtering happens before `SocketAddr` values reach the hyper connector.
- Reusing a pooled connection does not re-resolve; that connection was validated when established.
- The pre-request "pre-resolution" is only for stable error attribution (mapping policy rejection to 403 `ip_denied`); the real connection is validated again by the resolver.
- If pre-resolution and connection resolution differ (rebinding), connection resolution rejects and the request fails with 502; it never falls back to an unvalidated address.

## 5. Redirects

- Follow only 301/302/303/307/308; 304 is passed through as a final response.
- Every hop goes through `UrlPolicy`; "trust later domains after the first hop" is not allowed.
- Loop detection uses normalized URLs (including query); default maximum is 5 hops.
- Cross-origin redirects carry no client credentials: the request header allowlist never includes Authorization/Cookie.
- Client `Host` is not sent upstream; TLS SNI/certificate validation uses the target URL's domain.

## 6. Request and response headers

Requests forward only `Range`, `If-Range`, `If-None-Match`, `If-Modified-Since` (optionally `Accept`) and force `Accept-Encoding: identity`. Host/Cookie/Authorization/Proxy-*/X-Forwarded-*/Connection/TE/Trailer/Upgrade/Content-* are not forwarded.

Responses pass through file-semantics headers only; Set-Cookie, Location, Transfer-Encoding, Connection and fields listed in `Connection` are stripped. No additional security headers are injected into proxy responses. A valid upstream `attachment` is preserved; invalid values are removed. If no disposition remains, safe text types (`text/plain`, `text/csv`, `text/markdown`, JSON, YAML, PAC) use `inline`, while active content (HTML, SVG, XML, JavaScript, PDF and unknown types) falls back to `attachment`.

The optional static UI is a separate response path: its HTML gets a strict CSP and its files get `nosniff`, frame-denial, referrer-policy and cross-origin isolation headers. Public `/?url=` responses also get those hardening headers plus `private, no-store` caching and `X-Robots-Tag`; the token-protected JSON API keeps the original proxy header policy.

204/304 send no Content-Length; HEAD preserves upstream Content-Length but sends no body.

## 7. Logging and errors

Allowed fields: `request_id`, method, host, status, error_code, redirects, bytes_sent, duration_ms, outcome, listen_addr.

Forbidden fields: full URL, query string, `/p/{token}`, Authorization, Cookie, signing parameters. reqwest errors are passed through `without_url()` before logging.

Error responses contain only stable error codes and generic messages; target URL details are not echoed.

## 8. Authentication and rate limiting

- `auth.mode = "bearer"`: environment variable `ALPS_API_TOKEN` (>=16 bytes), constant-time comparison; duplicate Authorization headers are rejected.
- `auth.mode = "none"`: on a loopback listener, on a non-loopback listener with explicit `security.trusted_network_only = true`, or when `web.dir` is set with `web.expose_api = false` (public web-only mode).
- With `web.dir` set, `GET/HEAD /?url=...` is a public route and intentionally bypasses bearer auth. When `web.expose_api = false` (default), no JSON API route is reachable, so bearer mode does not require a token; `Auth` is disabled and `/?url=` is bounded by the allowlist. When `expose_api = true`, the API routes are mounted under the normal auth rules, but `/?url=` remains public.
- P0 only has global RPS. Per-IP limiting behind a reverse proxy requires trusted-proxy configuration (P1). Do not use `X-Forwarded-For` without configuring trusted proxies.
- Global/per-host concurrency uses `try_acquire` and fails immediately (503); it does not queue.

## 9. Deployment hardening

- Terminate TLS at the reverse proxy, limit request headers/body/connections, and forward only target paths.
- Run the container/unit as non-root, with a read-only root filesystem and no New Privileges.
- Egress firewall: allow only 443 (and explicitly allowlisted ports); block private, link-local and cloud metadata ranges.
- Inject `ALPS_API_TOKEN` via secret/env; never commit it or bake it into images.
- For the optional public web UI, serve an administrator-controlled `web.dir`, keep the listener behind the TLS terminator, and do not enable `web.expose_api` unless the JSON API must be reachable on the public listener. The `/?url=` route is unauthenticated; the allowlist is the content boundary.
- Monitor 429/503/502/504 and `ip_denied` rates; a spike may indicate abuse or a misconfigured allowlist.

## 10. Signed links

- A signed link is a bearer capability not bound to a client; anyone with the URL can download until it expires.
- Signing cannot bypass the allowlist: `/v1/sign` and `/p/{token}` both run UrlPolicy, and outbound connections still use SecureResolver.
- Tokens are base64url payloads signed with HMAC-SHA256, with an embedded expiry; there is no server-side revocation list. Rotate `ALPS_SIGNING_SECRET` after a leak; deleting `ALPS_SIGNING_SECRET_PREVIOUS` immediately invalidates tokens signed with the old secret.
- Tokens appear only in the `/p/{token}` path; logs record neither the path nor the token.
- `/v1/sign` is Bearer-protected; with `auth.mode = "none"` the issuing endpoint is for trusted networks only.

## 11. Metrics and trusted proxies

- `/metrics` listens on `127.0.0.1:9090` by default; never expose it publicly. Labels use status classes and fixed reason enums, with no host/URL.
- Per-IP limiting uses the TCP peer by default. `X-Forwarded-For` is parsed right-to-left only when the direct peer matches `limits.trusted_proxies`; otherwise the header is ignored entirely to prevent spoofing.
- Shared NAT/egress makes multiple clients share one rate-limit bucket; this is inherent to per-IP limiting.

## 12. Hot reload

- SIGHUP rereads and validates configuration; failures keep the old configuration and never half-apply.
- New requests use the new snapshot; in-flight streams keep the old snapshot and old Limits so security policy or quotas cannot change mid-request.
- Listener address, connection/header limits and the metrics listener address require a restart; hot reload does not rebind ports.
- `web.dir` selects the startup static service and requires a restart; reloads that add, remove or change it are rejected. `web.expose_api` is checked per request by the API guard, so it can be toggled by SIGHUP; hidden paths return 404 before auth or CORS preflight.
- Reload does not reset metrics.

## 13. Path rules, CORS and HTTP/2

- Path prefixes are literal matches on the normalized URL path; `/releases/../secret` and `%2e%2e` are folded by the parser and cannot bypass a prefix. `%2F` is not a separator, so `/releases%2Fsecret` does not match `/releases/`.
- Prefixes should end with `/`: `/releases` also matches `/releases-evil`; that is literal-prefix semantics.
- CORS echoes only exact configured origins, does not support `*`, and never sends `Access-Control-Allow-Credentials`. Preflight is handled before authentication but never contacts upstream or returns file data.
- Enable h2c only behind a trusted reverse proxy; HTTP/2 multiplexing remains subject to global/per-host semaphores and cannot bypass concurrency or byte limits.
- Path rules, CORS and h2 settings follow SIGHUP snapshots; the listen port and HTTP/2 switch require a restart.

## 14. Egress bandwidth shaping

- `limits.max_egress_bytes_per_second` is a global response-body budget, not a per-client quota; one client can still consume most of it.
- The token bucket allows bursts up to max(rate, 64 KiB); use a front proxy/TC when strict smoothing is required.
- Throttling does not change the order of allowlist, SSRF, byte-limit or authentication checks; over-limit requests still fail before waiting.

## 15. Known limitations

- The application cannot distinguish public-range cloud metadata (e.g. Azure 168.63.129.16); rely on the egress firewall.
- Rejecting all of `192.0.0.0/24` affects a few globally reachable addresses; this is a conservative trade-off.
- No DNSSEC validation; security relies on address filtering before connect.
- Upstream hotlink protection or Cookie-requiring sites will reject ALPS; this is an expected compatibility boundary.
- Signed links have no server-side revocation; per-IP limiting is affected by NAT/shared egress.
- The `test-util` feature provides loopback test constructors and development examples; release builds must not enable it.

## 16. Test coverage

The web tests cover static index/assets, CSP and cache headers, hidden API routes, unauthenticated public `/?url=`, bearer-protected `/v1/proxy` when exposed, Range forwarding on the public route, malformed queries and path-traversal rejection.

- Unit: allowlist label boundaries, IDNA/trailing dots, userinfo/fragment/IP literals, IP tables, header filtering, CD sanitization, constant-time auth, permit lifecycle, query fidelity, signing expiry/tampering/rotation, per-IP buckets, trusted-proxy parsing, path-prefix and CORS normalization, egress token bucket.
- Integration: real TCP end-to-end, Range/206/416, 304, HEAD, redirect per-hop/loop, cross-origin rejection, private-IP rejection (403 ip_denied), credential stripping, max response bytes, signing issue/fetch, metrics text, snapshot swap, per-IP limiting, CORS preflight, h2c.
- Command: `cargo test --all-features`.

## 17. Reporting security issues

Contact the maintainers privately. Do not include exploitable details, real signed URLs or credentials in a public issue.
