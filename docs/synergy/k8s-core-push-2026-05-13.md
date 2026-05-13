# K8s core push batch 1 — 2026-05-13

**Status:** 3-of-3 unmapped packages closed. cave-controller-manager
0.7333 → **0.7556**, cave-kubelet 0.7632 → **0.8158**.
**Predecessor:** `k8s-core-5-crate-measured-audit-2026-05-12.md`
laid out the unmapped backlog — this sweep takes the 3 biggest items.

## What landed

### A. cave-controller-manager: ResourceClaim controller (DRA, KEP-4381)

`crates/cave-controller-manager/src/resourceclaim.rs` (+ 23 tests).

Closes the `pkg/controller/resourceclaim/` unmapped package — the
biggest gap in the 2026-05-12 audit (cited as a "blocker for DRA
GA"). Implementation is a pure-function state-machine
reconciler in the same shape as `namespace_controller.rs`:

* `evaluate(claim, pods, candidate) → ClaimAction` — single-step
  decision per pass, mirroring upstream's one-write-per-reconcile
  convention.
* `ClaimAction` covers every transition: `AddFinalizer`,
  `AwaitImmediateCandidate`, `AwaitFirstConsumer`, `SetAllocation`,
  `AddReservation`/`RemoveReservation`, deletion drain
  (`AwaitConsumerDrain` → `RequestDeallocation` → `RemoveFinalizer`
  → `AwaitDeletion`).
* `AllocationMode::Immediate` vs `WaitForFirstConsumer` — the
  Immediate path uses any scheduler candidate; Wait requires both a
  scheduled pod AND a candidate targeting that pod's node.
* `apply_reservation_diff` helper for the diff write.
* `check_tenant` for cross-tenant isolation (controllers honor the
  tenant gate per cave's Charter rule).
* `reconcile_outcome` mapper into `crate::types::Reconcile` so the
  manager loop (`runtime.rs`) can drive it like any other
  controller.

Device-fitness selection remains in `cave-scheduler/src/dra.rs`
(the `AllocationCandidate` triple is fed into this reconciler as
input — controller decides *what to write*, scheduler decides
*which device*).

### B.1. cave-kubelet: PodStatusManager

`crates/cave-kubelet/src/pod_status_manager.rs` (+ 15 tests).

Closes `pkg/kubelet/status/`. The kubelet sync loop now has a real
status reconciliation queue instead of opportunistic writes:

* **Lazy hash-dedupe** — `PodStatus::content_hash()` excludes
  free-text `message`, so per-tick message edits don't force an
  apiserver round-trip.
* **Bounded queue** — `StatusManagerConfig::max_queued` (default
  1024) with oldest-eviction.
* **Exponential backoff retry** via
  `cave_kernel::backoff::Backoff::Exponential` (200ms → 30s default).
  Transient vs permanent failure separation: `PermanentFailure`
  drops the pending entry *without* poisoning the confirmed hash,
  so a future identical status still enqueues.
* **Deleted-pod drop semantics** — `delete_pod` clears the pending
  entry AND blocks future `set_status` calls for the same uid
  (racing GC thread can't resurrect a removed pod).
* `needs_update` short-circuit so the caller can skip building a
  status when nothing changed.
* `pop_ready` is deterministic across ties (lex-smallest uid wins)
  so tests don't flake.
* `from_probe_outcomes` helper that rolls up per-container probe
  results into a `PodStatus` ready for the queue.

### B.2. cave-kubelet: Prober worker pool + ledger

`crates/cave-kubelet/src/prober.rs` (+ 17 tests).

Closes `pkg/kubelet/prober/`. Sits on top of the existing
per-probe state machine in `probe.rs`:

* **Worker pool** — `cave_kernel::semaphore::Semaphore` (default
  16 concurrent probes, matching upstream). Both sync
  (`try_reserve`) and async (`reserve`) handles; permit RAII drop
  returns the slot.
* **Restart-coordination ledger** — when liveness fails for a
  container, fires `CoordinatorEvent::RestartContainer` exactly
  once. Further failures are suppressed until
  `mark_restart_completed` acknowledges the work, OR the
  configurable safety window (5min default) expires (defends
  against a stuck kubelet failing to ack).
* **Readiness flip dedup** — `MarkReady` / `MarkNotReady` only
  emit on true transitions; steady-state success/failure ticks
  produce no event.
* **Startup-complete dedup** — `StartupComplete` fires once;
  later success ticks suppress.
* `snapshot()` returns the per-container `LedgerSnapshot` list for
  the admin/kubelet status panel.
* `coordinate(container, action, now)` exposed as a pure entry
  point so the sync loop can drive the coordinator directly when
  it owns probe scheduling.

## Counts before/after

| Crate | Mapped | Skipped | Unmapped | fill_ratio |
|---|---:|---:|---:|---:|
| cave-controller-manager (before) | 23 | 10 | 12 | 0.7333 |
| cave-controller-manager (after) | **24** | 10 | **11** | **0.7556** |
| cave-kubelet (before) | 20 | 9 | 9 | 0.7632 |
| cave-kubelet (after) | **22** | 9 | **7** | **0.8158** |

These are the *measured* fill_ratios (`(mapped + skipped) / total`)
against enumerated `pkg/controller/*` + `pkg/kubelet/*` upstream
sub-packages. They are NOT self-reported.

## LOC + test counts

| File | LOC | Tests |
|---|---:|---:|
| `crates/cave-controller-manager/src/resourceclaim.rs` | ~700 | 23 |
| `crates/cave-kubelet/src/pod_status_manager.rs` | ~620 | 15 |
| `crates/cave-kubelet/src/prober.rs` | ~580 | 17 |
| **Total new code** | **~1900 LOC** | **55 tests** |

Plus parity-manifest + audit-doc updates (5 files).

cave-controller-manager: 744 → **767** total tests (all green).
cave-kubelet: 671 → **703** total tests (all green).

## Stub policy honored

Zero `unimplemented!()`, zero `todo!()`, zero
`#[ignore = "impl pending"]` introduced. Every code path in the
three new modules is exercised by a deterministic test.

## What's still unmapped (honest)

cave-controller-manager (11 unmapped, was 12):
- `tainteviction/`, `cidrallocator/`, `validatingadmissionpolicystatus/`,
  `storageversionmigrator/`, `storageversiongarbagecollector/`,
  `legacyserviceaccounttokencleaner/`, `endpoint/`,
  `replication/`, `volume/pvprotection/`, `volume/ephemeral/`,
  `storageversionmigrator/migrator/`.

cave-kubelet (7 unmapped, was 9):
- `cm/util/cgroups/`, `lifecycle/`, `preemption/`,
  `nodeshutdown/`, `userns/`, `runonce/`, `checkpoint/`.

## Follow-ups

* Wire `resourceclaim::evaluate` into the controller-manager
  runtime loop (`runtime.rs`) so the live cluster gains the
  reconciler as a real driver alongside Deployment / ReplicaSet /
  etc. (Pure pass-through — `ScaffoldReconciler` already handles
  the dispatch.)
* Wire `PodStatusManager::pop_ready` into the kubelet sync loop
  (today's sync loop writes status directly via the apiserver
  client; this manager is the queue layer the audit doc called
  for).
* Wire `ProberCoordinator` to replace the direct
  `ProberManager::record_sample` calls in `agent.rs`.

These are integration steps — the deterministic state machines
have landed and are tested; plumbing them into the live sync loop
is a separate sweep.
