# ADR-048: Data Persistence Portal Wraps

**Status:** Accepted (2026-04-28)
**Scope:** Cave Runtime (UI Consolidation extension; data persistence layer specialization)
**Category:** Runtime / Portal UX
**Decided:** 2026-04-28 (Burak Tartan)

> Slot 048 was previously occupied by `MinIO Selection Rationale`, which has been absorbed into ADR-050 (Object Storage). The slot is reused here for a semantically related decision: how upstream data-persistence UIs are consolidated into cave-portal.

## Context

`ADR-RUNTIME-UI-CONSOLIDATION-001` mandates that all tenant- and operator-facing UI lives inside **cave-portal** — no separate per-crate web admin UI is allowed. The data-persistence layer is the largest concrete instance of that mandate: ten crates (Postgres / MongoDB / Valkey / OpenSearch / MinIO / Kafka+Pulsar / Iceberg / DataFusion / OpenBao / Qdrant) each ship with a mature upstream UI (pgAdmin, Compass, Redis Insight, etc.). Without an explicit catalog, contributors will replicate those UIs inside the individual data crates and break the consolidation invariant.

This ADR enumerates each data crate's required cave-portal wrap and pins the work to the cave-portal track sprint.

## Decision

Every data-persistence crate **MUST** ship its admin/exploration surface as native React panels inside cave-portal. The upstream UI (pgAdmin, Compass, etc.) is **not** deployed alongside the crate in any profile — sovereign or otherwise.

| Crate | Upstream UI Reference | Native Wrap (cave-portal) |
|---|---|---|
| `cave-pg` | pgAdmin / DBeaver | query browser, schema view, EXPLAIN viewer, slow query log, connection mgmt |
| `cave-docdb` | MongoDB Compass | document browser, aggregation pipeline builder, index manager |
| `cave-cache` | Redis Insight | key browser (string/list/set/zset), pub/sub monitor, slowlog, memory analyzer |
| `cave-search` | OpenSearch Dashboards | Discover (query), Visualize (charts), Dashboard (panels), index manager |
| `cave-blob` | MinIO Console | bucket browser, object preview, lifecycle policy editor, IAM viewer |
| `cave-streams` | AKHQ + Pulsar Manager | topic browser, message viewer, consumer group monitor, schema registry |
| `cave-iceberg` | Catalog Browser | table catalog, schema/partition viewer, snapshot history, compaction monitor |
| `cave-datafusion` | Query Builder | SQL editor with autocomplete, EXPLAIN viewer, dataframe preview |
| `cave-vault` | Vault UI | secret browser, policy editor, auth method config, transit key mgmt |
| `cave-vector` (Qdrant) | Qdrant UI | collection browser, vector search playground, payload filter builder |

### Rationale

ADR-RUNTIME-UI-CONSOLIDATION-001 already prohibits separate UIs at the platform level. The data layer is the largest population of that rule and the easiest place for drift to slip in (every PG/Mongo/Redis dev wants pgAdmin/Compass/Insight). Naming the wraps explicitly removes ambiguity — "is it OK to ship pgAdmin?" has a single answer, in this ADR, and it is no.

### Critical separation rule (restated from UI-CONSOLIDATION-001)

UI code lives **only** in `cave-portal`. The data crates (`cave-pg`, `cave-docdb`, …) export REST/gRPC APIs and rich JSON payloads suitable for portal consumption. They do **not** import React, JSX, axum-html, or any view-layer dependency. Reviewers reject any data-crate PR that adds template rendering, static HTML, or front-end tooling.

### Implementation

The cave-portal track sprint adds a per-data-crate milestone. Cri-track template applies: ~48 portal tests / crate, 10–15 panel components / crate. Initial milestone order follows tenant demand: `cave-pg` → `cave-blob` → `cave-cache` → `cave-streams` → `cave-search` → `cave-vault` → `cave-docdb` → `cave-iceberg` → `cave-datafusion` → `cave-vector`.

Each milestone delivers:
1. REST/gRPC contract on the data crate (read-only first, mutating after RBAC review).
2. React panel set in cave-portal under `crates/cave-portal/src/panels/<crate>/`.
3. Persona-aware view (tenant developer, tenant admin, platform operator) per ADR-PORTAL-PERSONAS-001.
4. Single OIDC auth via cave-auth (ADR-006, ADR-007 federation).
5. ≥ 48 portal tests / crate (parity with cri reference).

## Consequences

### Positive
- One UX, one auth, one keyboard model across all data services.
- Sovereignty: upstream UIs do not need network egress, do not phone home, do not need separate accounts.
- Persona-aware presentation is uniform — tenant-scoped data never leaks between tenants in any panel.
- Rich panel telemetry feeds cave-llm-obs / cave-self-improver loops (panel-usage → which crates need richer admin UX).

### Negative
- ~10 crates × ~12 panels = ~120 React panels — large frontend investment.
- Every upstream UI feature gap (pgAdmin's plan visualizer, Compass's schema sampler) must be ported on demand; ports lag upstream.
- portal frontend bundle size grows; needs code-splitting per panel namespace.

### Risks
| Risk | Mitigation |
|---|---|
| Per-crate UI drifts back into the data crate (developer convenience) | Lint rule in cave-lint: data crates may not depend on `react`/`@cave/portal-panels`. CI gate. |
| pgAdmin/Compass parity gap blocks tenant onboarding | Phased rollout — read-only browser first, mutating ops after parity test. Tenant escape hatch documented (raw SQL/CLI). |
| Bundle bloat slows portal first paint | Per-panel lazy load + route-level code split; baseline budget 300 KB / panel. |

## Related

- **ADR-RUNTIME-UI-CONSOLIDATION-001** — parent invariant; this ADR is its data-layer specialization.
- **ADR-031** — WebApplication composition pattern (panel = composition unit).
- **ADR-PORTAL-PERSONAS-001** — persona-aware view model.
- **ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001** — defines the ten data crates this ADR wraps.
- **ADR-006 / ADR-007** — OIDC auth backends consumed via cave-auth.
- **ADR-047 / ADR-049 / ADR-050 / ADR-021 / ADR-020 / ADR-114** — upstream selections for the wrapped data services.
- **ADR-014** — Zero-Trust; per-panel RBAC + tenant scoping invariant.

## Compliance

- **SOC2 CC6.1** — single AuthN surface for all data admin actions.
- **GDPR Art.32** — tenant scoping enforced uniformly; no per-tool ACL drift.
- **NIS2 Art.21** — admin access auditable via cave-portal action log (cave-audit-pqc signed).

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-28 ADR session.*
