// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VolumeSnapshot lifecycle — `external-snapshotter` controllers
//! (`pkg/controller/volume/snapshot.go` upstream / sigs.k8s.io/external-snapshotter).
//!
//! Two CRDs in concert:
//!
//! * `VolumeSnapshot` (namespaced, user-facing) — points at a source PVC
//!   or a pre-provisioned `VolumeSnapshotContent`.
//! * `VolumeSnapshotContent` (cluster-scoped, infra-facing) — wraps the
//!   actual cloud-provider snapshot id; carries a `deletionPolicy`.
//!
//! State machine for a dynamic VS:
//!
//! `Pending → ContentCreated → ReadyToUse`. On VS deletion:
//! `deletionPolicy=Delete` removes both VS and underlying snapshot;
//! `Retain` removes only the VS, leaving the content + cloud snapshot.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletionPolicy {
    Delete,
    Retain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotPhase {
    Pending,
    ContentCreated,
    ReadyToUse,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SnapshotSource {
    /// Dynamic — controller will create a VolumeSnapshotContent for `pvc`.
    Pvc { name: String, namespace: String },
    /// Pre-provisioned — VS already references an existing content object.
    Content { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshot {
    pub name: String,
    pub namespace: String,
    pub source: SnapshotSource,
    pub bound_content_name: Option<String>,
    pub phase: SnapshotPhase,
    pub being_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshotContent {
    pub name: String,
    pub deletion_policy: DeletionPolicy,
    pub ready_to_use: bool,
    pub being_deleted: bool,
    /// Provider-side id (set once the snapshot has been taken).
    pub snapshot_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotAction {
    /// Create a new VolumeSnapshotContent for the dynamic VS.
    CreateContent,
    /// Bind the existing pre-provisioned content to the VS.
    BindContent,
    /// VS is bound but content not yet Ready — wait.
    WaitForContentReady,
    /// VS is Ready — no work.
    NoOp,
    /// VS is being deleted with policy=Delete → also delete the content.
    DeleteContent,
    /// VS is being deleted with policy=Retain → release content (clear ref).
    ReleaseContent,
}

pub fn evaluate(vs: &VolumeSnapshot, content: Option<&VolumeSnapshotContent>) -> SnapshotAction {
    if vs.being_deleted {
        return match content {
            Some(c) => match c.deletion_policy {
                DeletionPolicy::Delete => SnapshotAction::DeleteContent,
                DeletionPolicy::Retain => SnapshotAction::ReleaseContent,
            },
            None => SnapshotAction::NoOp,
        };
    }
    match (&vs.source, content) {
        (SnapshotSource::Pvc { .. }, None) => SnapshotAction::CreateContent,
        (SnapshotSource::Content { .. }, None) => SnapshotAction::BindContent,
        (_, Some(c)) if !c.ready_to_use => SnapshotAction::WaitForContentReady,
        (_, Some(_)) => SnapshotAction::NoOp,
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
    "syncSnapshot",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn vs(source: SnapshotSource, phase: SnapshotPhase, bound: Option<&str>) -> VolumeSnapshot {
        VolumeSnapshot {
            name: "snap".into(),
            namespace: "default".into(),
            source,
            bound_content_name: bound.map(|s| s.to_string()),
            phase,
            being_deleted: false,
        }
    }
    fn vsc(policy: DeletionPolicy, ready: bool) -> VolumeSnapshotContent {
        VolumeSnapshotContent {
            name: "snapcontent-1".into(),
            deletion_policy: policy,
            ready_to_use: ready,
            being_deleted: false,
            snapshot_handle: ready.then(|| "snap-id-1".into()),
        }
    }

    #[test]
    fn dynamic_vs_without_content_creates_content() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "syncSnapshot",
            "tenant-pv-snap-create"
        );
        let v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::Pending,
            None,
        );
        assert_eq!(evaluate(&v, None), SnapshotAction::CreateContent);
    }

    #[test]
    fn pre_provisioned_vs_without_bind_binds() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "syncSnapshotPreBound",
            "tenant-pv-snap-bind"
        );
        let v = vs(
            SnapshotSource::Content { name: "snapcontent-1".into() },
            SnapshotPhase::Pending,
            None,
        );
        assert_eq!(evaluate(&v, None), SnapshotAction::BindContent);
    }

    #[test]
    fn vs_with_unready_content_waits() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "syncSnapshot",
            "tenant-pv-snap-wait"
        );
        let v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::ContentCreated,
            Some("snapcontent-1"),
        );
        let c = vsc(DeletionPolicy::Delete, false);
        assert_eq!(evaluate(&v, Some(&c)), SnapshotAction::WaitForContentReady);
    }

    #[test]
    fn vs_with_ready_content_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "syncSnapshot",
            "tenant-pv-snap-noop"
        );
        let v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::ReadyToUse,
            Some("snapcontent-1"),
        );
        let c = vsc(DeletionPolicy::Retain, true);
        assert_eq!(evaluate(&v, Some(&c)), SnapshotAction::NoOp);
    }

    #[test]
    fn deletion_policy_delete_cascades_to_content() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "checkContentAndBoundStatus",
            "tenant-pv-snap-del-cascade"
        );
        let mut v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::ReadyToUse,
            Some("snapcontent-1"),
        );
        v.being_deleted = true;
        let c = vsc(DeletionPolicy::Delete, true);
        assert_eq!(evaluate(&v, Some(&c)), SnapshotAction::DeleteContent);
    }

    #[test]
    fn deletion_policy_retain_releases_content() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "checkContentAndBoundStatus",
            "tenant-pv-snap-retain"
        );
        let mut v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::ReadyToUse,
            Some("snapcontent-1"),
        );
        v.being_deleted = true;
        let c = vsc(DeletionPolicy::Retain, true);
        assert_eq!(evaluate(&v, Some(&c)), SnapshotAction::ReleaseContent);
    }

    #[test]
    fn deletion_without_content_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "syncSnapshot",
            "tenant-pv-snap-del-no-content"
        );
        let mut v = vs(
            SnapshotSource::Pvc { name: "pvc".into(), namespace: "default".into() },
            SnapshotPhase::Pending,
            None,
        );
        v.being_deleted = true;
        assert_eq!(evaluate(&v, None), SnapshotAction::NoOp);
    }

    #[test]
    fn snapshot_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/snapshot-controller",
            "SnapshotAction",
            "tenant-pv-snap-action-serde"
        );
        for a in [
            SnapshotAction::CreateContent,
            SnapshotAction::BindContent,
            SnapshotAction::WaitForContentReady,
            SnapshotAction::NoOp,
            SnapshotAction::DeleteContent,
            SnapshotAction::ReleaseContent,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: SnapshotAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn deletion_policy_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/apis/v1/types.go",
            "DeletionPolicy",
            "tenant-pv-snap-policy-serde"
        );
        for p in [DeletionPolicy::Delete, DeletionPolicy::Retain] {
            let s = serde_json::to_string(&p).unwrap();
            let back: DeletionPolicy = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
    }

    #[test]
    fn snapshot_phase_serializes() {
        let (_cite, _tenant) = test_ctx!(
            "sigs.k8s.io/external-snapshotter/pkg/apis/v1/types.go",
            "VolumeSnapshotStatus",
            "tenant-pv-snap-phase-serde"
        );
        for p in [
            SnapshotPhase::Pending,
            SnapshotPhase::ContentCreated,
            SnapshotPhase::ReadyToUse,
            SnapshotPhase::Failed,
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: SnapshotPhase = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
    }
}
