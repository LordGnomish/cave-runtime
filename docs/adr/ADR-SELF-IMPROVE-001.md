# ADR-SELF-IMPROVE-001: cave-agent — runtime-resident self-improvement

**Status:** Accepted
**Date:** 2026-04-23
**Author:** Burak Tartan (raised the inconsistency), Sonnet (scribe)
**Scope:** Universal (charter-binding)

## Context

Charter ADR-CHARTER-001 declares Cave Runtime **"self-healing, self-improving"**. Today's repo confuses two distinct layers under one name:

1. **`cave-local-llm` (build-time agent, exists today):** a Rust daemon that runs as a LaunchAgent on the developer's Mac, calls a local Ollama instance with Qwen3-Coder-Next, and produces tier-1 Rust skeleton drafts against the Cave source tree. It is a **contributor-bot** that lives outside the product, touching the Cave source repository the same way a junior developer would.

2. **`cave-agent` (runtime agent, does not exist yet):** a Cave cluster module — a first-class citizen alongside `cave-scheduler`, `cave-etcd`, `cave-apiserver` — that runs **inside a running Cave cluster**, reads observability streams, reacts to drift/incidents, and proposes (and optionally applies) operational changes through Cave's own APIs. It is what the charter's "self-improving" promise actually requires.

Burak (2026-04-23) flagged this: "bizim amele benim Mac'te çalışıyor, ama self-healing / self-improving Cave'in runtime'ı içinde çalışmalıydı gibi geliyor". Correct instinct. The current build-time daemon is useful dev scaffolding; it is not the sovereign-Cloud-OS self-improvement capability.

## Decision

**We keep both, and we name them separately.**

### 1. Build-time path (`cave-local-llm`, today, unchanged mission)

Stays as-is. Lives on developer / CI machines. Its mission is **source-code-side** code generation:
- Tier-1 skeleton drafts against crate parity targets.
- Upstream function port suggestions.
- Test scaffolding.
- OSS community PR-like contributions once the repo is public.

It is explicitly **not** part of a running Cave cluster. It is part of the development process. Renaming target: `cave-dev-llm` or `cave-build-agent` after OSS.

### 2. Runtime path (`cave-agent`, new, primary self-improvement)

A new first-class crate, `crates/cave-agent/`, that deploys as a regular Cave workload (a Deployment in its own control-plane namespace). It speaks to:
- **Ollama sidecar on the same node** via `http://127.0.0.1:11434` (standard cave-cri container, GPU device passthrough where available). Model is part of the Cave artifact set, version-locked per cluster.
- **Cave observability stream** (`cave-trace`, `cave-metrics`, `cave-alerts`) — reads traces, metrics, alert firings.
- **Cave control-plane APIs** (`cave-apiserver`, `cave-scheduler`, `cave-gateway` admin) — proposes changes; never executes without safety rails.
- **cave-kernel primitives** (Raft for coordination across replicas, EventBus for cluster-wide notifications, SPIFFE identity for who-can-change-what).

### 3. Responsibilities (runtime agent only)

- **Drift correction:** scheduler placement oscillation, cache TTL misconfiguration, rate-limit misalignment.
- **Operational tuning:** auto-tune bin-packing thresholds, circuit-breaker limits, retry/backoff params based on SLO telemetry.
- **Incident auto-mitigation:** classify a firing alert against a playbook; if a deterministic remediation exists, apply it (with rollback); otherwise propose a patch for human review.
- **Safety-rail enforcement:** every change is canary-deployed via cave-scheduler, rollback on regression. No live change without a prior canary window.
- **Constrained change surface:** cave-agent may modify: config, feature flags, resource limits, scheduler weights, cache policies, SLO budgets. It **may not** touch: identity/RBAC, network policy, crypto material, user-facing API signatures, ADR-flagged invariants.

### 4. What self-improving is NOT

