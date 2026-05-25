<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-trace — Charter v2 Parity Report

**Upstream:** [jaegertracing/jaeger](https://github.com/jaegertracing/jaeger) pinned **v1.52.0**
(`source_sha = 9866eba85aed1b0a66a77c8c6928a372edc5040f`).
**Upstream license:** Apache-2.0 (Copyright 2023 The Jaeger Authors).
**cave-trace license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-21.

cave-trace closes the **fifth and final leg** of the cave-runtime observability
ring (metrics ✓ logs ✓ dashboard ✓ oncall ✓ + **trace** ✓), providing a Jaeger
v1-compatible collector + query + UDP-agent surface plus a Grafana-Tempo-
compatible TraceQL search surface, all in-process inside cave-runtime.

---

## 1 · Fill-ratio (honest, measured)

```
mapped_count   = 21
partial_count  =  2
skipped_count  = 13
unmapped_count =  2
total          = 38
fill_ratio     = 0.9474   (mapped + partial + skipped) / total = 36/38
honest_ratio   = 0.6053   (mapped + partial) / total = 23/38
parity_ratio_source = "manifest"
```

### 2026-05-21 Charter v2 close-out delta

| Δ | field | before | after |
|---|---|---|---|
| ✓ | `fill_ratio`            | absent / `ratio = 0.0` placeholder | **0.9474** |
| ✓ | `honest_ratio`          | absent                              | **0.6053** |
| ✓ | `source_sha`            | absent                              | **`9866eba85aed1b0a66a77c8c6928a372edc5040f`** |
| ✓ | `parity_ratio_source`   | absent                              | **`"manifest"`** |
| ✓ | `last_audit`            | `2026-05-12`                        | **`2026-05-21`** |
| ✓ | subsystem inventory     | file-level only                     | **21 mapped / 2 partial / 13 skipped / 2 unmapped** |
| ✓ | `[[scope_cuts]]` groups | absent                              | **2 (ops storage + standalone CLIs)** |
| ✓ | `tests/parity_self_audit.rs` | absent                         | **9 assertions PASS** |

Subsystem-count formula matches the rest of the workspace
(`fill_ratio = (mapped + partial + skipped) / total`).

## 2 · Per-subsystem LOC table

```
src/lib.rs                             249   TraceConfig + TraceState + router + UDP agent supervisor
src/types.rs                           627   Span + Trace + TraceId + SpanKind + SpanStatus + TagValue + TraceSearchQuery
src/models.rs                          196   Internal storage records
src/storage.rs                         767   ColumnarTrace + TraceRecord + Bloom filter + retention GC
src/dependency.rs                      301   Service dependency graph extraction
src/collector.rs                       264   Span ingestion handler + tree build + critical path
src/query.rs                           475   QueryEngine over TraceStore + latency breakdown + bottleneck/anomaly
src/sampling.rs                        468   Constant + Probabilistic + RateLimiting + Tail + Adaptive samplers
src/spm.rs                             451   SPM RED metrics (request/error rate + percentiles)
src/propagation.rs                     508   W3C trace-context + B3 + Jaeger header parse/inject
src/traceql.rs                         843   TraceQL lexer + parser + AST + executor
src/multi_tenant.rs                    171   Tenant registry + X-Scope-OrgID extraction
src/correlation.rs                     234   Trace ↔ log + trace ↔ metric correlation
src/analyzer.rs                        434   Latency breakdown + bottleneck + anomaly + error propagation
src/comparison.rs                      176   PII anonymizer + trace diff
src/otlp.rs                            318   OTLP common helpers (base64 ID coercion)
src/error.rs                            52   TraceError + Result
src/ingestion/jaeger.rs                955   Jaeger Thrift HTTP + Jaeger UDP agent (compact 6831 + binary 6832)
src/ingestion/otlp.rs                  446   OTLP HTTP/JSON receiver
src/ingestion/zipkin.rs                258   Zipkin v2 JSON receiver
src/ingestion/opencensus.rs            398   OpenCensus JSON receiver
src/ingestion/mod.rs                    33   Receiver module index
src/routes/jaeger.rs                   456   Jaeger v1 query REST surface
src/routes/tempo.rs                    460   Tempo /api/search + TraceQL bridge
src/routes/ingest.rs                   190   Collector /v1/traces + /api/v2/spans + /oc/v1/traces routes
src/routes/mod.rs                       12   Route module index
                                     ─────
total                                9 742   27 files (all SPDX AGPL-3.0-or-later)
```

## 3 · Mapped subsystems (21)

### Core model (3)
1. **span-model** — `model/span.go` → `src/types.rs::Span`. Trace/span IDs, kind, status, tags, events, links, resource attributes, tenant scope, baggage, log labels.
2. **trace-model** — `model/trace.go` → `src/types.rs::Trace + ::TraceSearchQuery`. Trace aggregation, duration, root-detection, error-detection helpers.
3. **adjuster-anonymizer-pipeline** — `model/adjuster/adjuster.go` + `cmd/anonymizer/app/anonymizer` → `src/analyzer.rs` + `src/correlation.rs` + `src/comparison.rs`. Deduplicate / normalize + PII scrub (`user.*` hashing, `http.url` query-param redaction, `db.statement` value masking).

### Storage (2)
4. **in-memory-span-store** — `plugin/storage/memory/memory.go` → `src/storage.rs`. Bloom filter, columnar trace records, per-tenant cap, retention GC.
5. **dependency-store** — `storage/dependencystore/interface.go` → `src/dependency.rs`. Caller→callee edge extraction with call counts.

### Collector + query (4)
6. **collector-http-handler** — `cmd/collector/app/handler.go` → `src/collector.rs` + `src/routes/ingest.rs`. Span ingest + critical-path + tree build.
7. **query-service** — `cmd/query/app/querysvc/query_service.go` → `src/query.rs`. trace_by_id, traces_by_service, services, operations, dependencies, SPM lookups.
8. **jaeger-query-rest-api** — `cmd/query/app/http_handler.go` → `src/routes/jaeger.rs`. /api/traces, /api/services, /api/dependencies, /api/metrics/*.
9. **tempo-search-rest-api** — Tempo-compat overlay → `src/routes/tempo.rs`. /api/search + TraceQL bridge.

### Sampling (3)
10. **sampling-strategy-static** — `plugin/sampling/strategystore/static` → `src/sampling.rs`. Constant + Probabilistic + RateLimiting.
11. **sampling-strategy-adaptive** — `plugin/sampling/strategystore/adaptive/aggregator.go` → `src/sampling.rs::AdaptiveSampler`. TPS-target with window adjustment.
12. **sampling-tail-rules** — `internal/sampling/tail` → `src/sampling.rs::TailSampler`. 5 rule kinds (`AlwaysOnError` / `SlowTrace` / `TagMatch` / `ServiceMatch` / `Probabilistic`).

### Multi-tenancy (1)
13. **multi-tenancy** — `pkg/tenancy/tenancy.go` → `src/multi_tenant.rs`. X-Scope-OrgID extraction + per-tenant store isolation + auto-register policy.

### Ingestion (5)
14. **ingest-otlp-http** — `cmd/collector/app/handler.go` (OTLP receiver) → `src/ingestion/otlp.rs` + `src/otlp.rs`.
15. **ingest-jaeger-thrift-http** — `cmd/collector/app/http_handler.go` → `src/ingestion/jaeger.rs`.
16. **ingest-jaeger-udp-agent** — `cmd/agent/app/processors` → `src/lib.rs::run_jaeger_udp_agent`. Compact protocol port 6831 + binary protocol port 6832.
17. **ingest-zipkin-v2** — `cmd/collector/app/handler.go` (Zipkin v2) → `src/ingestion/zipkin.rs`.
18. **ingest-opencensus** — `cmd/collector/app/handler.go` (OpenCensus) → `src/ingestion/opencensus.rs`.

### Propagation + observability (3)
19. **propagation-w3c-b3-jaeger** — `internal/jaegerclientenv2otel/headers.go` → `src/propagation.rs`. parse_traceparent / parse_tracestate / extract_or_new / inject.
20. **traceql-engine** — Tempo-compat extension → `src/traceql.rs`. Lexer + parser + AST + executor.
21. **service-performance-monitoring** — `internal/jptrace/processor/spanmetrics/spanmetrics.go` → `src/spm.rs::SpmRegistry`. RED metrics + sliding window aggregation.

## 4 · Partial subsystems (2)

| Surface                    | Note                                                                                                     |
|----------------------------|----------------------------------------------------------------------------------------------------------|
| storage-pluggable-trait    | TraceStore surface in place; only memory backend implemented in-tree (operational backends in [[skipped]]). |
| clientcfg-remote-sampler   | reqwest client side present (used by AdaptiveSampler); server-side strategy distribution not exposed.   |

## 5 · Skipped subsystems (13 — out of MVP)

| Surface                    | Reason                                                                                         |
|----------------------------|------------------------------------------------------------------------------------------------|
| storage-cassandra          | Operational backend — needs cave-cassandra crate (not in workspace). Phase 3 cave-store route.|
| storage-elasticsearch      | Operational backend — depends on cave-search Manticore port. Phase 3 obs-LTS.                 |
| storage-badger             | Embedded LSM — in-memory + retention GC covers MVP. Phase 3 cave-store sled-shim.             |
| storage-kafka              | Async durable buffer — cave-streams provides workspace Kafka surface. Phase 3 bridge.         |
| storage-grpc-remote        | Out-of-process gRPC storage plugin host. cave-trace runs in-process; operational, not API.    |
| storage-scylladb           | Cassandra-compat — same rationale as storage-cassandra.                                       |
| storage-blackhole          | Null sink — TraceStore default + retention=0 covers the same need.                            |
| cmd-ingester-binary        | Kafka consumer binary — skipped jointly with storage-kafka.                                   |
| cmd-remote-storage-binary  | gRPC plugin host — skipped jointly with storage-grpc-remote.                                  |
| cmd-tracegen-binary        | Synthetic span load tester — qwen_drafted.rs + self-audit cover in-process equivalent.        |
| es-tooling                 | es-index-cleaner / es-rollover / esmapping-generator — operational ES tooling. Out of scope.  |
| sampling-leader-election   | Distributed sampling coord across collector replicas — Phase 3 (requires cave-ha).            |
| jaeger-v2-otel-binary      | `cmd/jaeger` v2 OTel-collector binary — separate code path; v1 API is the contract.           |

## 6 · Unmapped subsystems (2 — honest Phase-3 gaps)

| Surface                    | Note                                                                                              |
|----------------------------|---------------------------------------------------------------------------------------------------|
| agent-grpc-forwarder       | Jaeger agent's gRPC forwarder to collector. Today cave-trace's embedded UDP agent writes directly into the in-process TraceStore — structurally unnecessary in single-binary deployment; needed only if agent + collector get split across hosts. |
| storage-archival           | Secondary archival writer (warm → cold backend mirror). cave-trace exposes only one TraceStore today; archival composition lands with cave-store integration in Phase 3. |

## 7 · 4-track status

| Track          | Status     | Evidence                                                                                                        |
|----------------|------------|-----------------------------------------------------------------------------------------------------------------|
| Backend        | **GREEN**  | 100 lib unit tests + 9 parity_self_audit + 5 qwen_drafted propagation round-trip = 114 PASS. 9 742 LOC.         |
| Portal         | Phase 2    | `Jaeger UI` listed under `[portal_ui]` with status="partial" + P1 priority; tracked via cave-portal Phase 2.    |
| cavectl        | Phase 2    | `cavectl trace search` lands jointly with cave-metrics / cave-logs CLI in Phase 2.                              |
| Observability  | Phase 2    | alerts + dashboard + obs ring panel land with obs-stack-Phase-2 (cave-logs / cave-metrics already closed).      |

## 8 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                                  | Status |
|---|---------------------------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS + 100 lib unit tests + 5 qwen_drafted propagation tests PASS | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file (27/27)                                    | ✅      |
| 3 | `[upstream] source_sha` pinned to `9866eba85aed1b0a66a77c8c6928a372edc5040f` (v1.52.0) | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in `src/`                | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims                              | ✅      |
| 6 | Always-latest — Jaeger v1.52.0 (most recent stable v1 line; v2 OTel-binary deliberately deferred per scope_cuts) | ✅      |
| 7 | 4-track — Backend GREEN; Portal/cavectl/Obs honestly deferred Phase 2                 | ✅      |
| 8 | Honest measured `fill_ratio = 0.9474` (≥ 0.65 MVP floor)                              | ✅      |

## 9 · Reproducibility

```bash
cargo test -p cave-trace
cargo test -p cave-trace --test parity_self_audit
python3 scripts/build-parity-index.py
```
