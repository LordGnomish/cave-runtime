// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cloud volume controller — the last `[[unmapped]]` subsystem of the
//! upstream `staging/src/k8s.io/cloud-provider/volume/` tree.
//!
//! Upstream pre-CSI shipped a per-cloud "volume" controller that reconciled
//! attached vs. desired-attached state on each node. CSI subsumes the bulk
//! of this responsibility, but cloud-provider implementations still need to
//! orchestrate `Detached → Attaching → Attached → Detaching → Detached`
//! transitions when a Pod moves between nodes and the underlying disk has
//! to follow.
//!
//! This module ports the **state-machine half** of the upstream subsystem
//! into a Rust shape suitable for unit testing without network — two
//! in-process implementations (`HetznerCloudVolume`, `AzureDiskCloudVolume`)
//! and a `CloudVolumeController` watch loop driven by a queue of desired
//! transitions with exponential backoff.

use crate::types::{Cite, UPSTREAM_VERSION};
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Volume identifier — opaque to the controller. Cloud-side semantics differ
/// (Hetzner uses an `u64`; Azure uses an ARM URI), so the controller treats
/// these as opaque strings.
pub type VolumeId = String;

/// Node identifier — matches the `node.metadata.name` upstream uses.
pub type NodeId = String;

/// Five-state lifecycle of an upstream cloud volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum VolumeState {
    Detached,
    Attaching,
    Attached,
    Detaching,
}

impl VolumeState {
    /// Successor in the canonical forward direction (used when a step is
    /// granted permission to proceed). `Attached` and `Detached` are stable
    /// terminal points; any other state advances one step.
    pub const fn next_forward(self) -> Self {
        match self {
            VolumeState::Detached => VolumeState::Attaching,
            VolumeState::Attaching => VolumeState::Attached,
            VolumeState::Attached => VolumeState::Detaching,
            VolumeState::Detaching => VolumeState::Detached,
        }
    }
}

/// Errors a cloud-volume implementation can report. Each maps onto a real
/// upstream failure mode.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VolumeError {
    #[error("volume {volume} not found")]
    NotFound { volume: VolumeId },
    #[error("volume {volume} is already attached to {node}")]
    AlreadyAttached { volume: VolumeId, node: NodeId },
    #[error("volume {volume} attached to {existing}, refusing to attach to {requested}")]
    AttachConflict {
        volume: VolumeId,
        existing: NodeId,
        requested: NodeId,
    },
    #[error("volume {volume} is not attached anywhere")]
    NotAttached { volume: VolumeId },
    #[error("timed out waiting for volume {volume} to reach {:?}", target)]
    WaitTimeout {
        volume: VolumeId,
        target: VolumeState,
    },
    #[error("cloud-side: {0}")]
    Cloud(String),
}

/// Trait every cloud-volume implementation must satisfy. Mirrors the four
/// surface methods of upstream's `volume.Attacher` plus the `wait_for_state`
/// helper used in controller tests.
#[async_trait]
pub trait CloudVolume: Send + Sync {
    async fn attach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError>;
    async fn detach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError>;
    async fn list_for_node(&self, node_id: &NodeId) -> Result<Vec<VolumeId>, VolumeError>;
    async fn wait_for_state(
        &self,
        volume_id: &VolumeId,
        target: VolumeState,
    ) -> Result<(), VolumeError>;
}

/// Per-volume bookkeeping shared by both provider impls.
#[derive(Debug, Clone)]
struct VolumeRecord {
    state: VolumeState,
    attached_to: Option<NodeId>,
}

impl VolumeRecord {
    fn detached() -> Self {
        Self {
            state: VolumeState::Detached,
            attached_to: None,
        }
    }
}

/// Shared in-memory state-machine — both Hetzner and Azure use this, only
/// the surface citation and provider-id prefix differ.
#[derive(Debug, Default)]
struct VolumeStore {
    volumes: HashMap<VolumeId, VolumeRecord>,
}

impl VolumeStore {
    fn ensure(&mut self, id: &VolumeId) -> &mut VolumeRecord {
        self.volumes
            .entry(id.clone())
            .or_insert_with(VolumeRecord::detached)
    }
}

/// Concrete Hetzner cloud-volume implementation. Maps onto
/// `hcloud-cloud-controller-manager/hcloud/volume.go`.
#[derive(Debug, Default)]
pub struct HetznerCloudVolume {
    inner: Arc<RwLock<VolumeStore>>,
}

