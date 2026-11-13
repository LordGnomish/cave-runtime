# Deep Audit Report — 2026-05-25

**Branch:** `claude/deep-audit-refactor-2026-05-25`
**Base:** `0bd55755` (main)
**Scope:** workspace-wide audit, read-only, no source touch.

## Executive summary

| Area | Status | Note |
|---|---|---|
| Single-binary architecture | ✓ INTACT | 0 helm/, 1 Dockerfile, 0 per-crate K8s manifests; 85 reverted helm charts already purged in main |
| Theme reorg | ✓ COMPLETE | `crates/{theme}/cave-*` pattern, 10 themes, 112 crates, workspace glob `crates/*/*` |
| Cargo build | ✓ 0 errors | 824 warnings (mostly unused imports/vars, auto-fixable) |
| Test compile | ✓ 0 errors | 825 warnings, all targets compile |
| cargo deny | ✓ ok | advisories/bans/licenses/sources all PASS (10 unmatched-license warnings non-blocking) |
| Cycle check | ✓ 0 cycles | 134 cave→cave edges, 111 cave-* packages |
| **Orphan libs** | ✗ 26 ORPHANS | 23% of workspace not depended-on by anything |
| **Charter G5 stubs** | ✗ 89 src stubs | mostly cave-search (20), cave-upstream-watchd (46), cave-kubevirt (7), cave-local-llm (15), cave-acme (6), cave-karpenter (6) |
| Charter G2 SPDX | ✓ 100% | 3011/3011 .rs files |
| Charter G4 manifest | ✓ 100% | 112/112 parity.manifest.toml |
| Charter G1 source_sha | ⚠ 59/112 | in parity.manifest.toml; 53 missing pins |
| Charter G3 fill_ratio ≥0.95 | ⚠ 57/85 | non-infra: 28 at 1.00, 29 at 0.95-0.99, 2 at 0.90-0.95, 26 at 0.00 (unmeasured) |
| Charter G6 no-backcompat | ✓ clean | 0 cgroupv1, 0 docker-shim, 0 in-tree volume |
| Charter G8 4-track | ✗ 6/112 obs | only 6 crates have observability.toml; 0 dashboard.json; 0 alerts.yaml |
| cargo machete | ⚠ 10 packages | 39 unused dep entries (mix of real + false-positive trait deps) |
| Ghost branches | ⚠ heavy clutter | 522 local, 231 remote, 163 worktrees (67 locked, 2 prunable), 36 already-merged safe to delete |

## 1.1 Mimari coherence

### Single-binary integrity ✓
- `helm/`: **does not exist** (85 chart scaffolds reverted in main `739c47e0..0bd55755`)
- Root `Dockerfile`: 1 only
- Per-crate K8s manifests: 0
- ops/ directory: empty (no clutter)

### Theme distribution
```
ai:5  compute:13  core:3  data:15  networking:6
observability:14  ops:27  orchestration:10  registry:3  security:26
                                                              total: 112
```

### Binaries (legitimate independent outputs)
- `cave-runtime` (ops/cave-runtime) — the main platform binary
- `cavectl` (ops/cave-cli) — CLI (package name ≠ directory name — see anomaly below)
- `cave-llm-tracker` (ai/cave-llm-tracker)
- `cave-local-llm` (ai/cave-local-llm)
- `cave-upstream` (ops/cave-upstream)
- `cave-upstream-watchd` (ops/cave-upstream-watchd)

### Anomaly: package vs directory naming
`crates/ops/cave-cli/Cargo.toml` declares `name = "cavectl"`, breaking the
`directory_name == package_name` convention used everywhere else. Either
rename the directory to `cavectl` or the package to `cave-cli`.

### Orphan libs (no rdep, no binary, dead-ended in workspace) — 26
```
ai/cave-hermes              data/cave-cdc              ops/cave-portal-web
compute/cave-ha             data/cave-datafusion       ops/cave-techdocs
compute/cave-kube-proxy     data/cave-iceberg          orchestration/cave-crossplane
compute/cave-kubevirt       data/cave-lakehouse        orchestration/cave-karpenter
networking/cave-ebpf-common data/cave-ledger           orchestration/cave-keda
observability/cave-tracing  data/cave-search           orchestration/cave-knative
ops/cave-desktop            registry/cave-registry     security/cave-gitleaks
ops/cave-permission         security/cave-identity     security/cave-sandbox
ops/cave-portal-api         security/cave-scan-db
```

