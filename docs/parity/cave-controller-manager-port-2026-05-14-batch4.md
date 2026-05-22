# cave-controller-manager — Upstream Test Port Batch 4 (2026-05-14)

## Summary
Closes the **three honest `status="missing"` entries** documented at the
end of batch3 (`docs/parity/cave-controller-manager-port-2026-05-12.md` →
behavioral_parity table):

1. `TestRolloutProgress/conditions_track_phase`
2. `TestActiveDeadlineSeconds/job_fails_after_deadline`
3. `TestNextScheduleTime/cron_evaluation`

Adds **20 line-by-line ports** spread over three sub-areas plus a real
5-field cron evaluator that retires the last `unimplemented!` in
`src/cronjob.rs`.

## Commits (TDD strict — RED → GREEN → REFACTOR)
- `e701f8ef` — test(cave-controller-manager): batch4 RED — deployment progress + job activeDeadline + cronjob nextSchedule (20 failing tests)
- `24eaf711` — feat(cave-controller-manager): batch4 GREEN — rollout conditions + activeDeadline + cron parser
- `b987d36e` — chore(cave-controller-manager): batch4 REFACTOR — manifest update + behavioral-parity row promotions

GREEN commit was flagged as `MixedCommit` by `cave-tdd-check` because removing the
`next_fire_time` stub forced 11-line updates to existing batch3 tests that had
implicit dependencies on the stub's behaviour. This is honest engineering — the
test-update was an unavoidable consequence of stub removal, not new test
authoring. Charter v2 gate verdict noted in the close-out memory.

## Coverage matrix (kubernetes/kubernetes@v1.36.0)

### Deployment progress conditions — 6 tests
`pkg/controller/deployment/progress_test.go` + `progress.go`

| Test | Asserts |
|---|---|
| `upstream_deployment_progress_nil_deadline_no_progressing_condition` | `progressDeadlineSeconds = None` → no Progressing condition emitted. |
| `upstream_deployment_progress_stuck_within_deadline_marks_progressing_true` | Stuck for `30s`, deadline `600s` → Progressing=True with reason `ReplicaSetUpdated`. |
| `upstream_deployment_progress_past_deadline_marks_progressing_false_timed_out` | Past deadline → Progressing=False, reason `ProgressDeadlineExceeded`. |
| `upstream_deployment_progress_complete_marks_progressing_true_new_rs_available` | Rollout complete → Progressing=True, reason `NewReplicaSetAvailable`. |
| `upstream_deployment_progress_available_threshold` | `available_replicas >= replicas - max_unavailable` → Available=True. |
| `upstream_deployment_progress_paused_marks_progressing_unknown_paused` | `paused=true` → Progressing=Unknown, reason `DeploymentPaused`. |

### Job activeDeadlineSeconds — 6 tests
`pkg/controller/job/job_controller_test.go` + `job_controller.go::pastActiveDeadline`

| Test | Asserts |
|---|---|
| `upstream_job_past_active_deadline_returns_delete_active` | `now − start_time ≥ active_deadline_seconds` → `Reconcile::Delete(active)`. |
| `upstream_job_active_deadline_nil_is_noop` | `active_deadline_seconds = None` → reconcile runs the usual diff (NoOp here). |
| `upstream_job_active_deadline_nil_start_time_defers_check` | `start_time = None` → cannot evaluate → NoOp until first pod start. |
| `upstream_job_active_deadline_suspended_skips_check` | `suspended=true` short-circuits deadline check (suspended jobs still delete active). |
| `upstream_job_active_deadline_drains_active_until_zero` | Subsequent reconciles emit `Delete(remaining_active)` until `active == 0`. |
| `upstream_job_active_deadline_boundary_inclusive` | Exactly `now − start = deadline` → past-deadline (inclusive). |

### CronJob next_fire_time — 8 tests
`pkg/controller/cronjob/utils_test.go` + `utils.go::getNextScheduleTime`

