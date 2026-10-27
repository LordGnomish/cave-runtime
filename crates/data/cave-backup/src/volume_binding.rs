// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PersistentVolume / PersistentVolumeClaim binding-info reset.
//!
//! Faithful line-port of Velero `pkg/restore/restore.go` `resetVolumeBindingInfo`
//! (Apache-2.0). On restore the backed-up PV looks like a statically
//! provisioned, manually-bound volume so that the Kubernetes PV(C) controllers
//! re-bind the two objects: the highly-unique `spec.claimRef` identity fields
//! and the controller bind-tracking annotations are cleared. The same function
//! is applied to both PVs and PVCs upstream (a PVC simply has no `spec.claimRef`,
//! so those removals are no-ops). Pure in-memory transformation over an
//! unstructured object modelled as [`serde_json::Value`].

use serde_json::Value;

/// `pv.kubernetes.io/bind-completed` — signals the PV/PVC is already bound.
pub const KUBE_ANN_BIND_COMPLETED: &str = "pv.kubernetes.io/bind-completed";
/// `pv.kubernetes.io/bound-by-controller` — signals a controller did the bind.
pub const KUBE_ANN_BOUND_BY_CONTROLLER: &str = "pv.kubernetes.io/bound-by-controller";

/// Clear binding info out of a PersistentVolume / PersistentVolumeClaim, mutating
/// `obj` in place. Port of Velero `resetVolumeBindingInfo`:
///   * removes `spec.claimRef.uid` and `spec.claimRef.resourceVersion`
///     (`claimRef.name` / `claimRef.namespace` are preserved);
///   * deletes the `bind-completed` and `bound-by-controller` annotations
///     (other annotations — e.g. `provisioned-by`, `selected-node` — are kept).
pub fn reset_volume_binding_info(obj: &mut Value) {
    // Clean out ClaimRef UID and resourceVersion, since this information is
    // highly unique. (No-op for PVCs, which have no spec.claimRef.)
    if let Some(claim_ref) = obj
        .get_mut("spec")
        .and_then(|s| s.get_mut("claimRef"))
        .and_then(Value::as_object_mut)
    {
        claim_ref.remove("uid");
        claim_ref.remove("resourceVersion");
    }

    // Clear out the annotations used by the Kubernetes PV controllers to track
    // bindings, so the PV(C) controller binds the two objects again.
    if let Some(annotations) = obj
        .get_mut("metadata")
        .and_then(|m| m.get_mut("annotations"))
        .and_then(Value::as_object_mut)
    {
        annotations.remove(KUBE_ANN_BIND_COMPLETED);
        annotations.remove(KUBE_ANN_BOUND_BY_CONTROLLER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removes_only_the_two_bind_annotations() {
        let mut obj = json!({
            "metadata": {"annotations": {
                KUBE_ANN_BIND_COMPLETED: "yes",
                KUBE_ANN_BOUND_BY_CONTROLLER: "yes",
                "other": "keep"
            }}
        });
        reset_volume_binding_info(&mut obj);
        let ann = obj["metadata"]["annotations"].as_object().unwrap();
        assert_eq!(ann.len(), 1);
        assert_eq!(ann["other"], json!("keep"));
    }
}
