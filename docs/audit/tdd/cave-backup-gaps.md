# TDD coverage audit — cave-backup vs Velero

- **Cave crate:** `crates/data/cave-backup` (theme: data)
- **Upstream:** vmware-tanzu/velero @ `main`
- **Upstream test inventory:** 276 test files / 1084 test symbols (`/tmp/tdd-audit/cave-backup-upstream-tests.txt`)
- **Cave test functions:** 89 `#[test]` fns (10 src inline modules + 4 dedicated `tests/*_tdd.rs` files)
- **Date:** 2026-05-30

## Summary

cave-backup is a **focused port** of Velero's *pure, in-memory* backup/restore logic
(include-exclude filtering, restore resource ordering, cron scheduling, GC/TTL, hook
validation, BSL validation, FS-backup job lifecycle). The vast majority of Velero's
1084 test symbols are **scope-cut**: controller reconcilers, CRD codegen, CSI/CBT
plugin RPC, `pkg/cmd/cli/*` cobra commands, kube client factories, archive tar
extraction, and describe/printer output formatting — none of which cave ports.

The crate already has **substantial, faithful coverage**: `includes_excludes_tdd.rs`
mirrors Velero's `TestShouldInclude`/`TestIncludeEverything`/`TestValidateIncludesExcludes`;
`restore_order_tdd.rs` mirrors `TestStringOfPriorities`/`TestParse`/`getOrderedResources`;
`engine.rs`, `gc.rs`, `hooks.rs`, `storage.rs`, `volume.rs`, `filesystem.rs`,
`schedule.rs` all carry inline `#[cfg(test)]` modules.

The genuine uncovered **portable-coverage** gaps are concentrated in two places:
1. `src/types.rs` has **no test module at all** — its phase/TTL predicates are untested.
2. `src/schedule.rs::next_run` cron engine implements step / range / DOW-OR semantics
   that the existing 2-case `schedule_tdd.rs` never exercises.

## Classification table

| Upstream behavioral unit (Velero) | Cave fn / location | Class | Notes |
|---|---|---|---|
| `TestShouldInclude` (includes_excludes_test.go) | `IncludesExcludes::should_include` | covered | `includes_excludes_tdd.rs` |
| `TestIncludeEverything` | `IncludesExcludes::include_everything` | covered | `includes_excludes_tdd.rs` |
| `TestValidateIncludesExcludes` | `validate_includes_excludes` | covered | `includes_excludes_tdd.rs` |
| glob `*.bar` semantics (gobwas/glob) | `includes_excludes::glob_match` | covered | inline + tdd |
| `TestStringOfPriorities` / `Priorities.String` | `Priorities::to_priority_string` | covered | `restore_order_tdd.rs` |
| `Priorities.Set` parse (priority_test.go) | `Priorities::parse` | covered | `restore_order_tdd.rs` (8 cases) |
| `getOrderedResources` (restore.go) | `get_ordered_resources` | covered | `restore_order_tdd.rs` |
| `getNextRunTime` happy path (schedule_controller) | `schedule::next_run` | covered | 2 cases only (hourly, daily) |
| **`TestParseCronSchedule` step `*/N` fields (robfig/cron)** | `schedule::next_run` (`parse_field` `*/step`) | **portable-coverage** | step/interval matching untested |
| **`TestParseCronSchedule` range `lo-hi` fields** | `schedule::next_run` (`parse_field` `lo-hi`) | **portable-coverage** | range matching untested |
| **cron DOM/DOW OR semantics (robfig spec)** | `schedule::next_run` (`CronSchedule::matches`) | **portable-coverage** | `(true,true)=>dom OR dow` branch untested |
| `validate_cron` field-count | `schedule::validate_cron` | covered | inline |
| `describe_cron` / `due_schedules` | `schedule::describe_cron`, `due_schedules` | covered | inline |
| `Test_markInProgressBackupsFailed` terminal-phase gate | `BackupPhase::is_terminal` | **portable-coverage** | types.rs has **no** test module |
| `Test_markInProgressRestoresFailed` terminal-phase gate | `RestorePhase::is_terminal` | **portable-coverage** | untested |
| Backup TTL reaping (gc_controller TTL via age) | `Backup::is_expired` (created_at + ttl_hours) | **portable-coverage** | untested; distinct from `check_expiration` |
| `DownloadRequest` TTL expiry (downloadrequest controller) | `DownloadRequest::is_expired` | **portable-coverage** | untested |
| `check_expiration` (expires_at) | `engine::check_expiration` | covered | inline (3 cases) |
| `find_expired` / `mark_deleting` / `gc_stats` | `gc::*` | covered | inline |
| `TestBackupProgressIsUpdated` (subset) | `engine::complete_backup` | covered | inline (Completed + PartiallyFailed) |
| restore creation defaults | `engine::create_restore` | covered | inline |
| hook validation (item_hook_handler validate) | `hooks::validate_hooks`, `default_exec_hook` | covered | inline |
| `run_exec_hooks` log emission | `engine::run_exec_hooks` | covered | inline |
| `TestBackupStorageLocationValidate` | `storage::validate_bsl`, `mark_available` | covered | inline |
| volume snapshot readiness / naming | `volume::*` | covered | inline |
| FS-backup job lifecycle (podvolume) | `filesystem::*` | covered | inline |
| `RetentionPolicy` max/ttl constructors | `RetentionPolicy::new_max/new_ttl` | scope-cut | trivial constructors, no upstream behavioral test |
| controller reconcilers / `pkg/controller/*` | — | scope-cut | no reconcile loop in cave |
| CRD codegen / `pkg/apis/*` deepcopy | — | scope-cut | no CRDs |
| CSI / CBT / VGS plugin RPC (`pkg/backup/actions/csi/*`) | — | scope-cut | plugin RPC not ported |
| `pkg/cmd/cli/*` cobra commands | `routes.rs` (HTTP, separate) | scope-cut | CLI not ported; routes are axum |
| `pkg/archive/*` tar extract, path-traversal | — | scope-cut | no tarball archive layer in cave |
| `pkg/cmd/util/output/*` describe/printer | — | scope-cut | no describe formatters |
| `internal/resourcemodifiers/*` JSON-merge / strategic-merge patch | — | scope-cut | resource modifiers not ported |
| `internal/resourcepolicies/*` PVC/PV volume policy | — | scope-cut | volume policy engine not ported |
| kube client / factory / config | — | scope-cut | no kube client |

