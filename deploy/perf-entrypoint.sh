#!/bin/sh
set -eu
# Enforce a 256 MiB address-space limit (the perf acceptance target) even when
# the container runtime cannot delegate memory cgroups.
ulimit -v "${ALPS_VM_LIMIT_KB:-262144}"
exec /usr/local/bin/perf_fixture
