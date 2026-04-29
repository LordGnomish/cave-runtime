---
date: 2026-04-29
author: Claude (autonomous run)
status: merge HALTED, sprint preserved on branch
sprint_branch: feat/cave-net-real-100
sprint_tip: 31a7854891e7393aa4830aaaa723f184cccbd220
target: main (0041890)
---

# Honest note: why the merge to `main` was not executed

The sprint instruction was to autonomously merge `feat/cave-net-real-100`
into `main` with a `-X theirs` strategy, build/test, and persist the parity
report. The instruction also said: **"Tehlikeli durumda branch'te bırak +
dürüst not."**

The situation triggered that escape clause. The sprint deliverables (build,
test count, parity report) are validated and preserved. The merge itself
was deliberately not performed, for the reasons below.

## What was done (safe, validated)

* `cargo build --release -p cave-net` on `feat/cave-net-real-100` → clean.
* `cargo test -p cave-net --release` on `feat/cave-net-real-100` → **1759
  passed, 0 failed, 5 ignored** (matches `PARITY_REPORT.md` claim exactly:
  1697 lib + 56 e2e + 0 qwen-drafted + 6 wire-faithful + 0 doc).
* For comparison, ran the same on `cave-net-cilium-100pct` worktree at
  commit `53c6607` (which is on main's history, last cave-net touch before
  the observability merges): **1556 passed**. Confirms the +203 delta.
* `docs/parity/cave-net-2026-04-29.md` written as the dated permanent copy
  of `crates/cave-net/PARITY_REPORT.md`.

## Why the merge was halted

### 1. Main worktree has ~150 files of uncommitted in-flight work

The `main` worktree at `/Users/gnomish/Code/cave-runtime/.claude/worktrees/hopeful-poincare-b0ede9`
showed (at the time of this run) extensive staged additions and modifications
spanning what looks like several integration sprints:

* New apiserver v3 modules: `aggregator_v3`, `audit_policy_v3`, `discovery_v3`,
  `webhook_admission_v2`, `simple_cel`.
* New controller-manager modules: `csr_auto_approver`, `csr_pem`, `gc_lite/podgc_deeper`,
  `gc_lite/ttl_jitter`, `node_lease_deeper`, `pv/attach_detach`, `pv/protection`,
  `rbac/aggregation_conflict`, `root_ca_deeper`, `sa/legacy_token_cleaner`.
* New CRI v2 modules: `auth_v2`, `cgroup_v2_runtime`, `criu_v2`, `streaming_session`, `userns_v2`.
* New etcd modules: `auth_full`, `client_full`, `cluster_admin`, `maintenance_full`.
* New kubelet modules: `admission`, `cgroup_manager`, `eviction_api`, `lifecycle`,
  `restart_backoff`.
* New portal modules: `plugins/grafana_alerting`, `wraps`; new portal-api `routes/progress`.
* Two new ADRs: `ADR-048-data-persistence-portal-wraps`, `ADR-CHARTER-002-deployment-profiles-trinity`.
* Test additions in dozens of `tests/qwen_drafted.rs` files across the workspace.
* A sweeping deletion of the old `observability/` tree (~80 dashboards + ~80 alerts
  + `generate.py` + `validate.py` + `README.md`).
* A handful of unstaged modifications in apiserver (`simple_cel.rs`, `vap_advanced.rs`,
  `vap_advanced/tests.rs`, `tests/qwen_drafted.rs`).

None of this looks committed on any branch I could find. The instruction's
"stash veya WIP commit" step is technically reversible, but the surface area
is large enough that a wrong call about which files belong together (and the
observability deletion clearly belongs to a *different* track than the
controller-manager additions) could entangle several sprints' worth of work.

### 2. `-X theirs` would silently overwrite main's M37–M52 cilium track

`git merge-base main feat/cave-net-real-100` returns the very initial commit
`ac51758`. The two branches share no history beyond that. The feat branch
has only two commits past the initial: `3533651` (cave-upstream watchd) and
`31a7854` (the cilium parity sprint).

Meanwhile, main has independently developed cave-net through milestones
**M37–M52** (visible in `git log main`: 16+ "feat(cave-net): Mxx — …"
commits ending at `88ba9a8` — "🎯 1500 milestone"), then `53c6607`
("clean release build + 56 cross-module e2e parity tests"), then the
observability merges.

Of the 99 files the feat branch touches, **70 also exist on main with
independent implementations** (full list: see `git diff --name-only
ac51758..feat/cave-net-real-100` cross-checked against `git diff --name-only
ac51758..main`). On those 70 files a `-X theirs` strategy would unconditionally
choose the feat branch's version.

Concretely: tip-of-main's `cave-net` passes 1556 tests; the feat branch
passes 1759. The +203 delta is genuine, but the test sets are not strict
supersets — they are two parallel implementations of the same upstream
surface. A `theirs`-strategy merge produces 1759 tests on main, but the
specific behaviours pinned by main's M37–M52 tests that aren't pinned by
the feat branch's would be silently dropped without anyone reading them
side-by-side.

This is exactly the class of decision that benefits from human eyes on the
diff, not autonomous "decide and proceed".

### 3. The merge would commit on top of #1 above

Even if the cilium-overwrite question were resolved, the merge commit
would land on a working tree whose index already contains ~150 files of
half-finished feature work. The merge index conflict resolution would
interact with the existing staged state. Recovering from a bad outcome
would be hard.

## What remains for a human to decide

A safe forward path looks something like:

1. **Sort out main's uncommitted state first.** The work staged on the main
   worktree at `hopeful-poincare-b0ede9` should land as one or more proper
   commits on its own feature branches before any other branch lands on
   main. The author of those changes is the only one who can decide the
   right grouping.
2. **Decide the cilium reconciliation strategy explicitly.** Either:
   - Accept that the parity-sprint version is the canonical cave-net cilium
     and explicitly retire main's M37–M52 cilium files, OR
   - Rebase the parity sprint on top of main so the new code is *additive*
     to M37–M52 rather than parallel, OR
   - Cherry-pick only the new ported pkgs (the 17 in the parity report) onto
     main, skipping the files that already exist there.
3. Then merge / rebase / cherry-pick accordingly, with conflict resolution
   reviewed by hand.

## Sprint deliverables that ARE intact

* The sprint code itself: `feat/cave-net-real-100` @ `31a7854` is untouched.
* Build is clean, 1759 tests pass.
* `crates/cave-net/PARITY_REPORT.md` is in tree on the sprint branch.
* `docs/parity/cave-net-2026-04-29.md` is added to the sprint branch as the
  permanent dated record.
* This note (`docs/parity/cave-net-2026-04-29-merge-halt-note.md`) documents
  the autonomous-run decision so the human picking it up has full context.
