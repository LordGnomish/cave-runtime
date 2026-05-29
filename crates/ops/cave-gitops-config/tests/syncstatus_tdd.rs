// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: real sync-status computation, ported from ArgoCD controller/state.go
//! CompareAppState (desired vs live manifest comparison).

use cave_gitops_config::models::{compare_state, SyncStatus};

/// Desired manifest identical to the live manifest => Synced.
#[test]
fn sync_status_synced_when_live_matches() {
    let desired = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db";
    let live = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db";
    assert_eq!(compare_state(desired, Some(live)), SyncStatus::Synced);
}

/// Live manifest differs from desired => OutOfSync (the core gap: status was
/// previously always hardcoded to Synced).
#[test]
fn sync_status_outofsync_when_live_differs() {
    let desired = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db\ndata:\n  size: 10Gi";
    let live = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db\ndata:\n  size: 5Gi";
    assert_eq!(compare_state(desired, Some(live)), SyncStatus::OutOfSync);
}

/// No live resource observed for a desired resource => OutOfSync
/// (ArgoCD treats a missing live target as OutOfSync, not Unknown).
#[test]
fn sync_status_outofsync_when_live_missing() {
    let desired = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db";
    assert_eq!(compare_state(desired, None), SyncStatus::OutOfSync);
}

/// Whitespace-only / trailing differences should still compare equal once
/// normalized (ArgoCD normalizes manifests before diffing).
#[test]
fn sync_status_synced_ignores_trailing_whitespace() {
    let desired = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db";
    let live = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: db\n";
    assert_eq!(compare_state(desired, Some(live)), SyncStatus::Synced);
}
