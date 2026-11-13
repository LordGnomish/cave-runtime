# TDD coverage audit — cave-tracing

- **Crate:** `cave-tracing` (theme: observability)
- **Upstream:** https://github.com/jaegertracing/jaeger @ `v2.17.0`
- **Upstream test files scanned:** 462 (symbols ~1760)
- **Cave test fns (in-crate `#[test]`/`#[tokio::test]`):** 104 (100 lib + 4 smoke/self-audit)
- **Date:** 2026-05-30

## Framing

cave-tracing is **not** a port of Jaeger the collector/storage backend — it is the
sovereign OpenTelemetry-compatible *producer* SDK (TracerProvider → Tracer →
SpanBuilder → Span; W3C propagation; head + tail sampling; OTLP/Tempo export).
The companion crate `cave-trace` is the Jaeger-storage/query analogue.

Therefore the vast majority of Jaeger v2.17.0 upstream tests are **scope-cut** for
this crate: collector pipelines, span storage (Cassandra/ES/Badger/gRPC), query
handlers, adaptive-sampling probability calculators backed by a sampling store,
anonymizer CLI, CI scripts, model UDT marshalling, etc. The portable surface that
overlaps is: W3C trace-context propagation, trace/span ID handling, sampler
decision logic, and span-model serialization — all of which are **already
covered thoroughly** in cave-tracing.

This audit reports only the **genuine, narrow, portable-coverage gaps**: behaviors
the cave source already implements, that map to a Jaeger/OTel-portable behavioral
unit, and that currently have **no asserting test**.

## Classification table

| Upstream behavioral unit | Cave mapping | Classification |
|---|---|---|
| `TestParseTraceID` / traceparent strict parse + reject malformed/zero/ff | `propagation::parse_traceparent` (9 tests) | covered |
| W3C tracestate parse / 32-cap / upsert HEAD / drop-malformed | `propagation::parse_tracestate`, `TraceState::upsert` (5 tests) | covered |
| `TestRandomTraceID` / ID generation non-zero + low collision | `id::new_trace_id` / `new_span_id` (4 tests) | covered |
| `TestGetSamplerParams` / probabilistic + ratio sampler decision | `sampling::TraceIdRatioBased`, `AlwaysOn/Off`, `ParentBased` (12 tests) | covered |
| Tail-sampling policy enforcement (`TestTailSamplingProcessor_EnforcesPolicies`) | `sampling::TailSampler` + Error/Latency/AttrEqual policies (5 tests) | covered |
| `TestTraceIDJSONRoundTrip` / span model serde | `types` format/parse + Status serde (11 tests) | covered |
| Multi-tenant span scoping (`X-Scope-OrgID`) | `tenant::*` (6 tests) | covered |
| OTLP resource/scope grouping + typed attrs + status + events | `exporter::render_payload` (13 tests) | covered |
| BatchSpanProcessor flush/drop/shutdown/ticker | `batch::*` (11 tests) | covered |
| `Anonymizer_Hash` / `MapString` / `FilterStandardTags` (PII redaction) | no analogue in cave-tracing | missing-impl (scope: would live in cave-trace / cave-forensics) |
| Adaptive sampling `CalculateProbabilitiesAndQPS`, sampling-store probability loop | no analogue — head sampling only, no per-operation adaptive store | scope-cut (server-side, store-backed) |
| Collector/query/storage handlers (GetTrace, ArchiveTrace, FindTraceIDs, critical-path, topology) | cave-trace owns storage/query | scope-cut |
| CI scripts (safeNum/sanitizeMetricName/quota-manager) | infra/vendor plumbing | scope-cut |
| Span model UDT marshalling (Cassandra DB model) | storage-layer, not SDK | scope-cut |
| **OTLP `links` array serialization** | `exporter` `span_to_otlp` link path (emits traceId/spanId/attrs) | **portable-coverage (uncovered)** |
| **OTLP `parentSpanId` field for a child span** | `exporter` `span_to_otlp` parent-id path | **portable-coverage (uncovered)** |
| **`add_event` dropped when span not recording** (OTel non-recording guard) | `tracer::Span::add_event` recording guard | **portable-coverage (uncovered)** |
| **`set_status` dropped when span not recording** (OTel non-recording guard) | `tracer::Span::set_status` recording guard | **portable-coverage (uncovered)** |
| **TraceState `to_header` serialization order / round-trip via `get`** | `propagation::TraceState::to_header` | **portable-coverage (uncovered)** |
| **OTLP grouping splits two distinct tenants into separate resourceSpans** | `exporter::render_payload` tenant key in group | **portable-coverage (uncovered)** |

## Recommended TDD fills (portable-coverage first)

These are RED→GREEN candidates against existing public fns; each names the exact
cave fn the test exercises. All are behaviors already implemented in source but
without an asserting test.

1. **`exporter::OtlpHttpExporter::render_payload`** — assert the `links` array is
   serialized: build a `SpanData` with one `Link` (context + attribute), render,
   and assert `resourceSpans[0].scopeSpans[0].spans[0].links[0].traceId` /
   `.spanId` match the formatted hex and the link attribute is present. (No
   existing test populates `links`; the `span_to_otlp` link branch is unexercised.)

2. **`exporter::OtlpHttpExporter::render_payload`** — assert `parentSpanId` is the
   16-hex parent for a child span (set `parent_span_id = Some(0xabcd)`) and is an
   empty string for a root span (`parent_span_id = None`). (Currently every
   exporter test uses `parent_span_id: None` and never asserts the field.)

3. **`tracer::Span::add_event`** — assert that calling `add_event` on a span
   produced under an `AlwaysOff` sampler (non-recording) leaves `events` empty
   after `end()`. (The recording guard exists; only `set_attribute`'s guard is
   tested via `test_attribute_skipped_when_not_recording`.)

4. **`tracer::Span::set_status`** — assert that `set_status(Status::Error(..))` on
   a non-recording span (AlwaysOff) does not produce an Error-status span after
   `end()`. (Guard implemented, untested.)

5. **`propagation::TraceState::to_header`** — assert serialization is HEAD-of-list
   ordered (`upsert` puts newest first) and that `parse_tracestate(s.to_header())`
   round-trips with `get` returning the upserted values in order. (`to_header` is
   only exercised indirectly inside `inject`; no test asserts ordering/empty-state
   output directly.)

6. **`exporter::OtlpHttpExporter::render_payload`** — assert two spans with
   *different* `tenant_id` but identical scope/resource land in two separate
   `resourceSpans` groups (tenant is part of the grouping key). (Existing
   grouping test only varies `instrumentation_scope`, never the tenant key.)
