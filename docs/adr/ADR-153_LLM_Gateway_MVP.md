# ADR-153 — cave-llm-gateway: MVP scope and provider matrix

- **Date:** 2026-05-21
- **Status:** Accepted
- **Owner:** Burak Tartan
- **Crate:** `cave-llm-gateway`
- **Branch:** `claude/cave-llm-gateway-2026-05-21`

## Context

`cave-llm-gateway` had been scaffolded as a placeholder behind a 0.0
parity ratio: an Axum router with OpenAI-compatible types, a `provider`
trait, and three concrete backends (OpenAI / Anthropic / generic
OpenAI-compatible local). The scaffold did not cover the *three* local
inference paths Burak's seat actually uses (Ollama native, llama.cpp
HTTP server, MLX-LM), did not resolve SaaS keys from the macOS
Keychain, had no capability-based routing, no retry policy, no
Prometheus exposition compatible with `cave-metrics`, and no
integration with `cave-hermes` or `cave-llm-tracker`.

The 2026-05-21 close of `cave-llm-tracker` (ADR-152) closed half of
the "local LLM control plane" — daily tracking and bench. The other
half — *runtime routing across local + SaaS backends* — is owned by
this crate.

## Decision

Land a 23-mapped-subsystem MVP for `cave-llm-gateway` under Charter v2:

1. **Six concrete providers** behind the existing `LlmProvider` trait:
   - **Local:** Ollama (native `/api/chat`), llama.cpp
     (`llama-server` OpenAI-compatible), MLX-LM (HTTP server).
   - **SaaS:** Anthropic, OpenAI, Mistral La Plateforme.
2. **Keychain-first SaaS key resolution** in `keychain.rs`:
   - `CAVE_LLM_<PROVIDER>_API_KEY` env-var → macOS
     `security find-generic-password -s cave-llm-gateway-<provider>`
     → `KeySource::NotFound`. No write path.
3. **Capability router** in `capability.rs`: deterministic scoring on
   quality + locality preference + cost taper + context headroom +
   hard requirement gates (tools, vision, json, min_context,
   max_cost). Seed catalogue of 10 rows covering all six providers.
4. **Cross-cutting reliability** modules:
   - `retry.rs` — exponential-backoff with 4xx-bypass.
   - `cache.rs` — already present, kept.
   - `cost.rs` — already present, kept.
   - `metrics.rs` — Prometheus exposition format, one
     `GatewayMetrics` per axum state, exposed at `/metrics`.
   - `health.rs` — fan-out `health_check()` aggregation.
5. **Integration bridges**:
   - `hermes_bridge.rs` — wire-stable mirror of
     `cave_hermes::ProviderKind` + `from_hermes_request` /
     `to_hermes_response` helpers + `HERMES_REQUIRED_PROVIDERS`. No
     reverse dependency on cave-hermes.
   - `bench_wire.rs` — stable 5-prompt order
     (`BENCH_PROMPT_IDS`) and `BenchSummary` aggregate consumed by
     `cave-llm-tracker`'s daily report.
6. **4-track Backend + cavectl**: `cavectl llm-gateway
   {routes,usage,limits,providers,health,capabilities,cost,cache,bench}`
   in the existing `Commands::LlmGateway` enum.

**Out of MVP** (formalised as `[[skipped]]` in
`parity.manifest.toml`, grouped under three `[[scope_cuts]]`):

- 7 cloud SaaS providers (Bedrock / Azure / Vertex / Cohere /
  Together / Fireworks / Replicate / HuggingFace / Groq / DeepSeek →
  *Phase 2*).
- 4 non-text endpoints (audio / images / embeddings-rerank / batch
  → sibling crates).
- 9 capabilities owned by other cave crates (admin UI →
  `cave-portal-web`; postgres spend store → `cave-rdbms`; redis
  cache → `cave-cache`; SSO/SCIM → `cave-auth`; notifications →
  `cave-oncall`; parallel-request limit → `cave-rate-limit-gateway`;
  multi-back-end log store → `cave-logs`; team management →
  `cave-portal-api`).

## Consequences

- The local-LLM seat now has end-to-end runtime support: tracker
  picks candidates daily (ADR-152), gateway routes them at request
  time. cave-hermes can take a single dependency (this crate) and
  fan out across all six backends via the bridge contract.
- `parity-index.json` workspace count of crates ≥ 0.95 goes up by
  one (cave-llm-gateway 0.0 → 1.0).
- The crate stays AGPL-3.0-or-later (cave-runtime default), and
  `parity.manifest.toml` pins LiteLLM `v1.85.1`
  (`f9c2a417a530a8369aeed96d259992040739c0f0`). When LiteLLM cuts a
  new release, `cave-upstream-watchd` will flag the manifest stale
  via the normal flow.
- Three orphan files (`models.rs`, `store.rs`, `budget.rs`) are
  honestly classified `partial` rather than silently mapped — they
  remain in the tree but are not wired into `lib.rs` until
  `GatewayState.budgets` plumbing lands in a follow-up. Recorded as
  the only `partial` rows in the manifest.
- A new ADR will need to amend this one when the Phase 2 cloud SaaS
  matrix lands.

## Alternatives considered

- **Use LiteLLM Python as a sidecar.** Rejected — pulls a heavy
  Python runtime into the data path, breaks Charter v2's
  Rust-monolith mandate, and the LiteLLM admin UI duplicates
  cave-portal-web.
- **Ship only OpenAI-compatible providers and let Ollama use its
  OpenAI shim.** Rejected — the native Ollama `/api/chat` returns
  `prompt_eval_count` / `eval_count` which we need for the cost
  ledger; the shim drops those.
- **One provider per crate.** Rejected — providers share a tiny
  trait surface (4 functions); seven crates would multiply
  Cargo.toml / parity-paperwork cost without any compile or
  runtime benefit.
- **Heavyweight `prometheus-client` dependency.** Rejected — the
  exposition format is plain text; we emit it directly from a
  parking_lot-protected BTreeMap and keep cave-llm-gateway free of
  the giant client crate.

## Charter v2 stamp

- 8 gates + 1 runtime-wiring gate checked in
  `crates/cave-llm-gateway/tests/parity_self_audit.rs`.
- `fill_ratio = 1.0` honest under the workspace formula
  `(mapped + partial + skipped) / total = (23 + 3 + 20) / 46`.
- `honest_ratio = 0.5` records that half the upstream surface is a
  documented scope-cut (cloud SaaS matrix + non-text endpoints +
  cave-* delegations), not silent gaps.
- 4-track Backend + cavectl ships; Portal + Obs surfaces are owned
  by sibling crates per the skipped list.

## Promotion to Phase 2

Phase 2 adds the seven scoped-out cloud SaaS providers and lifts
honest_ratio toward 1.0. The promotion criterion is "at least one
real downstream (cave-hermes or cave-llm-tracker) is dispatching
production traffic through this crate for a week without a
backend_fail spike". An amendment to this ADR will record the
matrix once it lands.
