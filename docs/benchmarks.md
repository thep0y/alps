# ALPS Local Benchmark Record

Date: 2026-11-02
Subject: ALPS current worktree (P0 + P1 + P2 first batch), release build
Scope: single-host loopback smoke-level benchmark, not formal capacity planning or a cross-network benchmark.

## 1. Goal and scope

Record streaming proxy behavior under an approximate "1 CPU + 256 MiB address space" constraint, focusing on:

- whether 100 concurrent 32 MiB downloads OOM and whether peak RSS stays below the 128 MiB target;
- whether a single 1 GiB download grows RSS linearly with file size;
- whether active streams and memory fall back after clients disconnect mid-download;
- rough loopback throughput as a reference for later target-environment tests.

Not measured: TLS, real public-network RTT, packet loss/retransmission, p95 time-to-first-byte, disk cache hits, aarch64.

## 2. Environment

| Item | Value |
|---|---|
| Host | WSL2, Linux 6.6.87.2-microsoft-standard-WSL2 |
| Distribution | Debian GNU/Linux 13 (trixie) |
| CPU | AMD Ryzen 7 8845H, 8 cores / 16 threads (16 logical available) |
| Memory | 15 GiB (~10 GiB available during tests) |
| Toolchain | rustc 1.99.1 / cargo 1.99.1 |
| Build | `cargo build --release --locked` |
| Container | podman 5.4.2, rootless, cgroup v2 + cgroupfs |
| Performance image | `deploy/Dockerfile.perf` + `deploy/perf-entrypoint.sh` |
| Client | host curl 8.14.1, 100 concurrent processes |
| Upstream | container-local 127.0.0.1:19090, 1 MiB zero-filled stream, known Content-Length |
| Network | loopback + podman port publishing (18080 proxy / 18081 stats) |

### Resource limiting

Rootless podman on this host has no cgroup delegation, so `--cpus=1 --memory=256m` is not enforced by the kernel. Instead:

- CPU: `taskset -c 0` pins one CPU (inside the container `nproc=1`, `Cpus_allowed_list: 0`);
- Memory: the container entrypoint runs `ulimit -v 262144` (RLIMIT_AS = 256 MiB, stricter than RSS);
- This limit was verified to reject a 300 MiB allocation.

## 3. Measurement method

The performance image runs `examples/perf_fixture.rs`: one process contains the streaming upstream, the ALPS proxy and a `/stats` endpoint. `/stats` reads `VmRSS`, `VmHWM`, `VmPeak` and active_streams from `/proc/self/status`.

- **Peak RSS**: kernel `VmHWM` for the whole fixture process (upstream + proxy);
- **VmPeak**: peak virtual address space, including mmap/thread-stack reservations; not RSS;
- **Duration**: host curl wall time for the complete download;
- **Failures**: number of curl processes with non-zero exit codes;
- **Throughput**: total downloaded bytes / duration, roughly converted to MiB/s.

Every container run waits for `/healthz` to return 200 before load generation, so startup races are not counted as failures.

## 4. Results

### 4.1 Idle

| Metric | Value |
|---|---|
| RSS | 7.5 MiB |
| Peak RSS (VmHWM) | 7.5 MiB |
| VmPeak | 143.4 MiB |
| active_streams | 0 |

### 4.2 100 concurrent × 32 MiB (container, 1 CPU + 256 MiB AS)

Three independent starts:

| Run | Duration | Failures | Peak RSS | RSS at end | VmPeak |
|---|---|---|---|---|---|
| 1 | 6.84 s | 0 | 91.2 MiB | 66.0 MiB | 209.7 MiB |
| 2 | 6.06 s | 0 | 89.9 MiB | 67.3 MiB | 208.7 MiB |
| 3 | 5.46 s | 0 | 91.0 MiB | 71.3 MiB | 208.7 MiB |

Median: **6.06 s / 91.0 MiB peak RSS**, aggregate throughput about **528 MiB/s**. All three runs had zero failures and no OOM.

Note: earlier batches under higher host load observed 5.56–8.19 s and 69.5–95.4 MiB peak RSS. Host load strongly affects results; focus on the order of magnitude and resource ceiling, not a single decimal.

### 4.3 Single 1 GiB file (container)

| Run | Duration | Peak RSS | VmPeak |
|---|---|---|---|
| 1 | 0.900 s | 9.64 MiB | 143.4 MiB |
| 2 | 0.802 s | 9.60 MiB | 143.4 MiB |
| 3 | 0.936 s | 9.41 MiB | 143.4 MiB |