These crates compile and have parity manifests but **no one depends on them
and they are not binaries**. They are dead weight in the workspace from a
runtime perspective. Options:
1. Wire them into `cave-runtime` (preferred, restores single-binary thesis)
2. Convert to a binary (only if they are standalone tools — e.g. cave-desktop)
3. Delete from workspace if obsolete

### Crate dep graph
- 111 cave-* packages in cargo metadata (112 on disk; cave-cli ships as `cavectl`)
- 134 inter-crate dep edges
- **0 cycles** ✓
- 75 cave-* deps wired into cave-runtime binary (all 75 used in src)

## 1.2 Build & test sağlığı

### `cargo check --workspace --all-targets`
- **Errors: 0**
- Warnings: **824**
- Build time: 1m 36s on cold cache

### `cargo test --workspace --no-run`
- **Errors: 0** (all test binaries compile)
- Warnings: 825

### Warning categories (top)
```
44  unused variable: `mount`
26  unused import: `Permission`
18  unused import: `std::collections::HashMap`
16  unused import: `delete`
11  unused import: `std::sync::Arc`
11  unused import: `Serialize`
 9  unused variable: `state`
 9  unused import: `crate::portal_test_ctx`
 8  unused variable: `pulp_id`
 8  unused variable: `panel`
```
**>95% of warnings are auto-fixable with `cargo fix --workspace --allow-dirty`.**

A handful are real concerns:
- snake_case violations in `cave-portal/src/admin/layout/shortcuts.rs`,
  `toast.rs` — `caveToast_global_is_exposed_for_inline_scripts` etc.
  (test names matching JS API names — cosmetic).

### `cargo deny check`: ✓ all PASS
advisories ok, bans ok, licenses ok, sources ok.
10 advisory `license-not-encountered` warnings (declared allowances for AGPL/GPL/LGPL/OpenSSL/Unicode-DFS that never appeared in the dep graph — harmless, can prune).

## 1.3 Charter v2 8-gate audit

### G1 source_sha — 59/112 (53%)
53 crates lack a `source_sha` pin in `parity.manifest.toml`. List omitted
for brevity; backfill in Phase 2.4.

### G2 SPDX header — **100% (3011/3011 .rs files)** ✓
Perfect coverage.

### G3 fill_ratio ≥0.95 — 57/85 (non-infra), 27 infra excluded
| Bucket | Count |
|---|---|
| 1.00 | 28 |
| 0.95-0.99 | 29 |
| 0.90-0.95 | 2 (cave-sign 0.9487, cave-trace 0.9474 — both 1 nudge away) |
| 0.80-0.90 | 0 |
| 0.50-0.80 | 0 |
| 0.01-0.50 | 0 |
| **0.00** | **26** |

The 26 at 0.00 are mostly unmeasured (manifest exists but ratio not set).
Either real parity gap or hygiene-only crates that need explicit
`infra_only = true` flag.

Crates at 0.00: cave-{admission, ai-obs, alerts, backup, cdc, certs, chaos,
chat, cluster, compliance, cost, devlake, erp, gitops-config, ha,
incidents, infra, pam, permission, pipelines, search, security, slo,
store, tracker, uptime}.

### G4 parity.manifest.toml — **100% (112/112)** ✓

### G5 no-stub — **89 src/ stubs**
| Crate | stubs |
|---|---|
| ops/cave-upstream-watchd | 46 (29 todo! + 17 unimplemented!) |
| data/cave-search | 20 (unimplemented!) |
| ai/cave-local-llm | 15 (10 todo! + 5 unimplemented!) |
| compute/cave-kubevirt | 7 (unimplemented!) |
| security/cave-acme | 6 (unimplemented!) |
| orchestration/cave-karpenter | 6 (unimplemented!) |
| security/cave-scan | 6 (todo!) |
| compute/cave-controller-manager | 3 (unimplemented!) |
| data/cave-rdbms-operator | 3+2 (mixed) |
| ...rest | scattered 1-2 each |

