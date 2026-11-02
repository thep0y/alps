#!/usr/bin/env bash
set -euo pipefail

# Local throughput/RSS check.
#
# Requirements:
#   - an ALPS instance with a real (non-loopback) upstream in its allowlist
#   - ALPS_URL, ALPS_TOKEN and TARGET_URL exported
#   - optionally oha (https://github.com/hatoo/oha) for concurrent load
#
# Example:
#   ALPS_URL=http://127.0.0.1:8080 \
#   ALPS_TOKEN=... \
#   TARGET_URL=https://files.example.org/1GiB.bin \
#   ./scripts/bench.sh
#
# RSS is sampled from the local process when ALPS_PID is set.

ALPS_URL="${ALPS_URL:-http://127.0.0.1:8080}"
TARGET_URL="${TARGET_URL:?set TARGET_URL to an allowlisted https file}"
ALPS_TOKEN="${ALPS_TOKEN:-}"
CONCURRENCY="${CONCURRENCY:-100}"
PROXY_URL="${ALPS_URL}/v1/proxy?url=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1], safe=""))' "${TARGET_URL}")"

sample_rss() {
  if [[ -n "${ALPS_PID:-}" ]] && [[ -r "/proc/${ALPS_PID}/status" ]]; then
    awk '/VmRSS/{print $2" kB"}' "/proc/${ALPS_PID}/status"
  fi
}

echo "target: ${TARGET_URL}"
echo "proxy:  ${PROXY_URL}"
echo "rss before: $(sample_rss)"

if command -v oha >/dev/null 2>&1; then
  args=(-n "${REQUESTS:-1000}" -c "${CONCURRENCY}" -H "Authorization: Bearer ${ALPS_TOKEN}")
  oha "${args[@]}" "${PROXY_URL}"
else
  echo "oha not found; running a single timed curl"
  /usr/bin/time -v curl -sS -o /dev/null \
    -H "Authorization: Bearer ${ALPS_TOKEN}" "${PROXY_URL}" 2>&1 | grep -E "Elapsed|Maximum resident" || true
fi

echo "rss after: $(sample_rss)"
