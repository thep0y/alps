# ALPS — AllowList Proxy Service

ALPS is a lightweight, stateless file proxy for low-resource servers, restricted by a domain allowlist. A client hands ALPS an HTTP(S) URL; ALPS validates the destination, follows redirects hop by hop, and streams the upstream response back to the client.

ALPS is not a general-purpose forward proxy and not a transparent reverse proxy: only `GET` and `HEAD` are accepted, only allowlisted destinations are permitted, and non-public IP addresses are rejected by default.

## Status

- **P0 implemented**: allowlist, URL validation, SSRF/DNS-IP defenses, manual redirect handling, GET/HEAD, Range/206/304/416 passthrough, streaming and backpressure, concurrency/rate limits, timeouts, health checks, TOML configuration, structured logging, graceful shutdown, Docker/systemd/CI.
- **P1 implemented**: HMAC signed links (`/p/{token}` and `/v1/sign`), Prometheus metrics on a separate admin listener, SIGHUP hot reload, per-client-IP rate limiting behind trusted proxies.
- **P2 implemented (first batch)**: allowlist path prefixes, trusted-origin CORS including preflight, inbound HTTP/2 (h2c), global egress bandwidth shaping.
- **P2 optional items not implemented**: disk cache, OpenTelemetry, upstream HTTP/3. See [docs/decisions.md](docs/decisions.md) for the rationale.
- **Container validation**: production image build, read-only/non-root execution, metrics/sign smoke tests, and 100-concurrent/1 GiB downloads under a 1 CPU + 256 MiB address-space limit were validated with podman. See [docs/implementation-notes.md](docs/implementation-notes.md).

## Features

- Deny by default: an empty allowlist rejects everything; HTTPS only by default; IP literals, userinfo, fragments and control characters are rejected.
- Validation on every hop: ALPS owns the redirect chain and revalidates scheme, host, port and IP for each new URL, with loop detection and a hop limit.
- DNS rebinding defense: a custom `reqwest` resolver only hands connector-approved IPs to the transport and system proxies are disabled.
- File semantics: streaming body (no collect), backpressure, `Range` / `If-Range` / `If-None-Match` / `If-Modified-Since`, 206/304/416 passthrough.
- Response policy: only file-semantics headers are forwarded; `Set-Cookie`, `Location` and hop-by-hop headers are stripped. `Content-Disposition` is the only header replaced: active content defaults to `attachment`, while safe text types (plain text, CSV, Markdown, JSON, YAML, PAC) are inline.
- Resource governance: global/per-host concurrency semaphores, global RPS, connection/header/read-idle timeouts, maximum URL/response bytes, inbound header and connection limits, and an optional global egress token bucket (`limits.max_egress_bytes_per_second`).
- Observability: request-scoped structured logs without full URLs, query strings or tokens; stable error codes.
- Low footprint: one shared `reqwest::Client`; permits are released with the body; client disconnects cancel the upstream stream.
- Signed downloads: `POST /v1/sign` issues time-limited HMAC links; `GET/HEAD /p/{token}` needs no Bearer token. Signed URLs are still rechecked against the allowlist and IP policy.
- Metrics: `/metrics` exposes requests, rejections, redirects, bytes, failures and active streams with bounded label cardinality.
- Hot reload: SIGHUP rereads configuration and atomically swaps the runtime snapshot; invalid reloads keep the old configuration and in-flight requests keep their old snapshot.
- Path rules: each allowlist entry may restrict access to literal path prefixes such as `/releases/`; redirects and signed links are constrained too. Dot segments are normalized before matching.
- CORS: only exact origins listed in the configuration receive CORS headers; `OPTIONS` preflight is handled. Disabled by default; wildcards are rejected.
- Inbound HTTP/2: optional h2c (prior knowledge) for deployments where Caddy/Nginx terminates TLS. HTTP/1.1 remains the default.
- Egress shaping: `limits.max_egress_bytes_per_second` is a global response-body token bucket; 0 disables it. Excess chunks wait asynchronously inside the stream, propagating backpressure upstream.

## Quick start