Test/spec/bench stubs are accepted (133 of the 233 total). The 89 in `src/`
are real architectural gaps. Mostly already-known incomplete crates from
parity-index 0.00 bucket — same set.

### G6 no-backcompat — **clean ✓**
- 0 cgroupv1 / cgroup_v1 / cgroups_v1 references
- 0 docker-shim / dockershim references
- 0 in-tree volume / storage references

### G7 always-latest — not audited per-crate
Requires fetching GH `gh release view` per upstream — deferred to Phase 2.4.
Recent rays (2026-05-22 onwards) all pinned latest; sample spot-check ok.

### G8 4-track — **6/112 observability** ✗
- observability.toml: 6 crates (cave-identity, cave-bench, cave-policy,
  cave-falco, cave-sandbox, cave-vault — all security theme)
- dashboard.json: 0
- alerts.yaml: 0
- cavectl wiring: only 2 cave-* crates directly imported in cave-cli/main.rs
  (cave_bench, cave_falco). Native modules (auth, deploy, get, secrets,
  logs, chaos, events, flag, describe, topology, request, watch) are 11
  files but route via cave-cli's own HTTP client, not cave-* lib calls
  directly — which is the correct pattern per ADR-RUNTIME-CLI-CONSOLIDATION-001.

So G8 is heavily paperwork-only outside the security theme.

## 1.4 Duplicate + dead code

### `cargo machete`
- 10 packages with unused declared deps
- 39 total unused dep entries
- High-confidence drops: `cave-vault::rsa`, `cave-cri::tower`, `cave-crm::tower`,
  `cave-erp::tower`, `cave-crossplane::bytes`, `cavectl::axum`,
  `cave-store::axum-test`.
- False positives (used via traits/derives): cave-{identity,bench,sandbox,
  forensics,falco} extensive lists of cave-{auth,core,db,certs,pki} —
  machete misses workspace traits.

### Pub fn duplicate scan (cross-crate)
```
1400  new            (constructors — expected)
 188  render
 155  get
 130  router
 122  len / is_empty
 112  list
 101  validate / parse
  87  create_router / as_str
  71  register
  68  evaluate / delete
```
Most are conventional names (`new`, `len`, `is_empty`, `router`). The 100+
`validate`, 130 `router`, 87 `create_router`, 71 `register` suggest a
shared trait could be hoisted, but this is judgment-call territory.

### Consolidation candidates
- `retry/backoff/with_retry` defined in 9 different crates:
  cave-scheduler, cave-etcd (×2), cave-cloud-controller-manager,
  cave-llm-gateway, cave-workflows, cave-knative, cave-upstream,
  cave-streams. **Strong candidate for `cave_core::retry`** helper.
- `http_client` builder: only 1 instance found (`cave-local-llm`).

## 1.5 Workspace reorg health

- Theme structure: ✓ COMPLETE
- 10 themes (ai/compute/core/data/networking/observability/ops/orchestration/registry/security)
- 112 crates distributed
- Workspace `members = ["crates/*/*"]` glob
- 136 cross-crate `path = "../..."` refs in Cargo.toml files
- 105 cross-theme (`../../theme/`) refs
- 31 same-theme (`../cave-X`) refs
- **All paths resolved correctly** (cargo metadata succeeded with 0 errors).

## 1.6 Ghost branch + worktree inventory

| Category | Count |
|---|---|
| Local branches | 522 |
| Remote branches | 231 |
| `claude/` local | 276 |
| `claude/` remote | 177 |
| Worktrees registered | 163 |
| Worktrees locked | 67 |
| Worktrees prunable | 2 |
| Local branches merged-to-main (safe delete) | 36 |
| Remote `claude/` merged-to-main | 4 |

**The 9df510d1 pre-OSS-launch ghost wave appears already pruned** — sampled
177 claude/ remote branches, none have base at 9df510d1. Most diverge from
recent SHAs (76101add was top with 9 branches).

### Phase 2.7 cleanup targets
- 36 local merged branches → safe `git branch -d`
- 4 remote merged → `git push origin --delete`
- 163 worktrees vs 25 active branches → mass `git worktree remove` for
  prunable/old locked ones (with care — some are sibling-session worktrees)
