# Sweep-009 — `cave_kernel::observability`

**Author:** 2026-05-12 close-out
**Branch:** `claude/gracious-banach-9be8eb`
**Status:** **Landed (primitive + tests).** Live adoption in
cave-mesh / cave-gateway / cave-portal-api deferred (recon below).

## What landed

`crates/cave-kernel/src/observability.rs` — 358 LOC + 16 unit tests.

Three shapes, OTEL/Prometheus-faithful:

* **`LogRecord`** — five-level `LogLevel` (Trace/Debug/Info/Warn/Error,
  matching `tracing::Level`), `timestamp_unix_ms`, `target`,
  `message`, and a `BTreeMap<String, String>` of fields (sorted for
  deterministic JSON). `LogRecord::now(level, target, message)` is
  the common construction path; `.with_field(k, v)` chains attach
  fluently.
* **`Tracer` trait + `SpanGuard`** — span surface reduced to what
  every cave module actually needs: open, tag with attributes,
  drop-records-end. Matches `tracing::span::EnteredSpan` so a caller
  switching from `tracing` keeps the `let _g = tracer.start_span(...)`
  pattern. `NoopTracer` ships for tests / local dev.
* **`Metric` enum** — three OpenMetrics instrument kinds (Counter /
  Gauge / Histogram). `Histogram` carries `buckets_le_ms` + `counts`
  + `sum_ms` inline so a renderer can emit `_bucket{le="..."}` lines
  directly. `inc_by` / `set` / `observe_ms` enforce the instrument-kind
  semantics — `set` on a counter panics (matches `prometheus::IntCounter`).

## What is NOT in scope

* No exporter wiring. The kernel ships shapes; transport (OTLP /
  Prometheus scrape endpoint / log forwarder) is per-crate.
* No tracing propagator (`traceparent` header parsing) — call sites
  that need it pull the existing `tracing_opentelemetry` crate.

## Adoption — pilot landed, full migration deferred

The brief named cave-mesh, cave-gateway, cave-portal-api as target
adopters. All three currently emit through the `tracing` macros
directly. Migrating their full instrumentation surface to the kernel
`Tracer` trait would re-route hundreds of `tracing::info!` /
`tracing::span!` call sites — way too big for one PR.

### Landed: cave-portal-api LogEntry → kernel LogRecord bridge

`crates/cave-portal-api/src/routes/logs.rs` now exposes:

* `LogLevel::to_kernel()` — variant-by-variant cast to
  `cave_kernel::observability::LogLevel`. Preserves ordering.
* `LogEntry::as_kernel_record()` — converts a portal-api log entry
  into a `cave_kernel::observability::LogRecord` with `tenant` /
  `instance` / `id` surfacing as kernel record fields and the
  portal's `app` becoming the kernel record's `target`. Best-effort
  RFC3339 timestamp parsing (falls back to 0 on malformed input
  rather than panicking).

The bridge means an external log forwarder can ingest portal-api
entries with the same `LogRecord` parser it uses for every other
cave module — the kernel shape is the single wire contract.

3 new tests (`level_to_kernel_preserves_ordering`,
`as_kernel_record_surfaces_portal_fields_as_kernel_fields`,
`as_kernel_record_handles_malformed_timestamp`); portal-api logs
suite 18 → 21.

### Deferred

* `cave-mesh` — per-request span migration touches `proxy.rs`,
  `xds.rs`, `traffic.rs` instrumentation; 1759 tests would need to
  rebaseline against the new tracer.
* `cave-gateway` — auth + circuit-breaker spans, ~600 tests to
  re-baseline.

Both stay on `tracing::*` until a dedicated migration PR lands.

## Tests

`cargo test -p cave-kernel --lib observability::` — 16 passed.
`cargo test -p cave-portal-api --lib routes::logs::` — 21 passed
(was 18 + 3 bridge tests).
