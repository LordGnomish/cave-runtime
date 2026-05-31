# cave-llm-gateway — Charter v2 PARITY_REPORT

**Upstream**: [BerriAI/litellm](https://github.com/BerriAI/litellm) @ `v1.85.1` (commit `f9c2a417a530a8369aeed96d259992040739c0f0`)
**Last audit**: 2026-05-31  •  **honest_ratio**: 0.5532 (26 mapped / 47)
**ADR**: [ADR-153 — cave-llm-gateway MVP scope and provider matrix](../../docs/adr/0153-cave-llm-gateway-mvp.md)

## 8-gate summary

| Gate | Subject | Status |
| ---- | ------- | ------ |
| 1 | Upstream version pinned (`v1.85.1`) | PASS |
| 2 | Source SHA present (`f9c2a417…`, 40 hex chars) | PASS |
| 3 | fill_ratio measured (`1.0`, floor 0.95) | PASS |
| 4 | parity_ratio_source = `manifest` | PASS |
| 5 | last_audit is a 2026 ISO date (`2026-05-31`) | PASS |
| 6 | mapped + partial + skipped + unmapped == total (26 + 2 + 19 + 0 == 47), unmapped == 0 | PASS |
| 7 | No `unimplemented!()` / `todo!()` macros in `src/` | PASS |
| 8 | AGPL-3.0-or-later SPDX header on every `.rs` file (≥18 files) | PASS |
| 9 (runtime) | All six MVP providers dispatch from `ProviderRegistry::from_config` and appear in the capability seed catalogue + hermes bridge classifier | PASS |

## Coverage

- **Mapped** 26 — provider trait + 9 backends (incl. Groq + DeepSeek, OpenAI-compat SaaS) + keychain + capability router + cost + cache + retry + metrics + health + rate-limit + alias + api-keys + guardrails + logging + router-strategies + routes + streaming + cavectl `llm-gateway` subcommand + **live BudgetManager** (litellm/budget_manager.py, enforced in `complete()`) + **`/v1/embeddings`** (OpenAI-compat).
- **Partial** 2 — `models.rs`, `store.rs` are scaffold-only (not in lib.rs module tree) AND duplicate capabilities already live behind `GatewayRouter` (cost/cache/logging/rate-limit); kept honest as partial rather than double-counted. (`budget.rs` was promoted to mapped this round.)
- **Skipped** 19 — see `[[skipped]]` in `parity.manifest.toml`. Three scope_cut groups:
  - `cloud-saas-matrix` (6): Bedrock / Azure-OpenAI / Vertex-Google / Cohere / Together-Fireworks / Replicate-HuggingFace → Phase 2 (bespoke wire). Groq + DeepSeek shipped.
  - `non-text-endpoints` (4): audio / images / rerank / batch → sibling cave-llm-gateway-{audio,images,rerank,batch} crates. (embeddings shipped.)
  - `owned-by-other-cave-crates` (9): admin-ui → `cave-portal-web`; spend-store-postgres → `cave-rdbms`; cache-redis-back-end → `cave-cache`; auth-sso-oidc + auth-scim-provisioning → `cave-auth`; notifications-slack-discord → `cave-oncall`; rate-limit-parallel-request → `cave-rate-limit-gateway`; log-store-multi-backend → `cave-logs`; team-management → `cave-portal-api`.
- **Unmapped** 0 — every upstream subsystem is classified.

## Provider matrix (MVP)

| Provider | Locality | Transport | Health endpoint |
| -------- | -------- | --------- | --------------- |
| Ollama | local | native `/api/chat` | `/api/tags` |
| llama.cpp | local | OpenAI-compatible `/v1/chat/completions` | `/health` |
| MLX-LM | local | OpenAI-compatible `/v1/chat/completions` | `/v1/models` |
| Anthropic | SaaS | `/v1/messages` (system extracted) | always-OK probe |
| OpenAI | SaaS | `/v1/chat/completions` | `/v1/models` |
| Mistral | SaaS | `/v1/chat/completions` (OpenAI-compat) | `/v1/models` |

SaaS keys resolve in order: `CAVE_LLM_<PROVIDER>_API_KEY` env-var → macOS Keychain (`security find-generic-password -s cave-llm-gateway-<provider>`) → `KeySource::NotFound`. No key is ever written by the crate.

## Capability router

`CapabilityRouter::rank(req)` returns a deterministic, score-ordered list of `ModelCapability` rows. Scoring composes quality (0–100), locality bonus (+20), cost taper (+0–15), and context headroom (+0–10). Hard exclusions (missing tools / vision / json / context / cost-cap) drop a model with `f32::NEG_INFINITY`.

Seed catalogue (10 rows) covers all six MVP providers; `cave-llm-tracker`'s daily report can call `register()` to override quality scores.

## Integrations

- **cave-hermes**: `hermes_bridge` exposes `HermesProviderKind` (mirror of `cave_hermes::prompt::ProviderKind`), `from_hermes_request` / `to_hermes_response`, `classify_provider`, and `HERMES_REQUIRED_PROVIDERS`. No reverse dependency on `cave-hermes`.
- **cave-llm-tracker**: `bench_wire` declares the stable 5-prompt order (`BENCH_PROMPT_IDS`) and the `BenchSummary` aggregate. cave-llm-tracker imports the constant to keep daily reports diff-stable.
- **cave-metrics**: `metrics::render_prometheus()` emits exposition-format text; the `/metrics` route is mounted by `routes::create_router`.
- **cavectl**: `cavectl llm-gateway {routes,usage,limits,providers,health,capabilities,cost,cache,bench}` lands as a `Commands::LlmGateway` extension in `cave-cli/src/main.rs`.

## Test surface

- Library tests: 92 PASS (provider/registry, ollama/llama.cpp/mlx/mistral health/models, keychain resolution, capability router scoring, retry policy + with_retry, metrics counters + exposition, health probe, hermes bridge classification + mapping, bench wire summary).
- Parity self-audit: 9 PASS (gates 1–9, see table above).

## Open follow-ups

- Wire `models.rs` + `store.rs` + `budget.rs` into the module tree (current partial; needs `GatewayState.budgets` field + state lock plumbing).
- Move SaaS spend ledger to `cave-rdbms` per `[[skipped]]` `spend-store-postgres`.
- Phase 2 cloud SaaS matrix (Bedrock / Azure / Vertex / Cohere / Together / Replicate / Groq).
