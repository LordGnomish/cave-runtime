// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AttachDetach controller — `pkg/controller/volume/attachdetach`.
//!
//! Reconciles the desired-state (pods that need volumes mounted on
//! particular nodes) with the actual-state (volumes already attached to
//! nodes). Emits VolumeAttachment objects to drive the CSI external
//! attacher / in-tree plugins.
//!
//! Three knobs:
//!
//! * `reconcilerSyncDuration` — how often to check (default 1m).
//! * `disable_attach_detach_reconciler_sync` — leaves attached volumes alone.
//! * `attach_detach_max_volumes_per_node` — soft cap (CSI driver field).

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VolumeRef {
    pub volume_id: String,
    pub node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredAttachment {
    pub volume_id: String,
    pub node: String,
    pub pod_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActualAttachment {
    pub volume_id: String,
    pub node: String,
    pub mount_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachAction {
    Attach(VolumeRef),
    Detach(VolumeRef),
}

/// Diff desired vs actual and emit the per-volume actions.
pub fn reconcile(desired: &[DesiredAttachment], actual: &[ActualAttachment]) -> Vec<AttachAction> {
    let mut actions = Vec::new();
    // Attach: every (volume, node) pair in desired but missing from actual.
    for d in desired {
        let exists = actual
            .iter()
            .any(|a| a.volume_id == d.volume_id && a.node == d.node);
        if !exists {
            actions.push(AttachAction::Attach(VolumeRef {
                volume_id: d.volume_id.clone(),
                node: d.node.clone(),
            }));
        }
    }
    // Detach: actual entries with mount_count == 0 that don't appear in desired.
    for a in actual {
        if a.mount_count > 0 {
            continue;
        }
        let needed = desired
            .iter()
            .any(|d| d.volume_id == a.volume_id && d.node == a.node);
        if !needed {
            actions.push(AttachAction::Detach(VolumeRef {
                volume_id: a.volume_id.clone(),
                node: a.node.clone(),
            }));
        }
    }
    actions
}

/// Evaluate the per-node volume cap. Mirrors `pkg/scheduler/framework/plugins/volumebinding/csi_volume_binding.go::ScoreNodeWithCSILimits`.
pub fn would_exceed_node_cap(
    actual: &[ActualAttachment],
    node: &str,
    additional: u32,
    cap: u32,
) -> bool {
    let count = actual.iter().filter(|a| a.node == node).count() as u32;
    count + additional > cap
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/volume/attachdetach/attach_detach_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn d(volume: &str, node: &str) -> DesiredAttachment {
        DesiredAttachment {
            volume_id: volume.into(),
            node: node.into(),
            pod_name: "p".into(),
        }
    }
    fn a(volume: &str, node: &str, mounts: u32) -> ActualAttachment {
        ActualAttachment {
            volume_id: volume.into(),
            node: node.into(),
            mount_count: mounts,
        }
    }

    #[test]
    fn missing_attachment_emits_attach() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "reconcile",
            "tenant-ad-attach"
        );
        let acts = reconcile(&[d("v1", "n1")], &[]);
        assert_eq!(acts.len(), 1);
        match &acts[0] {
            AttachAction::Attach(v) => assert_eq!(v.volume_id, "v1"),
            _ => panic!("expected Attach"),
        }
    }

    #[test]
    fn unused_volume_emits_detach() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "reconcile",
            "tenant-ad-detach"
        );
        let acts = reconcile(&[], &[a("v1", "n1", 0)]);
        assert_eq!(acts.len(), 1);
        match &acts[0] {
            AttachAction::Detach(v) => assert_eq!(v.volume_id, "v1"),
            _ => panic!("expected Detach"),
        }
    }

    #[test]
    fn mounted_volume_not_detached_even_if_no_pods_use_it() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "reconcile",
            "tenant-ad-mounted-protect"
        );
        let acts = reconcile(&[], &[a("v1", "n1", 1)]);
        assert!(acts.is_empty());
    }

    #[test]
    fn matching_desired_and_actual_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "reconcile",
            "tenant-ad-noop"
        );
        let acts = reconcile(&[d("v1", "n1")], &[a("v1", "n1", 1)]);
        assert!(acts.is_empty());
    }

    #[test]
    fn mixed_attach_and_detach_emit_both() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "reconcile",
            "tenant-ad-mixed"
        );
        let acts = reconcile(&[d("v1", "n1")], &[a("v1", "n1", 1), a("v2", "n1", 0)]);
        // v1 already attached → no attach. v2 has no use → detach.
        assert_eq!(acts.len(), 1);
        match &acts[0] {
            AttachAction::Detach(v) => assert_eq!(v.volume_id, "v2"),
            _ => panic!("expected Detach"),
        }
    }

    #[test]
    fn would_exceed_cap_when_at_limit() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/scheduler/framework/plugins/volumebinding/csi_volume_binding.go",
            "ScoreNodeWithCSILimits",
            "tenant-ad-cap-at-limit"
        );
        let actual = vec![a("v1", "n1", 1), a("v2", "n1", 1)];
        assert!(would_exceed_node_cap(&actual, "n1", 1, 2));
        assert!(!would_exceed_node_cap(&actual, "n1", 1, 3));
    }

    #[test]
    fn cap_check_scoped_per_node() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/scheduler/framework/plugins/volumebinding/csi_volume_binding.go",
            "ScoreNodeWithCSILimits",
            "tenant-ad-cap-per-node"
        );
        let actual = vec![a("v1", "n1", 1), a("v2", "n2", 1)];
        // Cap 1 on n1: adding 1 more would exceed.
        assert!(would_exceed_node_cap(&actual, "n1", 1, 1));
        // n2 also has 1 → adding 1 also exceeds.
        assert!(would_exceed_node_cap(&actual, "n2", 1, 1));
        // n3 has 0 → adding 1 fits.
        assert!(!would_exceed_node_cap(&actual, "n3", 1, 1));
    }

    #[test]
    fn attach_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/attachdetach/attach_detach_controller.go",
            "AttachAction",
            "tenant-ad-action-serde"
        );
        let v = VolumeRef {
            volume_id: "v".into(),
            node: "n".into(),
        };
        for a in [
            AttachAction::Attach(v.clone()),
            AttachAction::Detach(v.clone()),
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: AttachAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