## Recommended TDD fills (portable-coverage first)

Each row names the **exact public cave fn** the new test would exercise.

1. **`schedule::next_run` — step field (`*/15 * * * *`)**
   Implemented in `parse_field`'s `*/step` branch + `CronSchedule::matches`, but the
   only schedule tests use `0 0 * * *` and `0 * * * *`. Assert that from a known
   `after` the next fire lands on the next multiple-of-15 minute. Exercises:
   `cave_backup::schedule::next_run`.

2. **`schedule::next_run` — range field (`0 9-17 * * *`)**
   Exercises `parse_field`'s `lo-hi` branch (currently unreachable by any test). From
   an `after` at 08:30 the next run is 09:00; from 17:30 it rolls to next day 09:00.
   Exercises: `cave_backup::schedule::next_run`.

3. **`schedule::next_run` — DOM/DOW OR semantics (`0 0 1 * 0`)**
   The `(dom_restricted, dow_restricted) => dom_match || dow_match` branch in
   `CronSchedule::matches` is never hit. Assert the schedule fires on *either* the
   1st of the month *or* any Sunday. Exercises: `cave_backup::schedule::next_run`.

4. **`types::BackupPhase::is_terminal`**
   No test module exists in `types.rs`. Mirror Velero's terminal-phase gate
   (`Test_markInProgressBackupsFailed`): assert `Completed`, `PartiallyFailed`,
   `Failed`, `FailedValidation` are terminal and `New`, `InProgress`, `Deleting`
   are not. Exercises: `cave_backup::types::BackupPhase::is_terminal`.

5. **`types::RestorePhase::is_terminal`**
   Companion to (4), mirroring `Test_markInProgressRestoresFailed`: assert the four
   terminal phases are terminal and `New`/`InProgress` are not. Exercises:
   `cave_backup::types::RestorePhase::is_terminal`.

6. **`types::Backup::is_expired`**
   TTL-by-age predicate (distinct from `engine::check_expiration`, which keys off a
   precomputed `expires_at`). Assert: ttl_hours==0 → never expired; a `Backup` whose
   `created_at` is older than `ttl_hours` → expired; recent `created_at` → not.
   Exercises: `cave_backup::types::Backup::is_expired`.

7. **`types::DownloadRequest::is_expired`**
   Mirrors Velero's download-request TTL reaping. Construct via
   `DownloadRequest::new(id, ttl_seconds)`, then force `expires_at` into the past →
   expired; future → not. Exercises: `cave_backup::types::DownloadRequest::is_expired`
   (and `DownloadRequest::new`).
