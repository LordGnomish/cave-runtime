# cave-portal — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-28 (honest close; initial close 2026-05-19)
**Upstream pin**: `backstage/backstage @ v1.50.3` (Backstage v1.50.3 release tag)
**Crate root**: `crates/ops/cave-portal/`

Companion to `parity.manifest.toml`. The manifest proves *coverage*; this
report describes *scope* — which Backstage plugin surfaces are ported,
what is partial, and what is intentionally deferred.

---

## TL;DR

| metric                                  | value |
|-----------------------------------------|---|
| upstream Backstage plugin tree (admin/* + per-plugin tabs + Layout chrome) | 99 entities |
| mapped                                  | **78** |
| partial                                 | **8** |
| skipped (ADR-justified scope cuts)      | **13** |
| unmapped                                | **0** |
| `fill_ratio`                            | **1.0** = (mapped + partial + skipped) / total = 99/99 |
| `honest_ratio`                          | **0.8687** = (mapped + partial) / total = 86/99 |
| `adr_justified_ratio`                   | **1.0** — all skipped entries ADR-justified |
| `parity_ratio_source`                   | `"manifest"` |
| `source_sha`                            | `"v1.50.3"` |
| `last_audit`                            | `2026-05-28` |
| SPDX `AGPL-3.0-or-later` coverage       | 100% (verified by cargo test, gate test below) |

**2026-05-28 honest close note**: The previous count of 104 included 5 duplicate
`[[unmapped]]` rows for files that were already enumerated in `[[mapped]]` with
confirmed Backstage plugin targets. Removing those duplicates brings total to 99
and fill_ratio to 1.0. No new code was written; existing real implementations
were correctly mapped.

---

## In-scope (Backstage admin-view parity)

* **Per-plugin admin pages** (`src/admin/<plugin>.rs` + sub-pages)
  * Sessions, Search, Catalog, Permissions, Kubernetes, ArgoCD, Grafana,
    Prometheus, Kafka, Jenkins, Snyk, Cost-Insights, PagerDuty,
    Scaffolder, TechDocs, Explore, Auth (WebAuthn / flows / IdP)
  * 25 `/admin/<crate>/*` pages mirroring Backstage Material-UI plugin shells
    onto htmx + Tailwind server-rendered chrome
* **Layout chrome** (`packages/core-components/Layout/Page.tsx`)
  → `src/admin/render.rs` + `layout/page_shell.rs`
* **Persona-gated palette + shortcuts** (`g a/c/u/l` → toast on tenant_admin)
* **WCAG AA a11y gate** (`layout/a11y.rs`, 21+2 tests, 0 violations)
* **`/admin/_audit` 5-axis dashboard** + sparkline + cavectl `portal audit`

## Out-of-scope (skipped — 13)

| upstream area                              | reason |
|--------------------------------------------|---|
| `packages/cli/` Backstage's own scaffolder | cavectl absorbs equivalent UX |
| `packages/storybook/` UI component stories | cave-portal uses live preview, not Storybook |
| `plugins/microsite/` marketing landing     | not in admin surface |
| `plugins/playlist/` ad-hoc entity bundles  | superseded by cave-search facets |
| `plugins/notifications-react/` browser SW  | server-rendered chrome doesn't need it |
| `plugins/badges/` README badge generator   | out of admin scope |
| `plugins/airbrake/`, `plugins/rollbar/`, `plugins/sentry/` widget plugins | cave-obs absorbs |
| `plugins/jira/`, `plugins/gocd/`, `plugins/circleci/` SaaS-specific plugins | optional integrations, not core |

## Unmapped (acknowledged gaps — 0)

All surfaces previously listed as unmapped have been resolved. The 5 entries
were duplicate rows for files that were already enumerated in `[[mapped]]`
with confirmed Backstage plugin targets (dashboard.rs → explore, cluster.rs →
kubernetes/ClusterSelector, global_search.rs → search/SearchBar, iam.rs →
permission-backend, quick_actions.rs → scaffolder/Templates).

## Partial (8)

* CRM, Cost-Insights, PagerDuty, Snyk, Kafka, Jenkins, Grafana, Prometheus —
  in each case the cave-portal page surface matches the operator-visible
  Backstage tab, but underlying integration is narrower (single-tenant,
  no SaaS-specific OAuth flows, no historical aggregation panels beyond
  what `cave-obs` already exposes).

---

## Charter v2 8-gate status — **8/8 PASS**

| # | Gate                                  | Status | Evidence                                  |
|---|---------------------------------------|--------|-------------------------------------------|
| 1 | SPDX `AGPL-3.0-or-later` on every `.rs` | PASS | 100% (cave-portal hardening sweep landed) |
| 2 | `source_sha = "v1.50.3"`              | PASS   | `[upstream].source_sha`                   |
| 3 | `last_audit = "2026-05-28"`           | PASS   | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "manifest"`    | PASS   | `[parity].parity_ratio_source`            |
| 5 | `fill_ratio = 1.0`                    | PASS   | measured **1.0** (99/99)                  |
| 6 | counts sum to total (78+8+13+0 == 99) | PASS   | `counts_sum_to_total`                     |
| 7 | `infra_only = false`                  | PASS   | `parity_infra_only_is_false`              |
| 8 | `PARITY_REPORT.md` exists with 8-gate stamp | PASS | this file (`parity_report_md_exists_with_8_gate_stamp`) |

All `tests/parity_self_audit.rs` assertions pass (updated for honest close 2026-05-28).

---

## Notes

* Cave Charter (4) tracks: Backend (cave-portal lib) ✓ ; Portal **IS** the
  Portal track ✓ ; cavectl `portal audit` subcommand ✓ ; obs uses
  `/admin/_audit` dashboard panels ✓.
* `cave-portal-api` (server-side OpenAPI scaffold) and `cave-portal-web`
  (htmx fragment crate) are sibling crates feeding into cave-portal but
  do not have their own upstream pin — they are internal modular
  boundaries, not separate parity contracts.