```bash
cargo build --release

export ALPS_API_TOKEN="$(openssl rand -hex 24)"
cp config/alps.example.toml config/alps.toml
# Edit [[allowlist]] in config/alps.toml

./target/release/alps --config config/alps.toml
```

Make a request (the target must be allowlisted):

```bash
curl -H "Authorization: Bearer $ALPS_API_TOKEN" \
  "http://127.0.0.1:8080/v1/proxy?url=https%3A%2F%2Ffiles.example.org%2Fapp.tar.gz"

# Range
curl -H "Authorization: Bearer $ALPS_API_TOKEN" -H "Range: bytes=0-1023" \
  -o part.bin \
  "http://127.0.0.1:8080/v1/proxy?url=https%3A%2F%2Ffiles.example.org%2Fapp.tar.gz"
```

> The `url` parameter must be percent-encoded. ALPS does not treat `+` as a space and does not reorder query parameters.

## API

| Method | Path | Description |
|---|---|---|
| GET | `/v1/proxy?url=<encoded_url>` | Proxy a file and follow redirects server-side |
| HEAD | `/v1/proxy?url=<encoded_url>` | Fetch file metadata without a body |
| POST | `/v1/sign` | Bearer-protected; JSON `{"url":"...","expires_in_secs":300,"max_bytes":0}` returns `{"token":"...","expires_at":...}` |
| GET/HEAD | `/p/{token}` | Proxy a file through a signed link; invalid/expired tokens return 401/403 |
| GET | `/healthz` | Process liveness |
| GET | `/readyz` | Startup/allowlist status (counts only) |
| GET | `/metrics` | Admin listener only; Prometheus text format |

When CORS is enabled, allowed origins receive a 204 preflight for `OPTIONS /v1/proxy`, `/v1/sign` or `/p/{token}`; other methods return `405` without contacting upstream. Set `server.http2 = true` to accept h2c (prior knowledge) on the main listener. Error responses look like:

```json
{"error":{"code":"destination_denied","message":"the destination is not allowed","request_id":"..."}}
```

See [docs/architecture.md](docs/architecture.md) for the complete error-code list.

## Configuration

Configuration is TOML, validated at startup; unknown fields fail startup. Secrets are read only from environment variables:

- `ALPS_API_TOKEN`: required when `auth.mode = "bearer"`; at least 16 bytes.
- `ALPS_SIGNING_SECRET`: required when `signing.enabled = true`; base64 or at least 32 raw bytes.
- `ALPS_SIGNING_SECRET_PREVIOUS`: optional; used to validate old tokens during key rotation.
- `ALPS_CONFIG`: optional alternative to `--config`.

When neither is set, startup uses the first existing file in this order: `$HOME/.coinfig/alps/alps.toml`, `$HOME/.alps.toml`, then `<binary directory>/alps.toml`.

Additional blocks: `[cors] allowed_origins` (exact origins; empty disables CORS), `[server] http2`, per-rule `path_prefixes`, and `limits.max_egress_bytes_per_second`.

Safety switches that cannot be disabled (startup fails if set to `false`): `security.block_non_global_ips`, `block_ip_literal_hosts`, `no_system_proxy`, `logging.redact_url_query`; `default_content_disposition` only accepts `attachment`.

`auth.mode = "none"` is allowed only on a loopback listener, or on a non-loopback listener when `security.trusted_network_only = true` is explicitly set.

See [docs/configuration.md](docs/configuration.md) for the complete field reference, constraints and examples, and `config/alps.example.toml` for an annotated configuration.

## Security model

- The allowlist is the security boundary: only administrator-listed domains, ports and schemes are reachable. There is no "trust any redirect after the first hop" mode.
- Outbound IPs must be globally routable: loopback, private, link-local, CGNAT, multicast, reserved, documentation, IPv4-mapped IPv6, NAT64, 6to4, ULA and Teredo ranges are rejected; IPv6 is limited to `2000::/3`.
- Every redirect and every new connection is revalidated; connections use the verified addresses returned by SecureResolver.
- Client `Host`, `Cookie`, `Authorization` and `X-Forwarded-*` headers are not forwarded; `Referer` is not forged.
- Logs omit full URLs, query strings, signing parameters and tokens; reqwest errors have their URL removed before logging.
- Signed links are bearer capabilities: default TTL 5 minutes, maximum TTL configurable; signing never bypasses allowlist/IP checks. During rotation, old links remain valid while the previous secret is configured.
- Per-IP rate limiting trusts `X-Forwarded-For` only when the direct peer matches `limits.trusted_proxies`; otherwise the TCP peer address is used.
- Path prefixes are literal matches against the normalized URL path; no wildcards or regex. Prefixes should end with `/` to avoid `/releases` matching `/releases-evil`.
- CORS echoes only exact configured origins and never sends `Access-Control-Allow-Credentials`. Do not configure `*` (it is rejected).

