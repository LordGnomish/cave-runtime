# TDD coverage audit — cave-ai-obs vs Langfuse @ v3.75.1

- **Cave crate:** `crates/observability/cave-ai-obs` (theme: observability)
- **Upstream:** https://github.com/langfuse/langfuse @ v3.75.1
- **Upstream test symbols:** 218 (21 test files — Playwright e2e, worker BullMQ jobs, ClickHouse/S3/Redis/Postgres services, ingestion pipeline)
- **Cave test fns:** 50 (`tests/trace_ingestion.rs` 15, `tests/analytics.rs` 11, `tests/self_audit.rs` 11, `tests/proptest_smoke.rs` 4, plus in-module unit tests in `analytics.rs`/`prompt.rs`/`engine.rs`/`trace_store.rs`)
- **Gap count (real, portable):** 4
- **Portable-coverage gaps:** 4

## Scope statement (honest)

cave-ai-obs is a **small in-memory backend subset** of Langfuse, a large multi-service web application. It implements: an in-memory `TraceStore` (traces/spans/generations/scores/prompt-templates), a cost/latency **analytics** engine, a `{{var}}` **prompt template** renderer, a legacy `LlmTrace` stats **engine**, and an axum **HTTP API**. It does **not** implement Langfuse's ClickHouse writer, S3/blob storage, Redis queues, Postgres/Prisma, BullMQ worker jobs (batch export, data retention, project/trace/score deletion, blob-storage integration), the eval/experiment services, the Next.js web UI, or the OTel/SDK ingestion pipeline. The overwhelming majority of the 218 upstream test symbols exercise those subsystems and are therefore **scope-cut**, not coverage gaps.

## Classification of upstream test groups

| Upstream test file / group | Symbols | Classification | Rationale |
|---|---|---|---|
| `web/src/__e2e__/auth.spec.ts`, `create-project.spec.ts` | 11 | scope-cut | Playwright web-UI / auth / project CRUD — no web app in cave |
| `worker/.../batchAction.test.ts`, `batchExport.test.ts` | 23 | scope-cut | BullMQ worker jobs + ClickHouse/S3 export — not implemented |
| `worker/.../blobStorageIntegrationProcessing`, `dataRetentionProcessing`, `projectDeletionProcessing`, `scoreDeletion`, `traceDeletion` | 28 | scope-cut | S3/cloud-storage retention & cascade-delete jobs — infra, not implemented |
| `worker/.../evalService*.test.ts`, `experimentsService.test.ts` | 51 | scope-cut | Eval/experiment job engine (handlebars eval, LLM-as-judge, dataset jobs) — separate subsystem, not in cave |
| `worker/.../modelMatch.test.ts` | 9 | scope-cut | Redis/Postgres model-price lookup — cave has no model-price DB |
| `worker/.../redisConsumer.test.ts`, `storageservice.test.ts`, `ClickhouseWriter.unit.test.ts` | 32 | scope-cut | Redis queue / S3 storage / ClickHouse batch writer infra |
| `worker/.../IngestionService.integration.test.ts` (token-cost merge, upsert ordering, metadata merge) | 17 | scope-cut (mostly) | Full ClickHouse ingestion merge semantics; cave's `upsert_*` is last-write-wins in-memory, no token-cost merge engine |
| `worker/.../calculateTokenCost.unit.test.ts` | 19 | scope-cut | Model-price-table token cost calc; cave stores `cost_usd` directly on the Generation (no price lookup) |
| `worker/.../IngestionService.unit.test.ts` (sort events ascending by timestamp) | 2 | partial / portable-adjacent | cave sorts traces newest-first in `list_traces`; sort behavior **is** tested. Date→DateTime conversion is infra → scope-cut |
| `worker/.../utils.unit.test.ts` (overwriteObject / metadata-merge) | 8 | scope-cut | Object-merge helper for ClickHouse upsert; cave upsert is whole-record replace |
| `worker/.../inMemoryFilterService.test.ts` (evaluateFilter: string/number/datetime/bool/options/AND-logic) | 17 | **portable-coverage** (subset) | cave **does** implement in-memory trace filtering (`list_traces` by user_id/session_id/tag). The combined/AND filter path and tag-membership filter are implemented but under-tested |

## Real gaps (cave IMPLEMENTS, no/partial test)

These are behaviors present in cave source, mirrored by an upstream test concept, with missing or incomplete cave coverage.

1. **`TraceStore::list_traces` — combined multi-filter (AND) + tag-membership filter.** `src/trace_store.rs:53` applies `user_id` AND `session_id` AND `tag` filters together. Existing tests (`test_list_by_session`, `test_list_traces_filtered_by_user`) only exercise one filter at a time; the AND-composition branch and the `tags.iter().any(...)` tag-membership branch are untested. Mirrors upstream `inMemoryFilterService` "multiple filters with AND logic" + stringOptions/arrayOptions tests.

2. **`TraceStore::get_active_prompt` — active-flag selection vs. plain max-version.** `src/trace_store.rs:231` filters `is_active` THEN takes `max_by_key(version)`. The only test (`test_prompt_template_versioning`) sets the highest version (3) as the active one, so it cannot distinguish "select the active version" from "select the max version". A test where a **lower** version is the active one (and a higher version is inactive) is needed to verify the `is_active` filter actually drives selection. Mirrors upstream prompt-version resolution semantics.

3. **`analytics::compute_cost_window` — `by_model` breakdown map.** `src/analytics.rs:151` builds a per-model cost map and a `generation_count`, but the integration test `test_cost_window_sum` only asserts `total_usd`. The `by_model` aggregation (multiple models in one window) is uncovered. Mirrors upstream batchExport "group costs by model" assertions.

4. **`engine::cost_per_thousand_tokens` — non-zero (Some) path.** `src/engine.rs:47` returns `Some(cost/total*1000)` for non-zero tokens, but only the zero-token `None` path is tested (`test_cost_per_thousand_tokens_zero_tokens`). The arithmetic (the actual value returned) is unverified. Mirrors upstream calculateTokenCost per-1k-token math.

## Recommended TDD fills (portable-coverage first)

| # | Cave public fn | Test to add (RED→GREEN) |
|---|---|---|
| 1 | `cave_ai_obs::trace_store::TraceStore::list_traces` | Insert traces with mixed user_id/session_id/tags; assert that passing `user_id=Some("alice")` AND `session_id=Some("s1")` together returns only traces matching **both**, and that `tag=Some("prod")` matches a trace whose `tags` vector contains "prod" but excludes one that does not. |
| 2 | `cave_ai_obs::trace_store::TraceStore::get_active_prompt` | Upsert versions 1 (active), 2 (inactive), 3 (inactive) of one prompt name; assert `get_active_prompt` returns **version 1** (proving `is_active` drives selection, not max version). Add inverse case where v2 is the sole active. |
| 3 | `cave_ai_obs::analytics::compute_cost_window` | Upsert generations for two models within the window with known costs; assert `by_model["gpt-4"]` and `by_model["claude"]` sum correctly and `generation_count` matches, in addition to `total_usd`. |
| 4 | `cave_ai_obs::engine::cost_per_thousand_tokens` | Build an `LlmTrace` with 100 prompt + 50 completion tokens and `cost_usd=0.0015`; assert `cost_per_thousand_tokens` returns `Some(0.01)` (0.0015 / 150 * 1000). |
