#!/usr/bin/env bash
set -euo pipefail

BIN="${1:-target/debug/alps}"
PORT="${PORT:-$((18080 + RANDOM % 500))}"
METRICS_PORT="${METRICS_PORT:-$((PORT + 1))}"
export ALPS_SIGNING_SECRET="${ALPS_SIGNING_SECRET:-$(openssl rand -base64 32)}"
DIR="$(mktemp -d)"
PID=""

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "${PID}" 2>/dev/null; then
    kill -TERM "${PID}" 2>/dev/null || true
    wait "${PID}" 2>/dev/null || true
  fi
  rm -rf "${DIR}"
}
trap cleanup EXIT

cat > "${DIR}/alps.toml" <<EOF
[server]
listen = "127.0.0.1:${PORT}"
shutdown_grace_secs = 5

[auth]
mode = "none"

[security]
trusted_network_only = true

[signing]
enabled = true
default_ttl_secs = 60
max_ttl_secs = 3600

[metrics]
enabled = true
listen = "127.0.0.1:${METRICS_PORT}"

[[allowlist]]
host = "example.org"
schemes = ["https"]
ports = [443]

[logging]
level = "info"
format = "text"
EOF

"${BIN}" --config "${DIR}/alps.toml" --check-config >/dev/null

"${BIN}" --config "${DIR}/alps.toml" >"${DIR}/log" 2>&1 &
PID=$!

ready=0
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.05
done
if [[ "${ready}" != "1" ]]; then
  echo "server did not become ready" >&2
  cat "${DIR}/log" >&2
  exit 1
fi

code() {
  curl -s -o /dev/null -w '%{http_code}' "$@"
}

[[ "$(code "http://127.0.0.1:${PORT}/healthz")" == "200" ]]
[[ "$(code "http://127.0.0.1:${PORT}/readyz")" == "200" ]]
[[ "$(code -X POST "http://127.0.0.1:${PORT}/v1/proxy")" == "405" ]]
[[ "$(code "http://127.0.0.1:${PORT}/v1/proxy")" == "400" ]]
[[ "$(code "http://127.0.0.1:${PORT}/v1/proxy?url=http%3A%2F%2F127.0.0.1%2F")" == "403" ]]

[[ "$(code "http://127.0.0.1:${METRICS_PORT}/metrics")" == "200" ]]
curl -fsS "http://127.0.0.1:${METRICS_PORT}/metrics" | grep -q "alps_requests_total"

SIGN_CODE="$(curl -s -o "${DIR}/sign.json" -w '%{http_code}' \
  -X POST -H 'Content-Type: application/json' \
  --data '{"url":"https://example.org/file","expires_in_secs":60}' \
  "http://127.0.0.1:${PORT}/v1/sign")"
[[ "${SIGN_CODE}" == "200" ]]
grep -q '"token"' "${DIR}/sign.json"

cat >> "${DIR}/alps.toml" <<EOF

[[allowlist]]
host = "example.com"
schemes = ["https"]
ports = [443]
EOF
kill -HUP "${PID}"
reloaded=0
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${PORT}/readyz" | grep -q '"allowlist_rules":2'; then
    reloaded=1
    break
  fi
  sleep 0.05
done
if [[ "${reloaded}" != "1" ]]; then
  echo "SIGHUP reload did not take effect" >&2
  cat "${DIR}/log" >&2
  exit 1
fi

kill -TERM "${PID}"
wait "${PID}"
PID=""
echo "smoke test passed (port ${PORT})"
