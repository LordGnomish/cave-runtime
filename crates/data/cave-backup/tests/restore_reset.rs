// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mirror tests for the restore metadata/status reset logic ported from Velero
//! `pkg/restore/restore.go` `resetMetadata` / `resetStatus` /
//! `resetMetadataAndStatus` (Apache-2.0). Behaviour and error strings verbatim
//! from upstream `pkg/restore/restore_test.go` (`TestResetMetadata`,
//! `TestResetStatus`).

use cave_backup::restore_reset::{reset_metadata, reset_metadata_and_status, reset_status};
use serde_json::json;

// ── resetMetadata ────────────────────────────────────────────────────────────

#[test]
fn reset_metadata_keeps_whitelisted_unlisted_keys() {
    // Upstream blacklist deletes 9 server/identity keys; everything else
    // (name, namespace, labels, annotations, managedFields, finalizers) stays.
    let mut obj = json!({
        "metadata": {
            "name": "pv-1",
            "namespace": "ns-1",
            "labels": {"app": "x"},
            "annotations": {"k": "v"},
            "managedFields": [{"manager": "kubectl"}],
            "finalizers": ["kubernetes.io/pv-protection"],
            "uid": "abc",
            "resourceVersion": "1",
            "generation": 2,
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "selfLink": "/api/v1/persistentvolumes/pv-1",
            "ownerReferences": [{"name": "owner"}],
            "generateName": "pv-",
            "deletionTimestamp": "2026-02-01T00:00:00Z",
            "deletionGracePeriodSeconds": 30
        }
    });
    reset_metadata(&mut obj).unwrap();
    let md = obj["metadata"].as_object().unwrap();
    // kept
    for k in ["name", "namespace", "labels", "annotations", "managedFields", "finalizers"] {
        assert!(md.contains_key(k), "expected {k} to survive, got {md:?}");
    }
    // removed
    for k in [
        "generateName",
        "selfLink",
        "uid",
        "resourceVersion",
        "generation",
        "creationTimestamp",
        "deletionTimestamp",
        "deletionGracePeriodSeconds",
        "ownerReferences",
    ] {
        assert!(!md.contains_key(k), "expected {k} to be removed, got {md:?}");
    }
}

#[test]
fn reset_metadata_remove_uid_owner_refs_keeps_name_namespace() {
    // Verbatim upstream case: input {name, namespace, uid, ownerReferences} ->
    // {name, namespace}.
    let mut obj = json!({
        "metadata": {"name": "n", "namespace": "ns", "uid": "u", "ownerReferences": [{"name": "o"}]}
    });
    reset_metadata(&mut obj).unwrap();
    let md = obj["metadata"].as_object().unwrap();
    assert_eq!(md.len(), 2, "got {md:?}");
    assert!(md.contains_key("name"));
    assert!(md.contains_key("namespace"));
}

#[test]
fn reset_metadata_missing_metadata_errors() {
    let mut obj = json!({});
    let err = reset_metadata(&mut obj).unwrap_err();
    assert_eq!(err, "metadata not found");
}

#[test]
fn reset_metadata_wrong_type_errors() {
    let mut obj = json!({"metadata": "somestring"});
    let err = reset_metadata(&mut obj).unwrap_err();
    assert!(
        err.contains("expected map[string]any"),
        "got {err:?}"
    );
}

#[test]
fn reset_metadata_does_not_touch_status() {
    let mut obj = json!({"metadata": {"name": "n"}, "status": {"phase": "Bound"}});
    reset_metadata(&mut obj).unwrap();
    assert!(obj.get("status").is_some(), "status must survive resetMetadata");
}

// ── resetStatus ──────────────────────────────────────────────────────────────

#[test]
fn reset_status_removes_status() {
    let mut obj = json!({"metadata": {"name": "n"}, "status": {"phase": "Bound"}});
    reset_status(&mut obj);
    assert!(obj.get("status").is_none(), "got {obj:?}");
    assert!(obj.get("metadata").is_some(), "metadata must survive");
}

#[test]
fn reset_status_noop_when_absent() {
    let mut obj = json!({});
    reset_status(&mut obj);
    assert_eq!(obj, json!({}));
}

// ── resetMetadataAndStatus ───────────────────────────────────────────────────

#[test]
fn reset_metadata_and_status_combines() {
    let mut obj = json!({
        "metadata": {"name": "pv-1", "uid": "abc", "resourceVersion": "9"},
        "status": {"phase": "Bound"}
    });
    reset_metadata_and_status(&mut obj).unwrap();
    let md = obj["metadata"].as_object().unwrap();
    assert!(md.contains_key("name"));
    assert!(!md.contains_key("uid"));
    assert!(!md.contains_key("resourceVersion"));
    assert!(obj.get("status").is_none());
}

#[test]
fn reset_metadata_and_status_propagates_metadata_error() {
    let mut obj = json!({"status": {"phase": "Bound"}});
    let err = reset_metadata_and_status(&mut obj).unwrap_err();
    assert_eq!(err, "metadata not found");
    // status must be untouched when metadata errors (upstream returns before resetStatus)
    assert!(obj.get("status").is_some());
}
