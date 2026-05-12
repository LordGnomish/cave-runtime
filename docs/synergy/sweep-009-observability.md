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

## Adoption — deferred

The brief named cave-mesh, cave-gateway, cave-portal-api as target
adopters. All three currently emit through the `tracing` macros
directly. Migrating to the kernel `Tracer` trait would re-route
their existing instrumentation through an indirection layer; the
sweep was scoped narrowly enough that doing both the primitive AND
the migration in one commit would have bloated the diff past honest
reviewability.

The primitive is the prerequisite; the adoption ticket can land
next, gated on each crate's per-PR test surface:

* `cave-mesh` — 1759 tests; per-request span migration touches
  `proxy.rs`, `xds.rs`, `traffic.rs` instrumentation.
* `cave-gateway` — auth + circuit-breaker spans, ~600 tests to
  re-baseline.
* `cave-portal-api` — audit log records (already structured —
  shortest migration of the three).

## Tests

`cargo test -p cave-kernel --lib observability::` — 16 passed.
