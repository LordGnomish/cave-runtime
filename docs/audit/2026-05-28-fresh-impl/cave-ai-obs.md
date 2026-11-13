# Coverage Audit — cave-ai-obs vs Langfuse

- **Crate:** cave-ai-obs (`crates/observability/cave-ai-obs`)
- **Upstream:** https://github.com/langfuse/langfuse
- **Tag / SHA:** `v3.75.1` / `d37e676ea61abc623a38ffe9c76d73de7aac14d0`
- **Upstream license:** MIT (line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29

## Summary

cave-ai-obs is a near-empty placeholder. Compiled module tree (`lib.rs`) is only
`engine` + `models` + `routes`. The crate contains:

- `models.rs` — two plain structs (`LlmTrace`, `LlmStats`), no behavior.
- `engine.rs` — pure in-memory aggregation over a `&[LlmTrace]` slice: `calculate_stats`,
  `total_tokens`, `filter_by_model`, `cost_per_thousand_tokens`. Real arithmetic, but operates
  on a caller-supplied vec; there is no ingestion, no persistence, no API surface for it.
- `routes.rs` — one static `GET /api/ai-obs/health` returning a hardcoded JSON literal.
- `store.rs` — **orphaned / dead code**: not declared in `lib.rs` (no `pub mod store;`), and
  references types (`LlmRequest`, `AiObsStats`, `CostStats`, `LatencyStats`, `TokenStats`,
  `RequestStatus`, `Provider`) that are **defined nowhere in the crate**. It does not compile
  as part of the build and contributes zero real coverage.

Langfuse is a full LLM-observability platform (trace/observation/score ingestion pipeline,
ClickHouse-backed analytics, prompt management, evals/LLM-as-judge, datasets/experiments,
public API + SDK ingestion, dashboards, RBAC/multi-tenancy, batch export, media handling).
Essentially none of this exists in the crate beyond toy stat math.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `packages/shared/src/server/ingestion/processEventBatch.ts` | Ingestion pipeline: validate + inflate trace/observation/score event batches | — | MISSING | No ingestion path at all; `engine` consumes a pre-built vec. |
| `packages/shared/src/server/ingestion/types.ts` | Ingestion event schema (trace-create, observation, score, span, generation) | `models.rs` | MISSING | Only flat `LlmTrace`; no event-type discrimination, no span/generation/observation. |
| `packages/shared/src/domain/traces.ts` | Trace domain model (sessions, nested observations, metadata) | `models.rs::LlmTrace` | PARTIAL | A trace struct exists but is a flat per-request record; no nesting/sessions/observations. |
| `packages/shared/src/domain/observations.ts` | Observation model (span/event/generation hierarchy, parent links, levels) | — | MISSING | No observation concept. |
| `packages/shared/src/domain/scores.ts` | Score model (numeric/categorical/boolean, data types, config) | — | MISSING | No scoring. |
| `worker/src/features/tokenisation/` | Token counting / model-based tokenisation | `engine.rs::total_tokens` | PARTIAL | Just adds two caller-supplied numbers; no tokeniser, no model-aware counting. |
| `web/src/features/models/` + cost calc | Model price table, cost computation from token usage | `engine.rs::cost_per_thousand_tokens` | PARTIAL | Cost is a caller-supplied field on the trace; no price table, no model→cost derivation. |
| `web/src/features/dashboard/` + `worker` analytics | Aggregations: counts, latency percentiles, cost/token breakdowns, error rates | `engine.rs::calculate_stats` | PARTIAL | Computes per-model totals/avg-latency/error-rate over a vec, but no p50/p95/p99, no time bucketing, no by-provider/by-user. (Percentile + breakdown logic lives only in orphaned dead `store.rs`.) |
| `packages/shared/src/server/clickhouse/` + `repositories/` | Persistent analytical store (ClickHouse) for traces/observations/scores | (`store.rs` orphaned) | MISSING | Crate has no persistence; the only store impl is dead code referencing undefined types. |
| `web/src/features/public-api/` | Public REST API (ingestion + query endpoints, pagination) | `routes.rs` | MISSING | Only a static health endpoint; no trace/observation/score/query routes. |
| `web/src/features/prompts/` | Prompt management (versioning, labels, deployment, rollback) | — | MISSING | Absent. |
| `web/src/features/evals/` + `worker/src/features/evaluation/` | Evals / LLM-as-a-judge scoring jobs | — | MISSING | Absent. |
| `web/src/features/datasets/` | Datasets + dataset items | — | MISSING | Absent. |
| `web/src/features/experiments/` | Experiments (run dataset through prompt/model, compare) | — | MISSING | Absent. |
| `web/src/features/scores/` | Manual + programmatic scoring UI/API | — | MISSING | Absent. |
| `web/src/features/annotation-queues/` | Human annotation queues | — | MISSING | Absent. |
| `packages/shared/src/server/llm/fetchLLMCompletion.ts` | LLM provider gateway (multi-provider completion fetch) | — | MISSING | Absent (lives in cave-llm-gateway instead). |
| `web/src/features/rbac/` + `organizations/` + `projects/` | Multi-tenancy, orgs/projects, RBAC roles | — | MISSING | No tenancy or auth model. |
| `web/src/features/auth/` + `auth-credentials/` + `llm-api-key/` | Auth (API keys, sessions, SSO), secret encryption | — | MISSING | Absent. |
| `web/src/features/batch-exports/` + `worker/src/features/batchExport/` | Batch export of traces/observations to CSV/JSON/S3 | — | MISSING | Absent. |
| `web/src/features/media/` | Multi-modal media (images/audio) storage + refs | — | MISSING | Absent. |
| `web/src/features/filters/` + `server/filterToPrisma.ts` | Rich filter DSL → query translation | `store.rs::filter_requests` (orphaned) | MISSING | Only a dead-code substring filter exists; not compiled, undefined types. |
| `worker/src/queues/` (ingestion, eval, export, retention) | Async job queues / background processing (BullMQ/Redis) | — | MISSING | No queueing/worker layer. |
| `web/src/features/comments/` + `audit-logs/` | Comments + audit logging on entities | — | MISSING | Absent. |
| `web/src/features/otel/` + `instrumentation.ts` | OpenTelemetry ingestion / self-instrumentation | — | MISSING | Absent. |
| `web/src/features/widgets/` + `query/` | Custom dashboard widgets + query builder | — | MISSING | Absent. |

### Status tally

- COVERED: 0
- PARTIAL: 5 (trace model, token sum, cost-per-1k, stats aggregation, plus dashboard-aggregation gap noted as partial)
- MISSING: ~20

Module count (upstream functional modules enumerated): 25.

## Actionable gaps for strict-TDD

Ordered lowest-effort / highest-value first. Each names the upstream reference and a concrete
failing test.

1. **Wire `store.rs` into the module tree and define its types (or delete it).**
   The crate ships an entire `AiObsStore` (append, get_by_id, filter, percentile latency,
   cost-by-model/provider/user) that is never compiled because `lib.rs` lacks `pub mod store;`
   and `LlmRequest`/`AiObsStats`/`Provider`/`RequestStatus`/`CostStats`/`LatencyStats`/`TokenStats`
   are undefined. This is the single biggest cheap win — most "analytics" already exists as dead code.
   - Upstream ref: `web/src/features/dashboard/`, `packages/shared/src/server/repositories/`
   - Test: `test_store_module_is_compiled_and_percentiles_work` — construct `AiObsStore::new()`,
     append 100 `LlmRequest` with known latencies, assert `compute_latency_stats().p95_ms`
     equals the 95th-percentile value. (Currently fails to even compile.)

2. **Ingestion endpoint that accepts a trace event and persists it.**
   - Upstream ref: `packages/shared/src/server/ingestion/processEventBatch.ts`,
     `web/src/features/public-api/server/`
   - Test: `test_post_ingestion_stores_trace` — `POST /api/ai-obs/ingestion` with a single
     trace-create event returns 201/207 and a subsequent `get_by_id` returns the stored trace.

3. **Latency percentiles in the live `calculate_stats` path.**
   `engine.rs::calculate_stats` returns avg latency and error rate but no p50/p95/p99,
   unlike upstream dashboard metrics.
   - Upstream ref: `web/src/features/dashboard/` latency charts
   - Test: `test_calculate_stats_reports_p95` — feed 20 traces with latencies 1..20,
     assert returned stats expose `p95_ms == 19`.

4. **Model-aware cost derivation from a price table.**
   `cost_per_thousand_tokens` only divides a caller-supplied `cost_usd`; there is no
   token→USD pricing.
   - Upstream ref: `web/src/features/models/` (model price definitions)
   - Test: `test_cost_derived_from_model_price_table` — given a registered price for
     "gpt-4o" (input/output per-1k) and a trace with 1000 prompt + 500 completion tokens,
     assert the computed `cost_usd` matches the price-table math.

5. **Filter DSL over stored traces (provider/model/user/status/time-range).**
   Only the orphaned `store.rs::filter_requests` substring filter exists.
   - Upstream ref: `web/src/features/filters/`, `packages/shared/src/server/filterToPrisma.ts`
   - Test: `test_filter_traces_by_model_and_time_range` — append traces across two models
     and timestamps, query model="gpt-4" within a `from`/`to` window, assert only matching
     traces (newest-first, limited) are returned.

6. **Scores: attach numeric/categorical scores to a trace.**
   - Upstream ref: `packages/shared/src/domain/scores.ts`, `web/src/features/scores/`
   - Test: `test_attach_numeric_score_to_trace` — create a trace, attach a score
     `{name:"quality", value:0.9, dataType:NUMERIC}`, assert it is retrievable and included
     in per-trace score aggregation.