See [docs/security.md](docs/security.md) for details.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --locked
cargo audit --no-fetch   # requires a local RustSec advisory database
```

The release profile uses thin LTO, `codegen-units = 1` and `strip = "symbols"`, while keeping `panic = "unwind"` so a panicking task does not abort the proxy process. The release binary shrank from 12.3 MB to about 8.4 MB on this machine.

Integration tests use the `test-util` feature, which provides loopback-permitting policy and a static DNS backend only in test builds; release binaries do not contain these constructors.

Local load testing: `cargo run --features test-util --example dev_proxy` starts a loopback-only development proxy on `127.0.0.1:18080` (not included in release builds). See [tests/integration_tests.rs](tests/integration_tests.rs) for the proxy chain. Local 1 CPU / 256 MiB benchmarks, environment and reproduction commands are in [docs/benchmarks.md](docs/benchmarks.md).

## Deployment

- Single binary; containers run as non-root with a read-only root filesystem. See `deploy/Dockerfile`.
- Image builds accept `RUST_IMAGE` / `RUNTIME_IMAGE` / `RUNTIME_USER` build arguments so base images can be replaced in environments without access to the default registries:
  ```bash
  podman build -t alps .
  podman run --read-only --tmpfs /tmp --cap-drop=ALL -p 8080:8080 \
    -v "$PWD/config/alps.toml:/etc/alps/alps.toml:ro" \
    -e ALPS_API_TOKEN=... alps
  ```
- The performance/resource test image is `deploy/Dockerfile.perf` plus `deploy/perf-entrypoint.sh` (test-util fixture; never deploy it).
- A hardened systemd unit example is `deploy/alps.service`.
- Terminate TLS with Caddy/Nginx and limit ingress connections and request bodies.
- Also restrict outbound traffic to private networks and cloud metadata endpoints at the deployment layer (defense in depth).
- The metrics listener defaults to `127.0.0.1:9090`; do not expose it publicly.
- Changing listener address/port, connection limits or header limits requires a restart; SIGHUP only hot-swaps policy, allowlist, auth, signing keys, limits and the upstream client configuration.

## CI and release

- The `warm-cache` workflow saves separate debug and release Cargo/target caches only on pushes to `main`; the pull-request validation workflows restore them without overwriting. The release profile's thin LTO cache is never mixed with the debug cache.
- Run the `release` workflow manually with `bump = patch|minor|major` and `kind = stable|alpha|beta`. `bump` chooses the target core and `kind` chooses the release type, so a pre-release never guesses whether it belongs to the next patch, minor or major. It updates `Cargo.toml` and `Cargo.lock`, validates, builds x86_64 and aarch64 in parallel, commits the version bump to `main`, creates a `v<version>` tag, and publishes a GitHub release with PR titles classified into feature, fix, docs, CI and other sections. Alpha/beta builds are marked as pre-releases.
- `dry_run` performs the bump, validation and builds without committing or publishing.
- Branch protection may require a `RELEASE_TOKEN` secret (contents: write) or an explicit Actions bypass for the version-bump commit.
- PR validation is split into independent workflows: `lint`, `test`, `release-build`, `msrv`, `cross`, `audit` and `docker`. Require the relevant checks in branch protection.
- Label changes do not trigger any validation workflow. Auto-merge and post-merge head-branch deletion are repository settings, not workflows; enable auto-merge per PR through GitHub if needed.

See [docs/release.md](docs/release.md) for cache keys, bump semantics, required permissions and recovery steps.

## License

MIT, see [LICENSE](LICENSE).
