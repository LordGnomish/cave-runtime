# Refactor changelog — deep-audit-refactor 2026-05-25

**Branch:** `claude/deep-audit-refactor-2026-05-25`
**Base:** `689497f4` (main after rebase)
**Original fork point:** `0bd55755`
**Phase 1 audit:** [docs/audit/2026-05-25-deep-audit.md](../audit/2026-05-25-deep-audit.md)
**Topology snapshot:** [docs/architecture/topology-2026-05-25.md](../architecture/topology-2026-05-25.md)

## Commits

| SHA (post-rebase) | Phase | Change |
|---|---|---|
| `310169aa` | P1 | docs(audit): Phase 1 deep audit report |
| `bacde581` | P2.5/P2.7 | refactor(workspace): drop 7 unused deps + ignore night-pump Cargo.lock |

A parallel session also discovered the parity-script theme-blindness and
landed `79aef885 chore(parity): full regen post-deep-audit-refactor`
directly on main — their rewrite is broader (724-line script vs my 58-line
patch) and supersedes the patch originally drafted here as `1bed9992`.
This branch defers to their version and drops the patch commit.

## Phase 2 outcomes

### P2.1 — single-binary purge ✓ NO-OP
Verified clean: `helm/` doesn't exist (85 chart scaffolds already reverted
in main `739c47e0..0bd55755`), 1 root `Dockerfile`, 0 per-crate K8s manifests.

### P2.2 — shared primitive consolidation ⏸ DEFERRED
9 sites of `retry/backoff/with_retry` identified across crates:
cave-scheduler, cave-etcd ×2, cave-cloud-controller-manager,
cave-llm-gateway, cave-workflows, cave-knative, cave-upstream,
cave-streams. Out-of-scope for this ray — would need a `cave_core::retry`
helper and 9 call-site migrations with careful semantic preservation.
Recommend separate ray.

### P2.3 — workspace reorg ✓ ALREADY DONE
Theme reorg pre-existed (commit `9c15b3fb` in main). Verified:
10 themes (ai/compute/core/data/networking/observability/ops/orchestration/registry/security),
112 crates, workspace glob `members = ["crates/*/*"]`, 136 cross-crate
path refs all resolve (cargo metadata succeeded 0 errors).

### P2.4 — Charter v2 uniform gate fix ⏸ PARTIAL
- G2 SPDX: 100% already ✓
- G4 parity.manifest.toml: 100% already ✓
- G6 no-backcompat: clean already ✓
- G1 source_sha: 59/112 (53 unfilled — per-crate backfill is its own ray)
- G5 stubs: 89 in src/, mostly in known-incomplete crates (cave-search 20,
  cave-upstream-watchd 46, cave-kubevirt 7, cave-local-llm 15, cave-acme 6,
  cave-karpenter 6) — each is a real port not yet started
- G7 always-latest: not audited per-crate (would need `gh release view`
  call per upstream — recent rays from 2026-05-22 onwards pinned latest)
- G8 4-track observability: only 6 crates have observability.toml — deep work

Not fixed in this ray because each gap requires per-crate work (not a
mechanical sweep). Documented for follow-up.

### P2.5 — cargo fix sweep ✗ REVERTED
`cargo fix --workspace --allow-dirty --allow-staged --all-targets` was
attempted. It broke 2 crates by stripping cfg(test) imports it couldn't
verify in non-test context:

- `cave-gateway` (lib test): 1 error (HealthCheckType undeclared in test mod)
- `cave-apiserver` (lib test): 11 errors
- `cave-portal` (lib test): 141 errors

This is the same cfg(test) footgun documented in memory
`refactor-sweep-2026-05-23.md` 2.6. Reverted entirely with
`git checkout -- .`. 824 warnings remain in place. Per-crate manual
cleanup is recommended over `cargo fix --workspace`.

