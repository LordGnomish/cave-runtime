<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-karpenter deep-port — continuation ray #4 progress

**Branch:** `claude/cave-karpenter-honest-100-cont4-*`
**Base:** `b16d5eb9` (continuation ray #3, LOC ratio 0.1597)
**Upstream:** kubernetes-sigs/karpenter **v1.12.1** (sha `ed490e8`, Apache-2.0)
**Scope:** core port, cloud-agnostic — AWS/cloud providers excluded per
ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001.

## Method

Strict TDD, two-commit cadence per cycle: a `test(...)` commit carrying the RED
proof (the compile-fail / assertion-fail output pasted in the body), then a
`feat(...)` commit carrying the GREEN proof. No commit mixes test + impl.
`git log --oneline` is the audit trail.

## Cycles

| Cycle | Module | Upstream | RED SHA | GREEN SHA | Tests |
|-------|--------|----------|---------|-----------|-------|
| 12 | `hash`            | `pkg/apis/v1/nodepool.go` Hash() + hashstructure/v2 | `7b094b6d` | `f3958e25` | 14 |
| 13 | `nodepool_utils`  | `pkg/utils/nodepool` OrderByWeight                  | `…(RED)`  | `02a1e8f9` | 7  |
| 14 | `hash_controller` | `pkg/controllers/nodepool/hash`                     | `…(RED)`  | `87b529ad` | 10 |
| 15 | `disruption` emptiness | `pkg/controllers/disruption/emptiness.go`      | `…(RED)`  | `3995ece2` | 7  |
| 16 | `binpack` daemonset | `provisioning/scheduling/scheduler.go`            | `…(RED)`  | `7a219a81` | 7  |
| 17 | `cluster_state`   | `state/cluster.go` + `disruption/candidate.go`      | `…(RED)`  | `d7767554` | 8  |

**Cycle 12 — NodePool.Hash() / hashstructure FormatV2.** Closes the apis/v1
tail item cont3 deferred. `src/hash.rs` ports the *combination structure* of
mitchellh/hashstructure `FormatV2`: hand-rolled FNV-1 (64-bit, matching Go
`hash/fnv.New64`), `ordered(key,value)` field hashing XOR-accumulated
(`unordered`) and re-run through `finish_unordered`, `SlicesAsSets` set
semantics, `IgnoreZeroValue` + `ZeroNil` + an `ignore_keys` model of the
`hash:"ignore"` tag. `nodepool_hash` hashes `[template.spec, labels,
annotations]` and renders the `u64` decimal like Go `fmt.Sprint`. Byte-exact Go
parity is unreachable across the reflection/serde boundary; the ported contract
is the drift-detection semantics (deterministic, set-order-independent,
zero-field-insensitive, spec-change-sensitive). Dependency-free. NOTICE gains
mitchellh/hashstructure (MIT).

**Cycle 13 — nodepool OrderByWeight.** `src/nodepool_utils.rs`:
`effective_weight` (`lo.FromPtr`, nil → 0), in-place `order_by_weight`
(weight-descending, name-ascending tie-break) and a non-mutating
`ordered_by_weight` clone. Wired into `schedule_first_match` so the
highest-weight *matching* pool wins instead of input order, matching the
upstream provisioner. The three pre-existing single-pool scheduler tests are
unaffected.

**Cycle 14 — NodePool hash controller (controllers/nodepool).**
`src/hash_controller.rs` ports `pkg/controllers/nodepool/hash`: the
`nodepool-hash` / `nodepool-hash-version` annotation keys + `HashVersion`
`"v3"`, idempotent `stamp_nodepool_hash` (over cycle 12), `nodepool_hash_drifted`
(recompute + compare), and `reconcile_hashes` (stamp pools, sync unstamped
claims, *preserve* existing claim hashes so `disruption::drift_candidates` still
fires on real drift). Closes the gap where `pool.template_hash` was externally
supplied — the controller now computes it from the spec.

**Cycle 15 — emptiness consolidation + ConsolidationPolicy gate.**
`src/disruption.rs` gains `DisruptionReason::Empty` (budget reason `"Empty"`),
`empty_candidates` (zero-utilization, highest confidence), the `WhenEmpty` /
`WhenEmptyOrUnderutilized` policy constants, and `consolidation_decisions`
applying the policy gate (empty always consolidated; underutilized only under
the OR-policy; nil policy defaults to OR). The pre-existing
`consolidation_candidates` / `budget_cap_for` arms are preserved.

**Cycle 16 — provisioning DaemonSet overhead.** `src/binpack.rs` gains
`daemon_overhead` (sum of DaemonSet requests) and `binpack_with_daemonset`,
which reserves that overhead on every candidate node (dropping instance types
too small to host it) before delegating to the existing binpacker. Remaining
capacity now reports post-DaemonSet allocatable, matching the upstream
scheduler. Zero overhead is behaviourally identical to plain `binpack`.

**Cycle 17 — cluster state nomination + deletion gate.**
`src/cluster_state.rs` ports the node-nomination and deletion-marking surface of
`state/cluster.go`: time-windowed `nominate` / `is_nominated`,
`mark_for_deletion` / `unmark_for_deletion` / `is_marked_for_deletion`, and
`is_disruption_candidate` (neither nominated-within-window nor marked).
`filter_disruptable` applies the `candidate.go` gate over disruption
`Decision`s, dropping protected nodes so the disruption controller cannot undo a
just-made scheduling placement. Time is threaded explicitly for pure,
deterministic logic.

## Metrics

| Metric | Ray #3 end | Ray #4 end | Δ |
|--------|-----------|-----------|---|
| cave `src` total LOC | 5,555 | 6,085 | +530 |
| LOC honest_ratio (/34,772) | 0.1597 | **0.1750** | +0.0153 |
| cave-karpenter active tests | 304 | **357** | +53 |
| TDD cycles (this ray) | — | **6** | — |
| manifest `mapped_count` | 19 | 19 | 0 (anti-inflation) |

The cont4 ray walks the full cont3 "next" queue: the apis/v1 hash tail, a utils
helper (OrderByWeight), the controllers/nodepool hash controller, a disruption
deepening (emptiness + policy), a provisioning-sim deepening (DaemonSet
overhead), and the first cluster-state slice (nomination + deletion gate). The
manifest gains descriptive `[[files]]` rows; the scalar `mapped_count` /
`honest_ratio` (19 / 0.8636) stay pinned to the v1.4.0 subsystem baseline (the
index is hook-regenerated from the scalars, not the `[[files]]` count).

## cli / portal wiring

cave-karpenter is a true leaf crate — `cave-cli` and `cave-runtime`/`cave-portal`
do **not** link it. The existing karpenter surface (`cavectl karpenter
nodepools|nodeclaims|drift` against `/api/karpenter/*`, plus the
`admin/karpenter` dashboard over a portal-local `NodePool`) was wired in an
earlier phase against portal-local structs. The cont4 cycles deepen library
internals (hash engine, ordering, controllers, simulation, cluster state) that
feed the Phase-3 cross-crate wiring (manifest 4-track: Portal 0/4, cavectl 0/4).
No stub CLI commands were fabricated to satisfy a per-cycle wiring template —
that would violate the no-stub red line and the `assertion_8_no_stub_macros`
self-audit.

## Remaining work (for continuation ray #5)

**utils tail:** `pkg/utils/nodeclaim`, `pkg/utils/node`, `pkg/utils/pdb`,
`pkg/utils/ringbuffer`, `pkg/utils/atomic`, `pkg/utils/env` (the controller-
runtime client paths stay scope-cut; pure helpers are TDD-able).

**Wave 4 controllers (large, partially open):** provisioning sim still lacks
preferred anti-affinity / PV zone constraints / pod priority preemption;
disruption lacks the multi-node consolidation command + `consolidateAfter` TTL
gate; cluster state lacks pod→node occupancy accounting and daemonset-request
tracking.

**Merge gate:** LOC honest_ratio ≥ 0.95 before any merge to main. At 0.1750 this
branch is **held, not merged** — honest in-progress state, reviewed together
with rays #1–#3.