impl HetznerCloudVolume {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-register a volume — test helper. Production code would learn of
    /// volumes via the volume-watcher informer.
    pub async fn seed(&self, id: VolumeId) {
        let mut g = self.inner.write().await;
        g.ensure(&id);
    }
}

#[async_trait]
impl CloudVolume for HetznerCloudVolume {
    async fn attach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError> {
        let mut g = self.inner.write().await;
        let rec = g.ensure(volume_id);
        match (rec.state, rec.attached_to.as_deref()) {
            (VolumeState::Attached, Some(n)) if n == node_id => Err(VolumeError::AlreadyAttached {
                volume: volume_id.clone(),
                node: node_id.clone(),
            }),
            (VolumeState::Attached, Some(n)) => Err(VolumeError::AttachConflict {
                volume: volume_id.clone(),
                existing: n.to_string(),
                requested: node_id.clone(),
            }),
            _ => {
                rec.state = VolumeState::Attaching;
                rec.attached_to = Some(node_id.clone());
                Ok(())
            }
        }
    }

    async fn detach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError> {
        let mut g = self.inner.write().await;
        let rec = g.volumes.get_mut(volume_id).ok_or(VolumeError::NotFound {
            volume: volume_id.clone(),
        })?;
        match rec.attached_to.as_deref() {
            Some(n) if n == node_id => {
                rec.state = VolumeState::Detaching;
                Ok(())
            }
            Some(_) | None => Err(VolumeError::NotAttached {
                volume: volume_id.clone(),
            }),
        }
    }

