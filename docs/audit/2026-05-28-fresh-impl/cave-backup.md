# cave-backup — Coverage Audit

- **Crate:** cave-backup (`crates/data/cave-backup`)
- **Upstream:** vmware-tanzu/velero
- **Upstream tag:** v1.18.1
- **Upstream commit SHA:** 26ef8fa7df68dfa9ab8aa4669b8ac47340b0c510
- **Upstream license:** Apache-2.0 (line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29

## Summary

cave-backup is a small (~2700 LOC) in-memory CRUD service exposing an axum REST API for
Velero-shaped objects (Backup, Restore, Schedule, BackupStorageLocation,
VolumeSnapshotLocation, FsBackupJob). It models the Velero *data shapes* and *phase enums*
faithfully, plus a few pure helpers (cron field-count validation, TTL expiry, GC marking,
hook validation, BSL validation, snapshot-name formatting). It does **not** implement the
actual backup engine: no Kubernetes resource discovery/collection, no object-store upload,
no restic/kopia uploading, no CSI snapshot orchestration, no controllers/reconcilers, no
restore item-application, no data-mover, and no real cron scheduling/next-run computation.
Most of the entries below are therefore PARTIAL (shape exists, behavior absent) or MISSING.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `pkg/backup/backup.go` | Backup engine: orchestrate item collection, hooks, snapshots, write tarball | `engine.rs::create_backup`/`complete_backup` | PARTIAL | Only sets phase/timestamps/counters; no item walking, no tarball, no snapshot orchestration. `complete_backup` takes caller-supplied counts. |
| `pkg/backup/item_collector.go` | Discover & collect K8s resources matching includes/excludes/label-selector | — | MISSING | Filter fields stored on `BackupSpec` but never evaluated against any resource set. |
| `pkg/backup/item_backupper.go` | Per-item backup: additional items, PV handling, restore-actions, exec hooks | — | MISSING | No item-level logic at all. |
| `pkg/backup/itemblock.go` + `pkg/backup/item_block_worker_pool.go` | ItemBlock grouping & parallel worker pool | — | MISSING | No concurrency/grouping. |
| `pkg/backup/snapshots.go` + `volume_snapshotter_cache.go` | Volume snapshotter invocation & caching during backup | `volume.rs` | PARTIAL | Pure helpers (`all_snapshots_ready`, `pending_snapshots`, `snapshot_name`) only; no snapshotter calls. |
| `pkg/backup/pv_skip_tracker.go` | Track PVs skipped from snapshotting with reasons | — | MISSING | No PV skip tracking. |
| `pkg/restore/restore.go` | Restore engine: prioritize, remap, apply items to cluster, wait readiness | `engine.rs::create_restore` | PARTIAL | Builds a `Restore` record with remap fields; never applies anything to a cluster. |
| `pkg/restore/prioritize_group_version.go` | Restore resource priority ordering & GV prioritization | — | MISSING | No ordering logic. |
| `pkg/restore/pv_restorer.go` | Restore PVs / dynamic reprovision decisions | — | MISSING | `restore_pvs` bool stored, never acted on. |
| `pkg/restore/merge_service_account.go` | Merge SA secrets/imagePullSecrets on existing-resource restore | — | MISSING | Not implemented. |
| `internal/restore/*` (change_*_action, pvc_from_pod, apiservice, admissionwebhook) | Built-in restore item actions / mutations | — | MISSING | No restore item-action framework. |
| `pkg/controller/schedule_controller.go` | Cron scheduling: compute next-run, fire backups, due detection | `schedule.rs` | PARTIAL | `validate_cron` only counts 5 fields; `describe_cron` hardcodes 4 strings; `due_schedules` just filters `!paused` — no time/cron evaluation, no next-run. |
| `pkg/controller/gc_controller.go` + `backup_deletion_controller.go` | TTL GC reconcile + cascading delete of snapshots/objects/CRs | `gc.rs` | PARTIAL | `find_expired`/`mark_deleting`/`gc_stats` compute in-memory; no object-store/snapshot deletion, no DeleteBackupRequest. |
| `pkg/controller/backup_controller.go` | Backup reconcile loop (validation, queue, run, finalize) | `routes.rs::create_backup` | PARTIAL | One synchronous insert into a HashMap; no reconcile, no validation gating, no requeue. |
| `pkg/controller/backup_storage_location_controller.go` | BSL validation/availability reconcile, periodic re-validate | `storage.rs` + `routes.rs` | PARTIAL | `validate_bsl` checks name/bucket non-empty; `mark_available` flips phase. No connectivity check, no periodic loop. |
| `pkg/controller/backup_sync_controller.go` | Sync backups FROM object store into cluster (DR import) | — | MISSING | No store-to-cluster sync. |
| `pkg/controller/backup_finalizer_controller.go` + `restore_finalizer_controller.go` | Finalize async item-operations then complete | — | MISSING | No finalizer phase. |
| `pkg/controller/download_request_controller.go` | Generate signed download URL for backup artifacts/logs | `types.rs::DownloadRequest` | PARTIAL | Struct + TTL expiry helper exist but unused; no route, no URL signing. |
| `pkg/controller/backup_repository_controller.go` + `pkg/repository/*` | Backup repository (restic/kopia) ensure/lock/maintain/keys | `models.rs::FsBackupMethod` | MISSING | Method enum only; no repo init, lock, maintenance, password keys. |
| `pkg/controller/server_status_request_controller.go` | ServerStatusRequest: report plugins & version | `routes.rs::server_status` | PARTIAL | Returns static plugin list & version string; no SSR CR, no plugin discovery. |
| `pkg/controller/backup_tracker.go` | Track in-flight backups to prevent concurrent deletion | — | MISSING | No tracker. |
| `pkg/podvolume/backupper.go` + `restorer.go` | Pod-volume (restic/kopia) backup/restore over pod volumes | `filesystem.rs` | PARTIAL | `create_fs_backup_job`/`complete_fs_backup` are status setters; no uploader, no pod/volume mount, no snapshot. |
| `pkg/uploader/kopia/*` + `pkg/uploader/provider/*` | Kopia/restic uploader: snapshot, dedup, progress | — | MISSING | No uploader. `method_description` returns a string. |
| `pkg/repository/maintenance/*` | Repo maintenance (prune/check) jobs | — | MISSING | Not implemented. |
| `pkg/persistence/object_store.go` + `object_store_layout.go` | Object-store backend: put/get/list/delete with bucket layout | `models.rs::BackupTarget`/`StorageProvider` | MISSING | Target/provider enums only; no S3/GCS/Azure client, no layout, no IO. |
| `pkg/datamover/*` + `pkg/datapath/*` | Data-mover micro-services & datapath (CSI snapshot data movement) | `models.rs::snapshot_move_data` | MISSING | A bool field; no data-mover, no async data path. |
| `pkg/exposer/csi_snapshot.go` + `pod_volume.go` | Expose CSI snapshots / pod volumes via backup pods | — | MISSING | No CSI VolumeSnapshot orchestration. |
| `pkg/itemoperation/*` + `pkg/controller/*_operations_controller.go` | Async item-operation tracking (long-running plugin ops) | — | MISSING | No async operation model. |
| `pkg/archive/parser.go` + `extractor.go` | Backup tarball layout parse/extract (resources/, v2 layout) | — | MISSING | No tar read/write or layout parsing. |
| `pkg/discovery/helper.go` | API discovery: GVR resolution, preferred versions | — | MISSING | No discovery client. |
| `pkg/metrics/metrics.go` | Prometheus metrics (backup duration, sizes, counts, failures) | — | MISSING | No metrics emission. |
| internal hooks (`internal/hook/*`, exec/init handlers) | Resolve & execute pre/post exec hooks, init-container restore hooks | `hooks.rs` + `engine.rs::run_exec_hooks` | PARTIAL | `validate_hooks` checks empty commands; `run_exec_hooks` only formats log strings — no pod exec. |
| Backup/Restore/Schedule/BSL/VSL API types (`pkg/apis/velero/v1`) | CRD-equivalent data shapes & phase enums | `models.rs` + `types.rs` | COVERED | Phase enums, BSL, VSL, Schedule, Restore, FsBackupJob shapes modeled and round-trip via serde. |
| REST surface (CLI/UI equivalent) | CRUD over backups/restores/schedules/locations/fs-backup | `routes.rs` | COVERED | Real axum router with create/list/get/delete/pause/unpause, 404 handling, cron 422; in-memory only. |

## Actionable gaps for strict-TDD

Ordered lowest-effort-highest-value first. Each test should be written RED first against the
named cave module, then made to pass by porting the referenced upstream logic.

1. **Cron next-run computation (`schedule.rs`)**
   Upstream ref: `pkg/controller/schedule_controller.go` (`getNextRunTime`, uses `robfig/cron`).
   Test: `schedule_next_run_after_known_time` — given cron `"0 0 * * *"` and `last_run = 2026-05-29T10:00:00Z`, assert `next_run` == `2026-05-30T00:00:00Z`. Currently no next-run is computed at all.

2. **Cron expression validation rigor (`schedule.rs::validate_cron`)**
   Upstream ref: `pkg/controller/schedule_controller.go` validation via cron parser.
   Test: `validate_cron_rejects_out_of_range_fields` — assert `validate_cron("99 0 * * *")` is `false` (minute 99 invalid) and `validate_cron("@daily")` is `true`. Current impl only counts 5 whitespace fields, so `"99 0 * * *"` wrongly passes and macros wrongly fail.

3. **Schedule retention / max-backups enforcement (`gc.rs` or new `retention.rs`)**
   Upstream ref: `pkg/controller/gc_controller.go` + Schedule retention semantics.
   Test: `retention_keeps_only_max_backups` — given 5 completed backups for a schedule with `RetentionPolicy::new_max(3)`, assert the 2 oldest are returned for deletion and the 3 newest kept. No retention logic exists today (`RetentionPolicy` is an unused struct).

4. **Resource filter evaluation (`engine.rs` or new `filter.rs`)**
   Upstream ref: `pkg/backup/item_collector.go` includes/excludes/label-selector matching.
   Test: `included_excluded_namespaces_filter_resources` — given a set of fake `{namespace, kind}` items and a `BackupSpec` with `included_namespaces=["app"]`, `excluded_resources=["secrets"]`, assert only `app`-namespace non-secret items are selected. Currently filters are stored but never applied.

5. **Object-store backup layout keys (`storage.rs` or new `layout.rs`)**
   Upstream ref: `pkg/persistence/object_store_layout.go` (`getBackupContentsKey`, `metadata/`, `<backup>/` prefixes).
   Test: `object_store_layout_keys_for_backup` — assert key for backup `"db-2026"` is `"backups/db-2026/db-2026.tar.gz"` and metadata key is `"backups/db-2026/velero-backup.json"`. No layout function exists.

6. **Restore resource priority ordering (new `restore.rs`)**
   Upstream ref: `pkg/restore/prioritize_group_version.go` + default restore priorities (namespaces, CRDs, PVs, PVCs, secrets, SAs first).
   Test: `restore_orders_high_priority_resources_first` — given unordered `["pods","customresourcedefinitions","namespaces","secrets"]`, assert ordering puts `namespaces` and `customresourcedefinitions` before `pods`. No prioritization exists.
