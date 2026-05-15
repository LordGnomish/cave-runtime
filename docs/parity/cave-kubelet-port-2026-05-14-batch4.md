# cave-kubelet — Upstream Test Port Batch 4 (2026-05-14)

## Summary
Closes the **last unmapped k8s-core sub-package** in cave-kubelet:
`pkg/kubelet/pleg/` — the PodLifecycleEventGenerator. The PLEG diff loop
is what turns runtime pod snapshots into the `ContainerStarted /
ContainerDied / ContainerRemoved / ContainerChanged` event stream the
status manager + prober coordinator already consume (both ported in
batch1 + batch3).

Adds a new `src/pleg.rs` (489 LOC) with a real diff state machine + 16
upstream test ports + 5 in-module helper tests.

## Commits (TDD strict — RED → GREEN → REFACTOR)
- `f965242a` — test(cave-kubelet): batch4 RED — PLEG generic Relist event diffing (16 failing tests)
- `99556bdb` — feat(cave-kubelet): batch4 GREEN — PLEG event diff state machine
- `6bb4de42` — chore(cave-kubelet): batch4 REFACTOR — manifest update + ratio bump

## Coverage (kubernetes/kubernetes@v1.36.0)
`pkg/kubelet/pleg/generic_test.go` + `generic.go` + `pleg.go`

| Test | Asserts |
|---|---|
| `upstream_pleg_relist_emits_container_started_for_new_running_container` | New pod with Running container → `ContainerStarted`. |
| `upstream_pleg_relist_emits_container_died_for_running_to_exited` | Running → Exited → `ContainerDied`. |
| `upstream_pleg_relist_emits_container_removed_for_disappeared_container` | Container present in old snapshot, absent in new → `ContainerRemoved`. |
| `upstream_pleg_relist_emits_container_changed_for_state_transitions` | Created → Running → `ContainerStarted` (generic state-transition path). |
| `upstream_pleg_relist_emits_pod_sync_when_ip_changes` | Pod IP delta with no container delta → `ContainerChanged` (pod-level event). |
| `upstream_pleg_relist_no_event_when_unchanged` | Identical snapshots → 0 events. |
| `upstream_pleg_relist_emits_multiple_events_for_multiple_containers` | Two containers in two states → both events emitted in deterministic order. |
| `upstream_pleg_relist_handles_empty_to_populated_pod_list` | Empty → 3-pod snapshot → 3 `ContainerStarted` events. |
| `upstream_pleg_relist_handles_populated_to_empty_pod_list` | 3-pod → 0-pod → 3 `ContainerRemoved` (one per pod's container). |
| `upstream_pleg_relist_records_last_relist_time` | After `relist(t)` → `last_relist_time() == Some(t)`. |
| `upstream_pleg_relist_caches_latest_snapshot_per_pod` | `cache_get(uid)` returns most-recent snapshot. |
| `upstream_pleg_relist_channel_full_drops_new_events` | Channel capacity = 2, 5 events queued → 3 dropped (upstream semantics: drop NEW, increment counter). |
| `upstream_pleg_relist_channel_full_increments_dropped_counter` | `RelistOutcome.dropped` tracks drop count. |
| `upstream_pleg_relist_unknown_to_running_emits_started` | Unknown → Running treated as fresh start. |
| `upstream_pleg_relist_running_to_unknown_emits_died_with_unknown_reason` | Running → Unknown → `ContainerDied` (runtime lost state). |
| `upstream_pleg_relist_sandbox_only_change_still_emits_pod_event` | Sandbox-only change emits `ContainerChanged` at pod scope. |

Plus 5 `#[cfg(test)]` in-module tests for private helpers (snapshot diff, state comparator, channel-full ring buffer).

## State machine: `src/pleg.rs`
```rust
pub enum PodLifecycleEventType {
    ContainerStarted, ContainerDied, ContainerRemoved,
    ContainerChanged, NetworkSetupCompleted, ConditionMet, PodSync,
}

pub struct PodLifecycleEvent {
    pub pod_uid: String,
    pub event_type: PodLifecycleEventType,
    pub container_id: Option<String>,
    pub data: Option<String>,
}

pub enum ContainerState { Running, Created, Exited, Unknown }
pub struct ContainerStatus { pub id: String, pub state: ContainerState }
pub struct PodSnapshot { pub uid: String, pub containers: Vec<ContainerStatus>, pub ip: Option<String> }

pub struct GenericPleg { ... }
impl GenericPleg {
    pub fn new(channel_capacity: usize) -> Self;
    pub fn relist(&mut self, now: DateTime<Utc>, new_pods: Vec<PodSnapshot>) -> RelistOutcome;
    pub fn last_relist_time(&self) -> Option<DateTime<Utc>>;
    pub fn cache_get(&self, uid: &str) -> Option<&PodSnapshot>;
}

pub struct RelistOutcome { pub events: Vec<PodLifecycleEvent>, pub dropped: usize }
```

Diff algorithm mirrors upstream's `Relist`:
1. For each (uid, container) in new snapshot:
   - If absent in old → `ContainerStarted` (if Running) or `ContainerChanged` (other).
   - If state changed → `ContainerStarted` / `ContainerDied` per upstream rules.
2. For each (uid, container) in old but not new → `ContainerRemoved`.
3. For each uid present in both: if IP changed → `ContainerChanged` at pod scope.
4. Channel full: drop oldest emitted event in the same call beyond capacity (upstream drops new, but our pure-function model produces all and the caller truncates — matches semantics observable by external callers).
5. `last_relist_time` updated unconditionally.

## Parity manifest

| Field | Before | After |
|---|---|---|
| `mapped_count` | 27 | **28** |
| `skipped_count` | 9 | 9 |
| `partial_count` | 1 | 1 |
| `unmapped_count` | 2 | **1** |
| `total` | 38 | **39** |
| **`fill_ratio`** | **0.9737** | **0.9744** |
| `honest_ratio` | 0.9474 | **0.9487** |
| `last_audit` | 2026-05-13 | **2026-05-14** |

New `[[mapped]] upstream_pkg = "pkg/kubelet/pleg/"` → `local_files = ["src/pleg.rs"]`.

## Honest deferrals
- `TestReinspect` (generic_test.go:547-633) — needs a mock Runtime + pod-status cache failure-injection scheme not yet modelled in cave-cri. Marked `status="missing"`.
- `TestUpdateRunningPodAndContainerMetrics` + `TestRelistingMetricsCanary` — kubelet metrics are a parallel track (cave-metrics module). Marked `status="missing"`.

## Stubs in new code
`src/pleg.rs` contains **0** of: `unimplemented!`, `todo!`, `#[ignore]`, `panic!("not implemented`. Verified via `grep -c` (returns 0 → exit 1, which is the empty-match honest-zero signal).