    async fn list_for_node(&self, node_id: &NodeId) -> Result<Vec<VolumeId>, VolumeError> {
        let g = self.inner.read().await;
        Ok(g.volumes
            .iter()
            .filter(|(_, r)| r.attached_to.as_deref() == Some(node_id.as_str()))
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn wait_for_state(
        &self,
        volume_id: &VolumeId,
        target: VolumeState,
    ) -> Result<(), VolumeError> {
        // Drive the state machine forward one tick at a time until it
        // reaches `target`, then return. The cap exists to make the helper
        // total — production code would block on a real cloud-watch.
        const MAX_TICKS: usize = 16;
        for _ in 0..MAX_TICKS {
            let mut g = self.inner.write().await;
            let rec = g.volumes.get_mut(volume_id).ok_or(VolumeError::NotFound {
                volume: volume_id.clone(),
            })?;
            if rec.state == target {
                if matches!(target, VolumeState::Detached) {
                    rec.attached_to = None;
                }
                return Ok(());
            }
            rec.state = rec.state.next_forward();
        }
        Err(VolumeError::WaitTimeout {
            volume: volume_id.clone(),
            target,
        })
    }
}

/// Concrete Azure cloud-volume implementation. Maps onto
/// `cloud-provider-azure/pkg/provider/azure_managed_disk.go`.
#[derive(Debug, Default)]
pub struct AzureDiskCloudVolume {
    inner: Arc<RwLock<VolumeStore>>,
}

impl AzureDiskCloudVolume {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn seed(&self, id: VolumeId) {
        let mut g = self.inner.write().await;
        g.ensure(&id);
    }
}

#[async_trait]
impl CloudVolume for AzureDiskCloudVolume {
    async fn attach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError> {
        let mut g = self.inner.write().await;
        let rec = g.ensure(volume_id);
        // Azure differs from Hetzner: re-attaching to the same node is a
        // no-op success, mirroring upstream's idempotent `AttachDisk`.
        if rec.state == VolumeState::Attached && rec.attached_to.as_deref() == Some(node_id.as_str())
        {
            return Ok(());
        }
        if let Some(existing) = rec.attached_to.as_deref() {
            if rec.state == VolumeState::Attached && existing != node_id.as_str() {
                return Err(VolumeError::AttachConflict {
                    volume: volume_id.clone(),
                    existing: existing.to_string(),
                    requested: node_id.clone(),
                });
            }
        }
        rec.state = VolumeState::Attaching;
        rec.attached_to = Some(node_id.clone());
        Ok(())
    }

    async fn detach(&self, node_id: &NodeId, volume_id: &VolumeId) -> Result<(), VolumeError> {
        let mut g = self.inner.write().await;
        let rec = g.volumes.get_mut(volume_id).ok_or(VolumeError::NotFound {
            volume: volume_id.clone(),
        })?;
        if rec.attached_to.as_deref() != Some(node_id.as_str()) {
            return Err(VolumeError::NotAttached {
                volume: volume_id.clone(),
            });
        }
        rec.state = VolumeState::Detaching;
        Ok(())
    }

    async fn list_for_node(&self, node_id: &NodeId) -> Result<Vec<VolumeId>, VolumeError> {
        let g = self.inner.read().await;
        let mut out: Vec<_> = g
            .volumes
            .iter()
            .filter(|(_, r)| r.attached_to.as_deref() == Some(node_id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        out.sort(); // Azure ARM returns sorted disk IDs deterministically
        Ok(out)
    }

    async fn wait_for_state(
        &self,
        volume_id: &VolumeId,
        target: VolumeState,
    ) -> Result<(), VolumeError> {
        const MAX_TICKS: usize = 16;
        for _ in 0..MAX_TICKS {
            let mut g = self.inner.write().await;
            let rec = g.volumes.get_mut(volume_id).ok_or(VolumeError::NotFound {
                volume: volume_id.clone(),
            })?;
            if rec.state == target {
                if matches!(target, VolumeState::Detached) {
                    rec.attached_to = None;
                }
                return Ok(());
            }
            rec.state = rec.state.next_forward();
        }
        Err(VolumeError::WaitTimeout {
            volume: volume_id.clone(),
            target,
        })
    }
}

/// One desired transition queued into the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeReconcileItem {
    pub volume_id: VolumeId,
    pub desired_state: VolumeState,
    pub node: NodeId,
    /// Number of consecutive failures so far, used for the exponential
    /// backoff schedule. Always starts at 0.
    pub failed_attempts: u32,
}

impl VolumeReconcileItem {
    pub fn new(volume_id: impl Into<VolumeId>, desired_state: VolumeState, node: impl Into<NodeId>) -> Self {
        Self {
            volume_id: volume_id.into(),
            desired_state,
            node: node.into(),
            failed_attempts: 0,
        }
    }

    /// Upstream uses a base * 2^attempts schedule capped at 30s; we mirror
    /// that with milliseconds for test ergonomics.
    pub fn backoff(&self) -> Duration {
        let base_ms: u64 = 100;
        let cap_ms: u64 = 30_000;
        let shift = self.failed_attempts.min(16);
        let ms = base_ms.saturating_mul(1u64 << shift).min(cap_ms);
        Duration::from_millis(ms)
    }
}

/// Outcome of a single `tick` of the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// Queue is empty.
    Idle,
    /// Item driven to its desired state and removed from the queue.
    Reached(VolumeId, VolumeState),
    /// Item failed this round; backoff schedule was incremented.
    Requeued { volume_id: VolumeId, backoff: Duration },
}

/// Watch-loop controller. Holds a queue of pending transitions and steps
/// one item per `tick`, mirroring upstream's `workqueue.Add / Get / Done`
/// loop. The actual driving is done via the `CloudVolume` trait so the
/// controller is identical for Hetzner and Azure.
pub struct CloudVolumeController {
    queue: VecDeque<VolumeReconcileItem>,
    /// Max consecutive failures per item before we drop it.
    max_failed_attempts: u32,
}

impl CloudVolumeController {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            max_failed_attempts: 5,
        }
    }

    pub fn with_max_failures(max_failed_attempts: u32) -> Self {
        Self {
            queue: VecDeque::new(),
            max_failed_attempts,
        }
    }

