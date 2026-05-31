// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mirror tests for `reset_volume_binding_info`, ported from Velero
//! `pkg/restore/restore.go` `resetVolumeBindingInfo` (Apache-2.0). Cases mirror
//! upstream `Test_resetVolumeBindingInfo` (the single table-driven test covers
//! both the PV and PVC paths — there is no separate `OnPVC` helper).

use cave_backup::volume_binding::reset_volume_binding_info;
use serde_json::json;

const BIND_COMPLETED: &str = "pv.kubernetes.io/bind-completed";
const BOUND_BY_CONTROLLER: &str = "pv.kubernetes.io/bound-by-controller";
const PROVISIONED_BY: &str = "pv.kubernetes.io/provisioned-by";
const SELECTED_NODE: &str = "volume.kubernetes.io/selected-node";

#[test]
fn pv_bound_strips_claimref_ids_and_bind_annotations() {
    let mut obj = json!({
        "kind": "PersistentVolume",
        "metadata": {
            "name": "pv-1",
            "annotations": {
                BIND_COMPLETED: "yes",
                BOUND_BY_CONTROLLER: "yes",
                PROVISIONED_BY: "csi.example.io"
            }
        },
        "spec": {
            "claimRef": {"namespace": "ns-1", "name": "pvc-1", "uid": "abc", "resourceVersion": "1"}
        }
    });
    reset_volume_binding_info(&mut obj);

    // claimRef trimmed to namespace + name only
    let claim_ref = obj["spec"]["claimRef"].as_object().unwrap();
    assert!(claim_ref.contains_key("namespace"));
    assert!(claim_ref.contains_key("name"));
    assert!(!claim_ref.contains_key("uid"));
    assert!(!claim_ref.contains_key("resourceVersion"));

    // binding annotations gone, provisioned-by kept
    let ann = obj["metadata"]["annotations"].as_object().unwrap();
    assert!(!ann.contains_key(BIND_COMPLETED));
    assert!(!ann.contains_key(BOUND_BY_CONTROLLER));
    assert!(ann.contains_key(PROVISIONED_BY), "provisioned-by must survive for PVs");
}

#[test]
fn pvc_bound_strips_bind_annotations_keeps_volume_name() {
    let mut obj = json!({
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": "pvc-1",
            "annotations": {
                BIND_COMPLETED: "yes",
                BOUND_BY_CONTROLLER: "yes"
            }
        },
        "spec": {"volumeName": "pv-1"}
    });
    reset_volume_binding_info(&mut obj);

    let ann = obj["metadata"]["annotations"].as_object().unwrap();
    assert!(ann.is_empty(), "both binding annotations should be gone, got {ann:?}");
    assert_eq!(obj["spec"]["volumeName"], json!("pv-1"), "volumeName must survive");
    // no claimRef present -> RemoveNestedField is a no-op (no panic)
}

#[test]
fn pvc_keeps_selected_node_annotation() {
    let mut obj = json!({
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": "pvc-2",
            "annotations": {
                BIND_COMPLETED: "yes",
                BOUND_BY_CONTROLLER: "yes",
                SELECTED_NODE: "node-1"
            }
        },
        "spec": {}
    });
    reset_volume_binding_info(&mut obj);
    let ann = obj["metadata"]["annotations"].as_object().unwrap();
    assert!(!ann.contains_key(BIND_COMPLETED));
    assert!(!ann.contains_key(BOUND_BY_CONTROLLER));
    assert!(ann.contains_key(SELECTED_NODE), "selected-node must NOT be removed");
}

#[test]
fn handles_missing_annotations_and_claimref() {
    // bare object with neither annotations nor claimRef must not panic
    let mut obj = json!({"kind": "PersistentVolume", "metadata": {"name": "pv"}, "spec": {}});
    reset_volume_binding_info(&mut obj);
    assert_eq!(obj["metadata"]["name"], json!("pv"));
}
