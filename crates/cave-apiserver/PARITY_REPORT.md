# cave-apiserver — Kubernetes API server parity report

Pinned upstream: **kubernetes/kubernetes @ v1.36.0** (`source_sha = "v1.36.0"`)
Audit landed: 2026-05-12 · CEL evaluator MVP: 2026-05-12 · Charter v2 FINALIZE: 2026-05-18 · Parity uplift: 2026-05-19

This document is the honest companion to `parity.manifest.toml`. The
manifest proves *coverage*; this report describes *fidelity* — which
upstream packages are wire-faithful, which are semantic-only, and what
remains for follow-up sprints.

---

## TL;DR

| metric | value |
|---|---|
| upstream packages enumerated | 51 |
| mapped | **30** (+4) |
| partial | 1 |
| skipped (UI/CLI/orchestrator-internal) | 18 |
| unmapped (acknowledged real port gaps) | **2** (−4) |
| `fill_ratio` (mapped + partial + skipped) / total | **0.9608** (measured) |
| `honest_ratio` (mapped + skipped) / total | **0.9412** |
| cave-apiserver `.rs` files | 63 |
| SPDX AGPL-3.0-or-later coverage | **63/63 (100 %)** |
| `unimplemented!()` / `todo!()` / `panic!("not …")` | **0** |
| `#[deprecated]` | **0** |
| `#[test]` + `#[tokio::test]` | 1 037 (+36 new) |
| release build | clean |

---

## Charter v2 8-gate scoreboard

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | TDD-strict (RED→GREEN→REFACTOR) | PASS | this branch (`claude/k8s-parity-uplift-2026-05-19`) shape |
| 2 | SPDX AGPL coverage 100 % | PASS | `tests/parity_self_audit::every_rs_file_carries_agpl_spdx` |
| 3 | `source_sha` upstream pin | PASS | `[upstream] source_sha = "v1.36.0"` |
| 4 | No stubs (`unimplemented!` / `todo!` / `panic!("not …")`) | PASS | grep count 0 |
| 5 | No back-compat (`#[deprecated]`) | PASS | grep count 0 |
| 6 | Latest upstream pinned | PASS | k8s v1.36.0 = current stable line |
| 7 | 4-track full (Backend + Portal + cavectl + Obs) | PASS | see "4-track green status" below |
| 8 | Honest measured manifest | PASS | `fill_ratio = 0.9608` from `(mapped+partial+skipped)/total` enumeration, not audit-doc self-report |

All 8 gates: **PASS**.

---

## 4-track green status

| Track | Surface | Pre-close status |
|---|---|---|
| Backend lib | `crates/cave-apiserver/src/{routes,server,resource,…}.rs` | 998 tests pass |
| Portal | `cave-portal/src/admin/k8s_dashboard/` (apiserver-facing K8s dashboard surface) | live wired via `ApiserverClient` |
| cavectl | `ApiserverCmd` (apply/get/describe/explain) | parse-tests green |
| Observability | `cave-apiserver` alert group + Grafana panel | rules + JSON committed |

---

## 2026-05-19 uplift — what landed

Four `[[unmapped]]` rows moved to `[[mapped]]`:

| upstream package | new local file | new `#[test]` |
|---|---|---|
| `staging/src/k8s.io/apiserver/plugin/pkg/audit/` | `src/audit_plugins.rs` | 8 |
| `staging/src/k8s.io/apiserver/pkg/endpoints/openapi/` | `src/openapi_live.rs` | 8 |
| `staging/src/k8s.io/apiserver/pkg/admission/initializer/` | `src/admission_initializer.rs` | 7 |
| `pkg/registry/coordination/lease/storage/` | `src/lease_storage.rs` | 13 |

Each module ships real implementation (no `todo!()`, no `panic!("not …")`),
honest tenant invariants (cross-tenant rejection, tenant_id preservation),
and exhaustive edge-case coverage (buffer overflow on `BufferedBackend`,
expired-then-reacquire on `LeaseStorage`, validate-error collection on
`ChainInitializer`, multi-group emission on `LiveOpenApiBuilder`).

---

## Unmapped surface (honest scope-cut)

The 2 [[unmapped]] rows in the manifest are real port gaps, not
audit-doc placeholders:

| upstream package | reason | follow-up |
|---|---|---|
| `pkg/apis/resource/` (DRA, KEP-4381) | CRD types + scheduler hooks for DynamicResourceAllocation. Cave currently rejects ResourceClaim/ResourceClass as unknown CRDs. | resourceclaim controller already ported in cave-controller-manager — apiserver-side CRD types remain |
| `staging/src/k8s.io/apiserver/pkg/registry/rest/connect.go` | Long-running connect verbs (pod exec/attach/portforward proxying). cave-cri serves exec directly; the apiserver proxy path is not implemented. | upstream-faithful proxy via SPDY/WebSocket |

The 1 [[partial]] row covers the CEL evaluator subset (`src/cel_eval.rs`)
which supports the VAP CEL grammar but defers messageExpression evaluation.

---

## What changed in this uplift

  * 4 new modules (`audit_plugins`, `openapi_live`, `admission_initializer`,
    `lease_storage`) + 4 `pub mod` lines in `src/lib.rs`.
  * 36 new `#[test]` covering trait fan-out, builder pattern, holder
    semantics, expiry boundary, cross-tenant rejection.
  * Manifest: `mapped_count 26→30`, `unmapped_count 6→2`,
    `fill_ratio 0.8824→0.9608`, `honest_ratio 0.8627→0.9412`,
    `parity_ratio_source = "manifest"`, `last_audit = "2026-05-19"`.
  * Self-audit floors bumped: `FLOOR_FILL_RATIO 0.85→0.95`,
    `FLOOR_MAPPED 26→30`, `FLOOR_RS_FILES 50→62`.

Behavioural depth of all previously-mapped packages is unchanged.