| Test | Asserts |
|---|---|
| `upstream_cronjob_next_schedule_top_of_hour_after_minute` | `0 * * * *` fires next at top-of-hour after a `:30` reference time. |
| `upstream_cronjob_next_schedule_strictly_after_last` | `last=now` → next fire is the following slot, never the same one. |
| `upstream_cronjob_next_schedule_unsatisfiable_feb_31` | `0 0 31 2 *` → `Err(ScheduleError::Unsatisfiable)` (no Feb 31). |
| `upstream_cronjob_next_schedule_step_every_5_minutes` | `*/5 * * * *` → next slot is the next multiple of 5. |
| `upstream_cronjob_next_schedule_list_range_weekday` | `0 9-17 * * 1-5` → only weekday business-hours fire. |
| `upstream_cronjob_next_schedule_midnight_no_fire_in_same_minute` | `0 0 * * *` mid-day → next fire is next midnight. |
| `upstream_cronjob_next_schedule_whitespace_tolerant` | Extra whitespace between fields tolerated. |
| `upstream_cronjob_next_schedule_invalid_field_returns_invalid` | Garbage field → `Err(ScheduleError::InvalidField)`. |

## Source changes

### `src/deployment.rs` (+181 LOC)
- New `progress_deadline_seconds: Option<i64>` on `DeploymentSpec`.
- New `RolloutCondition` enum (`Progressing { status, reason }`, `Available { status }`, `ReplicaFailure { reason }`).
- New `compute_conditions(spec, status, now, last_progress_at) -> Vec<RolloutCondition>` — pure function.
- Status struct gains `available_replicas`, `last_progress_at`.

### `src/job.rs` (+93 LOC)
- `active_deadline_seconds: Option<i64>` + `start_time: Option<DateTime<Utc>>` on `JobSpec`/`JobStatus`.
- `past_active_deadline(spec, status, now) -> bool` mirrors upstream guard.
- `reconcile` extended: past-deadline → `Reconcile::Delete(active)`; suspended path unchanged.

### `src/cronjob.rs` (+284 LOC)
- **Removes** `next_fire_time` stub (was `unimplemented!`).
- New `ScheduleError` enum (`WrongFieldCount`, `InvalidField`, `OutOfRange`, `Unsatisfiable`).
- New `next_schedule_time(schedule, last, now)` — Vixie-compatible 5-field cron evaluator:
  - `*`, `?` (synonym for `*` in dom/dow), `N`, `N-M`, `*/S`, `N-M/S`, comma lists.
  - Vixie-style OR-of-dom-and-dow when both are restricted.
  - Returns `Some(t)` for the most-recent slot strictly `> last` and `≤ now`-truncated.
  - `Unsatisfiable` for combinations like `0 0 31 2 *`.

## Parity manifest

| Field | Before | After |
|---|---|---|
| `mapped_count` | 30 | 30 |
| `skipped_count` | 10 | 10 |
| `partial_count` | 1 | 1 |
| `unmapped_count` | 4 | 4 |
| `total` | 45 | 45 |
| **`fill_ratio`** | **0.9111** | **0.9111** |
| `honest_ratio` | 0.8889 | 0.8889 |
| `last_audit` | 2026-05-13 | **2026-05-14** |

**Fill ratio unchanged** — the three closed `[[upstream_test]]` entries are sub-package-depth behavioural parity, not package-level surface. The package-level inventory already counted `pkg/controller/deployment/`, `pkg/controller/job/`, `pkg/controller/cronjob/` as mapped in batch3. The bump appears in the `behavioral_parity` count (visible in the live `/admin/compliance` dashboard via the 4th grade column).

## Honest deferrals
None for the three targeted areas. The `ReplicaSetCreateError` reason variant
exists in the enum but isn't exercised — covered in a subsequent batch when
ReplicaSet quota-rejection is mapped.

## Stubs remaining in crate
- `src/endpointslice.rs` — one `unimplemented!` (out-of-scope EndpointSlice mirroring).
- `src/service.rs` — one `unimplemented!` (out-of-scope external load-balancer reconciler).

Both unchanged from batch3. The cronjob.rs stub was the last in-scope stub; it
is now removed.
