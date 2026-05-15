# ADR-145 — CRM Upstream Selection: Twenty

- **Status:** Accepted
- **Date:** 2026-05-05
- **Decision owner:** Burak Tartan
- **Implemented by:** `crates/cave-crm/` (function-based, ADR-147 naming)
- **Upstream:** [twentyhq/twenty](https://github.com/twentyhq/twenty) — v2.2.0 (released 2026-05-04)
- **License:** AGPL-3.0 (upstream); cave-crm reimplementation under workspace Apache-2.0

## Context

The cave-runtime best-of-breed consolidation charter requires a sovereign CRM
upstream as a first-class business application alongside the existing ERP module
(`cave-erp`, ERPNext-compatible). Until now, CRM functionality has been
prototyped as a *submodule* inside `cave-erp` (`crates/cave-erp/src/modules/crm.rs`),
mirroring Odoo's bundled architecture. This bundling has two issues:

1. ERPNext is ERP-first; its CRM subsurface lags behind dedicated CRM platforms
   in data model richness (e.g. activity timeline, custom objects).
2. Bundling forces ERP and CRM release cadences to couple, which conflicts with
   the four-track release model (Backend / Portal / cavectl / Observability)
   declared in the Cave Runtime Golden Rules.

We need a dedicated CRM upstream that we can track independently and mirror
into a standalone `cave-crm` crate.

## Decision

Adopt **Twenty** ([twentyhq/twenty](https://github.com/twentyhq/twenty)) as the
sovereign CRM upstream. Implement a standalone `cave-crm` crate (function-based
naming per ADR-147), independent from `cave-erp`'s CRM submodule. Deprecate
`crates/cave-erp/src/modules/crm.rs` for removal in the v0.2 OSS launch hygiene
pass.

## Alternatives considered

| Candidate | Stack | License | Verdict |
|-----------|-------|---------|---------|
| **Twenty** | TypeScript + NestJS + Postgres + GraphQL | AGPL-3.0 | **Selected** |
| SuiteCRM | PHP (legacy) | AGPL-3.0 | Rejected — entrenched PHP stack, reduced dev velocity |
| EspoCRM | PHP | AGPL-3.0 | Rejected — same PHP concern; smaller community than Twenty |
| Krayin | PHP / Laravel | MIT | Rejected — Laravel-bound, smaller surface |
| Keep `cave-erp/crm.rs` only | — | — | Rejected — ERPNext's CRM is ERP-bundled and lags dedicated CRMs |

## Decision rationale

1. **Modern stack alignment.** Twenty's Postgres + GraphQL spine matches
   `cave-rdbms-operator` (ADR-147) and the cave data plane assumptions; no
   PHP-specific runtime infrastructure required.
2. **License alignment.** AGPL-3.0 aligns with the sovereign-OSS posture the
   workspace already accepts for `cave-erp`. cave-crm itself is an independent
   reimplementation in Rust and inherits the workspace Apache-2.0.
3. **Momentum.** Twenty is a Y Combinator W23 cohort project with active
   weekly releases; v2.2.0 (2026-05-04) provides a stable scaffold target.
4. **Cleanest separation.** Standalone CRM upstream lets `cave-crm` evolve on
   an independent cadence from `cave-erp`, matching the four-track release
   model.

## Consequences

### Immediate (this PR)

- New crate `crates/cave-crm/` scaffolded with `lib.rs`, `models/`, `store.rs`,
  `routes.rs`, `parity.manifest.toml`, and 5 ignored placeholder tests.
- Workspace `Cargo.toml` and `cave-runtime` Cargo.toml updated.
- `cave-runtime/src/main.rs` wires `cave_crm::router` (mirroring
  `cave_erp::router`); surfaces `/api/crm/health`.
- `cave-upstream/src/projects.rs` adds Twenty as a `TrackedProject`
  (category `crm`, phase 4, biweekly check).
- `cave-erp/src/modules/crm.rs` flagged with a deprecation notice; behavior
  unchanged in this PR.

### Pending v0.2 (out of scope here)

- Remove `cave-erp/src/modules/crm.rs` entirely (OSS launch hygiene).
- Portal track — CRM page in `cave-portal-web`.
- cavectl track — `cavectl crm ...` subcommands.
- Observability track — Grafana dashboard + Prometheus alert rules.
- Deep parity tests (real Person/Company/Opportunity/Activity CRUD against
  Twenty's REST + GraphQL surfaces).
- PostgreSQL backend (replacing the in-memory `RwLock` placeholder), driven
  through `cave-rdbms-operator`.
- Tenant isolation per ADR-MULTI-TENANT-001 — Kamaji vCluster boundary.

### Charter compliance

- **Always-latest mandate.** Upstream version `v2.2.0` was fetched from the
  GitHub releases API at scaffold time (2026-05-05) and stamped into both
  `parity.manifest.toml` and the `routes::health` payload.
- **No-backcompat / PQC-ready.** New crate; Linux 7.1 only, no legacy shims.
- **Function-based naming (ADR-147).** Crate is `cave-crm`, not `cave-twenty`.
- **No-stub / no-mock golden rule.** Skeleton is honest — placeholder tests
  are `#[ignore]` with a clear reason, not silently passing.
- **Four-track full standard.** This PR delivers Backend wiring only (1/4);
  Portal + cavectl + Observability tracks are explicitly listed above as
  pending v0.2.

## References

- ADR-147 — Persistence Consolidation (function-based crate naming pattern).
- ADR-MULTI-TENANT-001 — Kamaji multi-tenant boundary.
- `crates/cave-erp/src/modules/crm.rs` — deprecated CRM submodule (kept in
  this PR; removed v0.2).
- Twenty release notes: https://github.com/twentyhq/twenty/releases/tag/v2.2.0
