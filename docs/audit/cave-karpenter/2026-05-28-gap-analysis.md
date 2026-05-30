<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-karpenter — Gap Analysis (2026-05-28)

Upstream: **kubernetes-sigs/karpenter v1.12.1** (sha `ed490e8`), core repo
only (see ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001 — AWS provider out of
scope).

`honest_ratio` is measured as **cave non-test LOC / upstream-core non-test
LOC**. Upstream-core non-test LOC = **34,772** across 202 Go files.

## Upstream package × cave coverage matrix

| Upstream package | core LOC | cave module | status |
| ---------------- | -------: | ----------- | ------ |
| `pkg/apis/v1` (CRD model, validation, defaults, budgets) | 2,483 | `src/models/` | **partial** — type shapes ported; CRD validation, defaults, budget math NOT ported |
| `pkg/scheduling` (requirement set-algebra, taints, hostport, volume) | 1,300 | — | **MISSING** — this ray's first target |
| `pkg/cloudprovider` (interface, types, fake) | 1,544 | `src/provider/` | **partial** — trait surface ported; metrics/overlay decorators not |
| `pkg/controllers/provisioning` | 4,539 | `src/scheduler.rs`, `src/batcher.rs`, `src/binpack.rs` | **partial** — first-match + batcher + binpack; full scheduler sim missing |
| `pkg/controllers/disruption` | 3,092 | `src/disruption.rs` | **partial** — consolidation/drift/expiration decisions; full methods + budgets missing |
| `pkg/controllers/nodeclaim` | 2,202 | `src/nodeclaim_lifecycle.rs` | **partial** — launch/drain/terminate; garbage-collection, liveness, registration missing |
| `pkg/controllers/state` + `pkg/state` | 3,017 | `src/store.rs` | **partial** — in-mem store; cluster-state graph + antiaffinity missing |
| `pkg/controllers/node` (termination, drain) | 1,567 | `src/drain.rs` | **partial** — drain ported; termination controller wiring missing |
| `pkg/controllers/nodepool` | 555 | — | **MISSING** — counter/hash/readiness reconcilers |
| `pkg/controllers/nodeoverlay` | 591 | — | **MISSING** |
| `pkg/controllers/static` | 492 | — | **MISSING** |
| `pkg/controllers/metrics` | 922 | — | **MISSING** |
| `pkg/utils` | 2,059 | — | **MISSING** — resource math, pretty-print, node helpers |
| `pkg/operator` | 746 | — | out-of-scope (runtime bootstrap owned by cave-runtime) |
| `pkg/events` | 147 | — | **MISSING** — event recorder shims |
| `pkg/metrics` | 273 | — | **MISSING** |

Baseline cave non-test src LOC: **1,887** → honest_ratio ≈ **0.0543**.

## Missing-feature priority list (this ray ports top-down by ROI)

1. **`pkg/scheduling` set-algebra** (1,300 LOC) — self-contained,
   deterministic, rich test corpus. Highest TDD ROI. *(in progress)*
   - `Requirement`: complement-based set with inclusive int bounds;
     `Intersection`, `HasIntersection`, `Has`, `Operator`, `Len`,
     `withinBounds`, `min/maxIntPtr`.
   - `Requirements`: keyed collection; `Add` (intersect-on-collide),
     `Get` (undefined → Exists), `Compatible`, `Intersects`, `IsCompatible`.
   - `Taints.Tolerates`, `HostPortUsage`, `VolumeUsage`/`VolumeCount`.
2. **`pkg/apis/v1` validation + defaults + budgets** — NodePool budget
   `AllowedDisruptions`, defaults, CEL-equivalent validation.
3. **`pkg/utils` resource math** — `resources.Merge/Subtract/Fits`.
4. **`pkg/controllers/nodepool`** counter + hash + readiness reconcilers.
5. **Full disruption methods** — consolidation candidates, command
   construction, drift reason enumeration.
6. **Cluster state graph** — node/nodeclaim/pod tracking, antiaffinity.

## Method

Strict TDD per component: failing test commit → verify FAIL → impl commit
→ verify PASS. No combined test+impl commits. Checkpoint-push every few
cycles. honest_ratio recomputed from LOC at the end.

## Progress — 2026-05-30 ray (4 strict-TDD cycles)

| Cycle | Module | Upstream file | cave LOC | Tests |
| ----- | ------ | ------------- | -------: | ----: |
| 1 | `scheduling::requirement` | `pkg/scheduling/requirement.go` | 384 | 12 |
| 2 | `scheduling::requirements` | `pkg/scheduling/requirements.go` | 268 | 10 |
| 3 | `scheduling::hostport` | `pkg/scheduling/hostportusage.go` | 155 | 8 |
| 4 | `resources` (Quantity + Ceiling) | `pkg/utils/resources.go` + k8s PodRequests | 274 | 10 |

- cave non-test src LOC: **1,887 → 2,970** (+1,083; new scheduling + resources).
- LOC honest_ratio (cave src / upstream-core 34,772): **0.0543 → 0.0854**.
- Crate test count: **114 → 154** (+40), all green, zero regressions.
- Every cycle: separate `test(...)` RED commit then `feat(...)` GREEN commit.

**Note on the repo parity metric.** This crate's `parity.manifest.toml` /
`parity-index.json` use a *count-based* `honest_ratio` (mapped/total = 19/22
= 0.8636) guarded by a self-consistent 9-assertion gate, and the index is
regenerated from manifests by a post-commit hook. The LOC honest_ratio
defined for this ray is a **different metric**; it is recorded here and in
the index `cave_src_loc` field rather than overwriting the count-based ratio
(which would break the gate and be reverted by the hook). At 0.0854 LOC the
ray is far below the 0.95 merge gate, so the branch is pushed but **not
merged** — honest in-progress state, not an early bail.

## Remaining work to LOC 1.00 (≈ 31.8K Rust LOC)

Priority order for continuation rays:
1. `pkg/scheduling`: `volumeusage.go` (226), `taints.go` (81; needs k8s
   toleration-matching port — no local corpus).
2. `pkg/apis/v1`: NodePool budget `AllowedDisruptions`, defaults, CEL-equiv
   validation (~2,000 LOC, has corpus).
3. `pkg/utils`: remaining helpers (~1,900 LOC).
4. `pkg/controllers/nodepool` (555), then disruption methods (3,092),
   provisioning scheduler sim (4,539), cluster state graph (3,017).