    pub fn enqueue(&mut self, item: VolumeReconcileItem) {
        self.queue.push_back(item);
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Drive a single queued item by one `attach`/`detach` + `wait_for_state`
    /// roundtrip. Returns the outcome. The controller does not sleep on
    /// requeue — the backoff duration is reported so the caller can decide.
    pub async fn tick<V: CloudVolume>(&mut self, cv: &V) -> TickOutcome {
        let Some(mut item) = self.queue.pop_front() else {
            return TickOutcome::Idle;
        };

        // Pick the right side-effect for the desired state.
        let step = match item.desired_state {
            VolumeState::Attached | VolumeState::Attaching => {
                cv.attach(&item.node, &item.volume_id).await
            }
            VolumeState::Detached | VolumeState::Detaching => {
                cv.detach(&item.node, &item.volume_id).await
            }
        };

        // `AlreadyAttached` to the requested node is a success for an
        // attach goal — the desired state is already in place.
        let step_ok = matches!(
            (item.desired_state, &step),
            (VolumeState::Attached, Ok(()))
                | (VolumeState::Attaching, Ok(()))
                | (VolumeState::Detached, Ok(()))
                | (VolumeState::Detaching, Ok(()))
                | (VolumeState::Attached, Err(VolumeError::AlreadyAttached { .. }))
        );

        if !step_ok {
            item.failed_attempts = item.failed_attempts.saturating_add(1);
            if item.failed_attempts >= self.max_failed_attempts {
                // Drop the poisonous item — upstream calls this
                // `workqueue.Forget`.
                return TickOutcome::Requeued {
                    volume_id: item.volume_id,
                    backoff: Duration::from_secs(0),
                };
            }
            let backoff = item.backoff();
            let vid = item.volume_id.clone();
            self.queue.push_back(item);
            return TickOutcome::Requeued {
                volume_id: vid,
                backoff,
            };
        }

        // Side-effect succeeded — drive state machine forward.
        let waited = cv.wait_for_state(&item.volume_id, item.desired_state).await;
        match waited {
            Ok(()) => TickOutcome::Reached(item.volume_id, item.desired_state),
            Err(_) => {
                item.failed_attempts = item.failed_attempts.saturating_add(1);
                let backoff = item.backoff();
                let vid = item.volume_id.clone();
                if item.failed_attempts < self.max_failed_attempts {
                    self.queue.push_back(item);
                }
                TickOutcome::Requeued {
                    volume_id: vid,
                    backoff,
                }
            }
        }
    }
}

impl Default for CloudVolumeController {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite {
    repo: "kubernetes/kubernetes",
    path: "staging/src/k8s.io/cloud-provider/volume/",
    symbol: "VolumeController",
    version: UPSTREAM_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(s: &str) -> VolumeId {
        s.to_string()
    }
    fn node(s: &str) -> NodeId {
        s.to_string()
    }

    #[test]
    fn next_forward_is_a_four_state_cycle() {
        assert_eq!(VolumeState::Detached.next_forward(), VolumeState::Attaching);
        assert_eq!(VolumeState::Attaching.next_forward(), VolumeState::Attached);
        assert_eq!(VolumeState::Attached.next_forward(), VolumeState::Detaching);
        assert_eq!(VolumeState::Detaching.next_forward(), VolumeState::Detached);
    }

    #[test]
    fn backoff_grows_then_caps_at_30s() {
        let mut item = VolumeReconcileItem::new("v1", VolumeState::Attached, "n1");
        let b0 = item.backoff();
        item.failed_attempts = 1;
        let b1 = item.backoff();
        item.failed_attempts = 20;
        let b_cap = item.backoff();
        assert_eq!(b0, Duration::from_millis(100));
        assert_eq!(b1, Duration::from_millis(200));
        assert_eq!(b_cap, Duration::from_millis(30_000));
    }

    #[tokio::test]
    async fn hetzner_attach_then_wait_reaches_attached_state() {
        let cv = HetznerCloudVolume::new();
        cv.seed(vol("v1")).await;
        cv.attach(&node("n1"), &vol("v1")).await.unwrap();
        cv.wait_for_state(&vol("v1"), VolumeState::Attached)
            .await
            .unwrap();
        let attached = cv.list_for_node(&node("n1")).await.unwrap();
        assert_eq!(attached, vec!["v1".to_string()]);
    }

    #[tokio::test]
    async fn hetzner_attach_to_second_node_returns_conflict() {
        let cv = HetznerCloudVolume::new();
        cv.seed(vol("v1")).await;
        cv.attach(&node("n1"), &vol("v1")).await.unwrap();
        cv.wait_for_state(&vol("v1"), VolumeState::Attached)
            .await
            .unwrap();
        let err = cv.attach(&node("n2"), &vol("v1")).await.unwrap_err();
        assert!(matches!(err, VolumeError::AttachConflict { .. }), "got {:?}", err);
    }

    #[tokio::test]
    async fn hetzner_full_lifecycle_detached_attached_detached() {
        let cv = HetznerCloudVolume::new();
        cv.seed(vol("v1")).await;
        cv.attach(&node("n1"), &vol("v1")).await.unwrap();
        cv.wait_for_state(&vol("v1"), VolumeState::Attached)
            .await
            .unwrap();
        cv.detach(&node("n1"), &vol("v1")).await.unwrap();
        cv.wait_for_state(&vol("v1"), VolumeState::Detached)
            .await
            .unwrap();
        let attached = cv.list_for_node(&node("n1")).await.unwrap();
        assert!(attached.is_empty(), "node should have no volumes left");
    }

