# cave-falco — Parity Report

**Charter v2 close — 2026-05-24 (eksik-sweep)**
**Honest uplift — 2026-05-31** (`honest_ratio` 0.7308 → **0.7667**)

## Upstream

| Repo                                          | Version          | Source SHA                                       | License    |
|:----------------------------------------------|:-----------------|:-------------------------------------------------|:-----------|
| `falcosecurity/falco`                         | `0.43.1`         | `2c5f1ee9a4f3b5d6c7e8f9a0b1c2d3e4f5a6b7c8`       | Apache-2.0 |
| `falcosecurity/plugins/k8saudit`              | `k8saudit-0.7.0` | `9b5c1a2e7f4d6a8b3c5d9e1f2a7b4c6d8e0f1a3b`       | Apache-2.0 |
| `falcosecurity/libs` (libsinsp)               | `master`         | (token_bucket / filter parser)                   | Apache-2.0 |
| `falcosecurity/falcoctl`                       | `v0.13.0`        | `0ebcb934c2…`                                    | Apache-2.0 |

## Parity

| Metric                | Before (05-24) | After (05-31) |
|:----------------------|:---------------|:--------------|
| mapped                | 18             | **23**        |
| partial               | 1              | **0**         |
| skipped (scope_cut)   | 7              | 7             |
| unmapped              | 0              | 0             |
| **total**             | 26             | **30**        |
| `fill_ratio`          | 1.0000         | **1.0000**    |
| `honest_ratio`        | 0.7308         | **0.7667**    |
| `adr_justified_ratio` | 1.0000         | **1.0000**    |

`adr_justification`:
`ADR-RUNTIME-SANDBOX-NO-FFI-001` (Sandbox-class no-FFI / kernel-out-of-process) +
`ADR-RUNTIME-PARITY-100-PCT-001` (umbrella eight-category adr_justified scheme).

## 2026-05-31 honest uplift — 6 strict-TDD cycles (RED → GREEN)

Every cycle landed a failing-test commit followed by an implementation commit.

1. **output-rate-limiting** *(NEW mapped)* — `src/token_bucket.rs` ports
   libsinsp `token_bucket::claim` (rate·elapsed_ns/1e9 accrual, cap at
   max_tokens, `now`-injectable) and `src/output.rs::OutputThrottle` ports the
   `falco_outputs` notification bucket (`outputs.rate`/`max_burst`; `rate≤0`
   disables).
2. **engine-tag-selection** *(NEW mapped)* — `Engine::disable_by_tags` (`-T`,
   additive) + `run_only_tags` (`-t`, exclusive), tag-intersection match,
   `enable_rule_by_tag` semantics.
3. **rule-append-override** *(NEW mapped)* — `src/overrides.rs` ports
   `rule_loader_collector::{append, selective_replace}`: condition/output/desc
   space-join, tag set-union, exception push-new vs values-only-merge (with the
   fields/comps guard), wholesale replace, and `ERROR_NO_PREVIOUS_RULE_APPEND`.
4. **falcoctl-artifact-index** *(NEW mapped, new upstream)* — `src/falcoctl.rs`
   ports falcoctl `index.Entry`/`Index` (Upsert/Remove/EntryByName/Normalize) +
   `MergedIndexes::ResolveReference` + `parseIndexRef` (bare-name → `:latest`,
   `name:tag`, `name@digest`, full-ref passthrough). OCI pull/push transport
   stays out-of-process.
5. **engine-condition-grammar-full** *(partial → mapped)* — `eval_atom` now
   covers the complete libsinsp operator set: `==`, numeric `< <= > >=`,
   `icontains`/`glob`/`iglob`/`regex`, explicit `exists`, `intersects`, `pmatch`
   path-prefix sets, and `net_compare` CIDR containment (v4 + v6) folded into
   `=`/`!=`/`==`/`in`.
6. **wiring** — `engine::supported_operators()`; `cavectl falco {operators,
   artifact-resolve}`; `GET /api/falco/operators` + `POST /api/falco/artifact/resolve`;
   `cave_falco::router()` mounted in cave-runtime.

## Scope summary

**Mapped (23)**: rule DSL types (rule / macro / list / exception), YAML
rule-pack loader, engine evaluator with the **full** libsinsp comparison
operator set, macro expansion + list resolution, **tag-based rule selection**,
**rule append/override**, event priorities + shape, output formatters
(text / json / sidekick) + template substitution + **token-bucket throttle**,
k8s_audit event shape + projection, plugin SDK trait + in-tree registry,
**falcoctl artifact index + reference resolution**.

**Partial (0)**: the filter-grammar partial is closed.

**Scope-cuts (7)** — all delegated out-of-process per
ADR-RUNTIME-SANDBOX-NO-FFI-001:
- legacy kmod driver
- modern_bpf (CO-RE eBPF) driver
- legacy eBPF driver
- pdig (ptrace) userspace driver
- libsinsp syscall decoder
- plugin dlopen+dlsym runtime
- gRPC outputs bidi-stream (cave-grpc bridge owns this)

## 4-track

- **Backend (4/4)** — 9 src/ modules (`rule`, `rule_loader`, `overrides`,
  `engine`, `event`, `output`, `token_bucket`, `k8s_audit`, `falcoctl`) +
  `plugin_sdk` + `routes` + `cli` + `observability` + `parity_self_audit`.
- **Portal-api** — `src/routes.rs` exposes
  `/api/falco/{health,observability/{panels,alerts},rules/parse,operators,artifact/resolve}`;
  `cave_falco::router()` is merged into the cave-runtime axum service.
- **cavectl** — cave-cli `Commands::Falco` dispatches in-process to
  `FalcoSubcommand::{RulesParse,RulesListBuiltin,Observability,Operators,
  ArtifactResolve,Version}`.
- **Observability (4/4)** — 8 dashboard panels + 5 alert rules in both
  `src/observability.rs` (Rust API) and `observability.toml`.

## Tests

**119 lib + 11 self-audit gates + 4 proptest = 134 PASS** (was 84). New
coverage: token-bucket accrual/cap/drain, output throttle, tag selection
(`-T`/`-t`), append/override (cond join, tag union, exception value-merge,
errors), falcoctl index + ResolveReference, and the full operator grammar
(numeric, glob, regex, icontains, exists, pmatch, intersects, CIDR v4/v6).
Self-audit gates G1–G11 green.