Median: **0.900 s / 9.60 MiB peak RSS**, about **1138 MiB/s**. RSS did not grow linearly with file size relative to the 32 MiB case, confirming the streaming path does not buffer whole files. This is a loopback ceiling reference, not public-network throughput.

### 4.4 Disconnect cleanup (50 × 1 GiB downloads, clients terminated after 250 ms)

| Metric | Value |
|---|---|
| active_streams 1 s after disconnect | 0 |
| Peak RSS | 32.1 MiB |
| RSS at end | 11.1 MiB |
| Process state | Healthy, no OOM |

### 4.5 Host-only control (same taskset + ulimit)

| Scenario | Duration | Failures | Peak RSS | VmPeak |
|---|---|---|---|---|
| 100 × 32 MiB | 9.84 s | 0 | 72.3 MiB | 143.7 MiB |
| 1 GiB | 3.49 s | 0 | 10.1 MiB | 143.7 MiB |

Host-only and container results are in the same order of magnitude. Differences come mainly from port publishing, container networking and instantaneous host load; do not read them as the container being faster or slower.

## 5. Interpretation

- Under the approximate 1 CPU + 256 MiB address-space limit, neither 100 concurrent 32 MiB downloads nor a 1 GiB file OOMed; peak RSS stayed below the 128 MiB target.
- 1 GiB RSS is close to 32 MiB RSS, consistent with the streaming + backpressure design; no whole-file collect.
- 50 downloads terminated mid-transfer drove active_streams to zero within 1 s and RSS fell, showing permits and upstream streams are released with the body.
- 100-concurrent duration is sensitive to host load (5.5–9.8 s, ~70–95 MiB peak RSS on this host); this record verifies resource ceilings, not a precise throughput promise.
- These numbers use loopback HTTP and the test-util permissive resolver; TLS, real public networks and strict DNS filtering will change CPU/latency behavior.

## 6. Reproduction

```bash
# 1) Build the test-only performance image (from the repository root)
podman build --pull=never --network=host -f deploy/Dockerfile.perf -t alps-perf .

# 2) One CPU + 256 MiB address space (--memory is declarative; the entrypoint's ulimit -v enforces it)
taskset -c 0 podman run --rm --name alps-bench \
  --cpus=1 --memory=256m \
  -p 18080:18080 -p 18081:18081 \
  -e PERF_FILE_MB=32 alps-perf

# 3) In another terminal: idle/peak stats
curl -s http://127.0.0.1:18081/stats

# 4) 100 concurrent downloads
URL='http://127.0.0.1:18080/v1/proxy?url=http%3A%2F%2Fupstream.test%3A19090%2Ffile'
for i in $(seq 1 100); do curl -sS -o /dev/null "$URL" & done
wait
curl -s http://127.0.0.1:18081/stats

# 5) 1 GiB: restart the container with PERF_FILE_MB=1024
# 6) Disconnect: start 50 curls, kill them after 250 ms, then inspect active_streams via /stats
```

## 7. Limitations

- Rootless podman has no cgroup delegation; `--cpus/--memory` are not truly enforced. CPU uses affinity and memory uses RLIMIT_AS. RLIMIT_AS is stricter than a 256 MiB RSS cap but is not a substitute for real cgroup quotas.
- Upstream and proxy share one container/process, so peak RSS includes both; the upstream uses zero-filled memory and does not emulate real disk/network.
- TLS, HTTP/2 upstream, public RTT, packet loss, mixed concurrent workloads and p95 latency were not measured.
- The actual precision of `limits.max_egress_bytes_per_second` was not quantified; the egress token bucket's wait logic is covered only by paused-clock unit tests.
- Clients are host curl processes; 100 concurrent clients consume host CPU, and the single-CPU container shares the machine with a 16-core host.
- Results vary with host load; this record keeps three same-batch runs and the median rather than reporting only the best number.

## 8. Next steps

- Re-run on a host with cgroup v2 delegation using real `--cpus=1 --memory=256m`, and add a TLS upstream.
- Add p95 time-to-first-byte/total time, error rate and connection reuse metrics; turn repeated steps into a script.
- Merge with the podman validation in `docs/implementation-notes.md` and the long-running load workflow in `scripts/bench.sh`.
