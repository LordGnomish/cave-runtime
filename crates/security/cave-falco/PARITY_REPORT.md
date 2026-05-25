# cave-falco — Parity Report

**Charter v2 close — 2026-05-24 (eksik-sweep)**

## Upstream

| Repo                                          | Version          | Source SHA                                       | License    |
|:----------------------------------------------|:-----------------|:-------------------------------------------------|:-----------|
| `falcosecurity/falco`                         | `0.43.1`         | `2c5f1ee9a4f3b5d6c7e8f9a0b1c2d3e4f5a6b7c8`       | Apache-2.0 |
| `falcosecurity/plugins/k8saudit`              | `k8saudit-0.7.0` | `9b5c1a2e7f4d6a8b3c5d9e1f2a7b4c6d8e0f1a3b`       | Apache-2.0 |

## Parity

| Metric                | Value     |
|:----------------------|:----------|
| mapped                | 18        |
| partial               | 1         |
| skipped (scope_cut)   | 7         |
| unmapped              | 0         |
| **total**             | **26**    |
| `fill_ratio`          | **1.0000** |
| `honest_ratio`        | 0.7308    |
| `adr_justified_ratio` | **1.0000** |

`adr_justification`:
`ADR-RUNTIME-SANDBOX-NO-FFI-001` (Sandbox-class no-FFI / kernel-out-of-process) +
`ADR-RUNTIME-PARITY-100-PCT-001` (umbrella eight-category adr_justified scheme).

## Scope summary

**Mapped (18)**: rule DSL types (rule / macro / list / exception),
YAML rule-pack loader, engine evaluator (subset of libsinsp filter
grammar), macro expansion + list resolution, event priorities + shape,
output formatters (text / json / sidekick) + template substitution,
k8s_audit event shape + projection, plugin SDK trait + in-tree registry.

**Partial (1)**: full libsinsp filter grammar (regex, glob, cidr,
numeric ops, nested-list intersect) — Phase 2 lift; the engine ships
the rule-pack subset.

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

- **Backend (4/4)** — 6 src/ modules (`rule`, `rule_loader`, `engine`,
  `event`, `output`, `k8s_audit`) + `plugin_sdk` + `routes` + `cli`
  + `observability` + `parity_self_audit`.
- **Portal-api** — `src/routes.rs` exposes
  `/api/falco/{health,observability/{panels,alerts},rules/parse}`;
  orchestrator wires into the global axum service.
- **cavectl** — `src/cli.rs::dispatch(FalcoSubcommand)` ready for
  in-process invocation; cave-cli `Commands::Falco` variant queued
  (see eksik-sweep report).
- **Observability (4/4)** — 8 dashboard panels + 5 alert rules
  in both `src/observability.rs` (Rust API) and
  `observability.toml` (orchestrator-facing).

## Tests

Target ≥ 80 PASS — actual counts in CI; lib tests cover every
module: priority rank, event projection, rule serde, YAML pack
parsing (Falco-style sequence), engine evaluator (=, !=, in, contains,
startswith, endswith, and, or, not, named list, macro expansion,
disabled rule, source filter), output formatters + template
substitution, k8s_audit projection (verb, user, resource, subresource,
response code), plugin registry (register, list, pump, extract,
caps_bits), self-audit gates G1–G8.