### P2.6 — naming consistency ⏸ DEFERRED
One concrete anomaly noted: `crates/ops/cave-cli/Cargo.toml` declares
`name = "cavectl"` (directory ≠ package name). All other crates match.
Fix is either `git mv crates/ops/cave-cli crates/ops/cavectl` (and update
~20 `path = "../cave-cli"` refs) or rename the package back to `cave-cli`.
Out-of-scope: requires a careful mass rewrite across Cargo.toml + docs.

### P2.7 — ghost branch + worktree cleanup ✓ DONE (limited)
Deleted 9 local merged branches with no attached worktree:
- claude/doc-sync-2026-05-24
- claude/finisher-2-2026-05-24
- claude/test-uplift-2026-05-25
- worktree-agent-a71d2bb5847c8cd8c
- worktree-agent-a72a17f50d538a387
- worktree-agent-a77f29f228696d374
- worktree-agent-a8bbe4a1b1ea8e346
- worktree-agent-a9b6fc3966f26b59c
- worktree-agent-a9d77babbd827ebb3

Deleted 4 merged remote claude branches via `git push origin --delete`:
- origin/claude/portal-v3-1779861138
- origin/claude/sec-audit-2026-05-24
- origin/claude/test-uplift-2026-05-25
- origin/claude/theme-reorg-2026-05-25

Pruned 2 worktrees with missing gitdir:
- worktrees/cave-deptrack-worktree
- worktrees/cave-uplift-wave2-2026-05-24

**Not touched:** 67 locked worktrees + 28 merged branches WITH attached
worktrees. These belong to sibling agent sessions and would interfere if
removed. Their cleanup requires coordination — separate ray.

### P2.8 — parity-index regen ✓ FIXED (by parallel ray) + DONE
`scripts/build-parity-index.py` had a critical bug post-theme-reorg:
all manifest lookups used the legacy flat path `crates/<crate>/...`.
A patch was drafted on this branch (originally `1bed9992`); a parallel
ray landed a more comprehensive rewrite directly on main as
`79aef885 chore(parity): full regen post-deep-audit-refactor`. After
rebase, this branch defers entirely to that rewrite. Regen output went
from 98 crates (14 missing, 99 unfilled, 98 phantoms) to **112 crates,
112 manifest_filled, 0 phantoms**. 99 disk-overlay flips and 71 ratio
overrides surfaced.

Final bucket distribution (non-infra, 85 crates):
| Bucket | Count |
|---|---|
| 1.00 | 28 |
| 0.95-0.99 | 29 |
| 0.90-0.95 | 2 (cave-sign 0.9487, cave-trace 0.9474) |
| 0.80-0.90 | 0 |
| 0.50-0.80 | 0 |
| 0.01-0.50 | 0 |
| 0.00 (unmeasured) | 53 |
| infra_only (excluded) | 27 |

## Refactor scope NOT changed

Per the ray's explicit "no source touch beyond consolidation + fix"
constraint, the following audit findings are reported but unchanged:

1. **26 orphan libs** (no rdep, no binary) — biggest finding, 23% of workspace
2. **89 src/ stubs** in known-incomplete crates
3. **824 build warnings** (>95% auto-fixable but cargo fix breaks tests)
4. **522 local branches, 161 worktrees** (90% are sibling-session artifacts)
5. **cave-cli vs cavectl naming mismatch**

These deserve dedicated follow-up rays.

## LOC delta (this branch's commits only — script fix delegated to main)

| Change | Files | +ins | -dels |
|---|---|---|---|
| Audit report | +1 | +257 | 0 |
| Dep cleanup | 9 | +3 | -221 |
| Refactor changelog + topology | +2 | +234 | 0 |
| **Total** | **12** | **+494** | **-221** |

## Net effect

- Workspace builds clean: 0 errors, 824 warnings (unchanged)
- Dep tree: −7 unused workspace deps, Cargo.lock −214 lines
- Branch count: 522 → 513 local, 231 → 227 remote, 163 → 161 worktrees
- parity-index integrity restored (112/112 vs 98/112) via parallel ray
- Phase 1 audit baseline documented for future rays