    #[tokio::test]
    async fn azure_reattach_to_same_node_is_idempotent() {
        let cv = AzureDiskCloudVolume::new();
        cv.seed(vol("disk-a")).await;
        cv.attach(&node("aks-0"), &vol("disk-a")).await.unwrap();
        cv.wait_for_state(&vol("disk-a"), VolumeState::Attached)
            .await
            .unwrap();
        // Second call must succeed silently — Azure attaches are idempotent.
        cv.attach(&node("aks-0"), &vol("disk-a")).await.unwrap();
    }

    #[tokio::test]
    async fn azure_list_for_node_is_sorted() {
        let cv = AzureDiskCloudVolume::new();
        for v in ["disk-c", "disk-a", "disk-b"] {
            cv.seed(vol(v)).await;
            cv.attach(&node("aks-0"), &vol(v)).await.unwrap();
            cv.wait_for_state(&vol(v), VolumeState::Attached)
                .await
                .unwrap();
        }
        let list = cv.list_for_node(&node("aks-0")).await.unwrap();
        assert_eq!(list, vec!["disk-a", "disk-b", "disk-c"]);
    }

    #[tokio::test]
    async fn azure_detach_on_unattached_volume_returns_not_attached() {
        let cv = AzureDiskCloudVolume::new();
        cv.seed(vol("disk-x")).await;
        let err = cv.detach(&node("aks-0"), &vol("disk-x")).await.unwrap_err();
        assert!(matches!(err, VolumeError::NotAttached { .. }), "got {:?}", err);
    }

    #[tokio::test]
    async fn controller_idle_when_queue_is_empty() {
        let cv = HetznerCloudVolume::new();
        let mut ctrl = CloudVolumeController::new();
        assert_eq!(ctrl.tick(&cv).await, TickOutcome::Idle);
    }

    #[tokio::test]
    async fn controller_reaches_attached_then_pops_item() {
        let cv = HetznerCloudVolume::new();
        cv.seed(vol("v1")).await;
        let mut ctrl = CloudVolumeController::new();
        ctrl.enqueue(VolumeReconcileItem::new("v1", VolumeState::Attached, "n1"));
        assert_eq!(ctrl.pending(), 1);
        let out = ctrl.tick(&cv).await;
        assert_eq!(out, TickOutcome::Reached("v1".into(), VolumeState::Attached));
        assert_eq!(ctrl.pending(), 0);
    }

    #[tokio::test]
    async fn controller_requeues_on_conflict_and_drops_after_max() {
        let cv = AzureDiskCloudVolume::new();
        cv.seed(vol("disk-x")).await;
        cv.attach(&node("aks-0"), &vol("disk-x")).await.unwrap();
        cv.wait_for_state(&vol("disk-x"), VolumeState::Attached)
            .await
            .unwrap();
        // Try to attach to a *different* node — that's a conflict.
        let mut ctrl = CloudVolumeController::with_max_failures(2);
        ctrl.enqueue(VolumeReconcileItem::new(
            "disk-x",
            VolumeState::Attached,
            "aks-1",
        ));
        let out1 = ctrl.tick(&cv).await;
        assert!(matches!(out1, TickOutcome::Requeued { .. }));
        // Still queued for retry.
        assert_eq!(ctrl.pending(), 1);
        let out2 = ctrl.tick(&cv).await;
        assert!(matches!(out2, TickOutcome::Requeued { .. }));
        // Hit max_failed_attempts on the second failure; item gets dropped.
        assert_eq!(ctrl.pending(), 0);
    }

    #[tokio::test]
    async fn controller_drives_full_detach_lifecycle() {
        let cv = HetznerCloudVolume::new();
        cv.seed(vol("v1")).await;
        cv.attach(&node("n1"), &vol("v1")).await.unwrap();
        cv.wait_for_state(&vol("v1"), VolumeState::Attached)
            .await
            .unwrap();
        let mut ctrl = CloudVolumeController::new();
        ctrl.enqueue(VolumeReconcileItem::new("v1", VolumeState::Detached, "n1"));
        let out = ctrl.tick(&cv).await;
        assert_eq!(out, TickOutcome::Reached("v1".into(), VolumeState::Detached));
        assert!(cv.list_for_node(&node("n1")).await.unwrap().is_empty());
    }
}
