# ALPS web UI

A SolidJS + Tailwind download page for ALPS. Node is only needed to build the
static assets; the deployed service is the ALPS binary itself, which serves the
built files from a directory and handles `GET/HEAD /?url=...` directly.

There is no second HTTP server and no production Node process.

## How it works

```text
browser
  |  HTTPS :443
  v
Caddy / Nginx (TLS, HSTS)
  |  HTTP 127.0.0.1:8080
  v
ALPS [web] dir = /opt/alps/web/dist
  |  GET /?url=... -> internal proxy pipeline
  v
allowlist + DNS/IP policy + upstream
```

- `GET /` serves `index.html`.
- `GET/HEAD /?url=<percent-encoded-url>` runs the same validation, redirect,
  concurrency and streaming pipeline as `/v1/proxy`, but without a bearer
  token. The browser cannot hold the private API token, so this public route is
  unauthenticated by design.
- Other paths are served by a static file service. With the default
  `web.expose_api = false`, the JSON API routes are not reachable:
  `/v1/proxy`, `/v1/sign` and `/p/{token}` return 404.
- `/healthz` and `/readyz` remain available for operational checks.

The ALPS allowlist remains the security boundary. This mode is suitable for a
public download page over a deliberately limited set of destinations.

## Build

```bash
cd web
npm ci
npm run build
```

The result is `web/dist/`:

```text
dist/
  index.html
  assets/index-*.js
  assets/index-*.css
  favicon.svg
```

Node 22.12 or newer is required to build. The build is not needed at runtime.

Validation:

```bash
npm run typecheck
npm test
npm run build
npm run check   # all three
```

## Enable the UI in ALPS

Point ALPS at the build directory with the `[web]` section:

```toml
[server]
listen = "127.0.0.1:8080"

[web]
# Must contain index.html. Relative paths are resolved from the process
# working directory and canonicalized at startup.
dir = "/opt/alps/web/dist"

# Default false. When false, /v1/proxy, /v1/sign and /p/{token} are hidden.
expose_api = false
```

Alternatively use the CLI or environment variable:

```bash
alps --config /etc/alps/alps.toml --static-dir /opt/alps/web/dist

# or
ALPS_STATIC_DIR=/opt/alps/web/dist alps --config /etc/alps/alps.toml
```

Precedence is `--static-dir` > `ALPS_STATIC_DIR` > `web.dir`.

With `web.dir` set and `expose_api = false`:

- `auth.mode = "bearer"` does not require `ALPS_API_TOKEN`, because no
  bearer-protected route is mounted. `Auth` is disabled internally.
- `auth.mode = "none"` is allowed even on a non-loopback listener; the public
  `/?url=` route would be unauthenticated anyway.
- Set `expose_api = true` if you also need the JSON API. In that mode the
  normal auth rules apply: bearer mode needs `ALPS_API_TOKEN`, and
  `auth.mode = "none"` on a public listener requires
  `security.trusted_network_only = true`. The public `/?url=` route still
  bypasses bearer auth.

`web.dir` is a startup setting; restart ALPS after changing it, and a
SIGHUP that tries to add, remove or change the directory is rejected.
`web.expose_api` is checked per request and can be toggled by SIGHUP; hidden
API paths return 404 before authentication or CORS preflight.

## Security properties

- The private API token, if any, is never sent to the browser.
- API routes are hidden by default, so `/v1` and `/p/` probing returns 404.
- The public proxy still applies the allowlist, scheme/port checks, userinfo/
  fragment/IP-literal rejection, DNS/IP filtering, redirect revalidation,
  concurrency limits, byte limits and timeouts.
- Public proxy responses override caching with `Cache-Control: private,
  no-store` and add `X-Robots-Tag: noindex, nofollow`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY` and `Cross-Origin-Resource-Policy: same-origin`.
- Static responses add:
  - `Content-Security-Policy` for HTML
  - `X-Content-Type-Options: nosniff`
  - `Referrer-Policy: no-referrer`
  - `X-Frame-Options: DENY`
  - `Cross-Origin-Opener-Policy: same-origin`
  - `Cross-Origin-Resource-Policy: same-origin`
  - `Permissions-Policy`
  - `X-Robots-Tag: noindex, nofollow`
- `ServeDir` handles static path resolution, range requests and conditional
  requests; directory listings are disabled.
- No ALPS version header is added. Remove the TLS terminator's own `Server`
  header with `header -Server` if that matters to you.

The page is intentionally public. If the service must be private, put an
authentication layer (SSO, mTLS, basic auth) in front of ALPS, or leave
`web.dir` unset and use the token-protected API only.

## Development

Build the UI once, run an ALPS instance with the static directory, then start
Vite:

```bash
cd web
npm run build            # ALPS serves this directory
npm run dev              # http://127.0.0.1:5173/
```

Vite proxies `/?url=...` to `http://127.0.0.1:8080` so the page behaves like
production. If ALPS listens elsewhere, update `server.proxy` in
[`vite.config.ts`](vite.config.ts). The proxy target is only for development.

## Deployment

### Release artifact

Each ALPS release publishes `alps-web-<tag>.tar.gz` separately from the
platform binaries. It contains `dist/`, this README and the license:

```bash
tar -xzf alps-web-v0.1.2.tar.gz -C /opt/alps/web
# [web] dir = "/opt/alps/web/dist"
```

The archive has no Node runtime dependency and no version string in the built
HTML/JS; the artifact name identifies the release.

### Caddy

Use [`deploy/Caddyfile.example`](../deploy/Caddyfile.example) as a starting
point. Caddy terminates TLS on 443 and forwards to ALPS on loopback. Configure
`limits.trusted_proxies = ["127.0.0.1/32"]` if you want per-client rate limiting
to use the forwarded address.

### Docker

The ALPS image stays a single Rust binary; mount the built directory and set
`web.dir`:

```bash
podman build -t alps .
podman run --read-only --tmpfs /tmp --cap-drop=ALL -p 8080:8080 \
  -v "$PWD/config/alps.toml:/etc/alps/alps.toml:ro" \
  -v "$PWD/web/dist:/opt/alps/web:ro" \
  alps
```

Use `[web] dir = "/opt/alps/web"` in the mounted config. No Node runtime is
installed in the image. If `web.expose_api = true` and the API uses bearer
auth, add `-e ALPS_API_TOKEN=...` as before.

## Limitations

- The public `/?url=` route has no user authentication; the allowlist is the
  content boundary.
- No upload, WebSocket, signed-link UI or SPA client-side routes.
- The UI ships in English. Copy lives in [`src/App.tsx`](src/App.tsx) and
  [`src/lib/proxy.ts`](src/lib/proxy.ts).