- Not "the LLM rewrites its own kernel at runtime".
- Not "autonomous production code deploys without a human gate" (at least not for OSS v1; that lives behind a cluster-level feature gate `CAVE_AGENT_AUTOAPPLY=true` that is off by default).
- Not "the LLM has keys to everything". cave-agent is a SPIFFE workload with a limited role binding (`system:cave-agent`) that grants only the namespaced change surface above.

## Rationale

**Why runtime, not only build-time?**
- Charter madde 5 ("self-improving") is user-facing. Users deploy Cave and Cave gets better. That cannot be a build-time claim.
- Operational knowledge (actual traffic shapes, real failure modes, workload-specific tuning) only exists inside a running cluster. Build-time LLMs cannot observe this.
- Incident response SLO: minutes-to-mitigate depends on an in-cluster agent that reacts, not on a push from a developer's laptop.

**Why keep build-time too?**
- Source code generation (new upstream ports, test coverage, refactoring sweeps) is different work from operational tuning; they need different context (full repo vs cluster telemetry) and different review paths (PR vs canary).
- Build-time agent pre-dates the repo being public; removing it loses a working tool.

**Why Ollama as sidecar?**
- Sovereign: no external API dependency; charter madde 3.
- Matches the "dış bağımlılık yok" principle.
- Per-node locality: no cross-node inference traffic; meets HA/DR latency-hiding (charter madde 10).

## Consequences

**Immediate (this sprint):**
- Rename existing crate internally: `cave-local-llm` → continue as-is, but docs clearly label it "build-time dev agent", not a runtime capability. Charter language moves all "self-improving" wording to cave-agent references.
- New crate: `crates/cave-agent/` scaffolded, empty, with parity.manifest.toml targeting zero upstream (it is a Cave-native invention).
- ADR-CHARTER-001 charter evidence section updated: "self-improving" links here.
- Portal: runtime progress page adds cave-agent as a tracked module at 0% (truth-in-advertising).

**Pre-OSS (28 days to 2026-05-21):**
- cave-agent minimal MVP: reads Prometheus scrape, reacts to one named alert (`ApiLatencyP99High`), proposes a canary-tuned scheduler weight change, rollback hook. Even MVP makes the charter claim truthful rather than aspirational.

**Post-OSS (roadmap):**
- Expand safety rails: ADR-constraint engine enforces policy at proposal time.
- Multi-cluster: cross-cluster learning via federated EventBus, propagate proven tunings.
- Feedback loop: agent records every change + measured delta; future decisions Bayesian-weighted on past outcomes.

## Alternatives considered

1. **Keep only build-time agent, call charter's "self-improving" aspirational.** Rejected: dishonest, and charter madde 5 is user-facing promise.
2. **Embed the LLM directly in cave-apiserver.** Rejected: violates single-responsibility; apiserver's critical path must not wait on inference; coordination with scheduler becomes opaque.
3. **Run cave-agent off-cluster (SaaS backplane).** Rejected: violates sovereign charter (madde 3); introduces external dependency.

## Migration notes

- Move `cave-local-llm/README.md` to explicitly title itself "Build-Time Development Agent".
- Rename future-facing charter references from "the local LLM daemon" → "cave-agent (runtime) + cave-local-llm (build-time)".
- Portal landing copy: add a short paragraph explaining the two-layer design so visitors do not confuse the dev-laptop daemon with runtime capability.

## References

- ADR-CHARTER-001 — Cave Runtime charter (self-improving as user-facing promise)
- ADR-GOLDEN-001 — upstream line-by-line parity
- ADR-GOLDEN-002 — cave-kernel shared primitive requirement (used by cave-agent for coordination)
- ADR-GOLDEN-003 — no-backcompat + PQC-ready (cave-agent respects this in any change it proposes)
- ADR-LOCAL-LLM-001 — tiered self-improving workflow (predecessor, superseded in scope by this ADR)
- 2026-04-23 user remark: "benim beklentim cave self healing self improving runtime benim Mac'te çalışmalı ama LLM runtime'da çalışmalıydı gibi geliyor bana"
