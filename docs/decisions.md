# Decision Records (ADR)

This file records ALPS decisions about optional P2 enhancements. It supplements ADR-001..005 in [DESIGN.md](./DESIGN.md).

## ADR-006: No disk file cache

**Status**: Rejected (keeps DESIGN ADR-001)

**Context**: Disk cache was an optional P2 item. Caching requires handling Range segments, conditional requests, expiry/validation, concurrent writes, disk quotas, eviction and persistent state, turning a stateless proxy into a stateful service.

**Decision**: Do not implement a disk cache. Stream upstream responses through, including Range passthrough. Edge caching is delegated to Caddy/Nginx/CDN.

**Consequences**: Repeated downloads consume upstream bandwidth again; in exchange, disk/memory footprint stays small and the security model stays simple.

## ADR-007: OpenTelemetry deferred

**Status**: Deferred

**Context**: OpenTelemetry was an optional P2 item. P1 already provides Prometheus metrics and request-scoped structured logs, which cover the main single-instance operational needs. An OTLP exporter adds dependency surface, binary size, background tasks and provider lifecycle management at process exit.

**Decision**: Do not embed OTLP export yet. Keep stable request log fields and Prometheus metrics so an external collector can convert them to OTel. Revisit as an isolated crate/feature when multi-instance tracing is required.

**Consequences**: No native distributed tracing; cross-instance investigations rely on log correlation and metrics.

## ADR-008: Upstream HTTP/3 deferred

**Status**: Deferred

**Context**: reqwest's `http3` feature is still unstable and requires the extra `reqwest_unstable` cfg; HTTP/3 also needs Alt-Svc or prior-knowledge configuration. ALPS currently uses HTTP/1.1 + HTTP/2 upstream, with optional h2c inbound.

**Decision**: Do not enable reqwest HTTP/3. Terminate HTTP/3 at the edge with Caddy/Nginx/CDN and connect back to ALPS with HTTP/1.1 or h2c.

**Consequences**: Clients can use HTTP/3 to the edge; ALPS-to-upstream remains h1/h2. File download semantics are unchanged.

## ADR-009: Bandwidth shaping uses a global token bucket

**Status**: Implemented

**Context**: Total egress shaping was an optional P2 item. Per-client/per-stream quotas require additional state and fair scheduling; the goal is to cap the instance's total egress, not to provide carrier-grade QoS.

**Decision**: Implement a process-wide `limits.max_egress_bytes_per_second` token bucket shared by all response streams. Bucket capacity is max(rate, 64 KiB); large chunks are split into multiple acquisitions. Throttling happens in `GuardedStream` and returns Pending while waiting, preserving backpressure.

**Consequences**: One client can consume most of the global budget. Strict fairness or per-tenant quotas belong at the edge/gateway layer.

## ADR-010: Inbound HTTP/2 is h2c prior-knowledge only

**Status**: Implemented

**Context**: ALPS usually runs behind a TLS-terminating proxy; cleartext h2 does not need h2c Upgrade negotiation, and the hyper auto builder supports prior knowledge well.

**Decision**: `server.http2 = true` enables h2c prior knowledge while retaining HTTP/1.1. Do not implement HTTP/1.1 Upgrade: h2c or inbound TLS.

**Consequences**: Upstream callers must explicitly use h2c prior knowledge; ordinary h1 clients still work.
