// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CSI volume manager — Node service operations.
//!
//! Mirrors the upstream kubelet's `pkg/volume/csi` and `pkg/kubelet/volumemanager`
//! state machine: NodeStageVolume, NodePublishVolume, NodeUnpublishVolume,
//! NodeUnstageVolume, plus fsType / mountOptions / fsGroupPolicy / accessModes /
//! inline ephemeral / raw block / snapshot semantics.
//!
//! Operations are idempotent. State transitions follow the CSI Node spec.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    /// RWO — single node, read/write.
    ReadWriteOnce,
    /// ROX — many nodes, read-only.
    ReadOnlyMany,
    /// RWX — many nodes, read/write.
    ReadWriteMany,
    /// RWOP — single pod, read/write (k8s 1.22+).
    ReadWriteOncePod,
}

impl AccessMode {
    pub fn allows_multi_node(self) -> bool {
        matches!(self, AccessMode::ReadOnlyMany | AccessMode::ReadWriteMany)
    }

    pub fn is_read_only(self) -> bool {
        matches!(self, AccessMode::ReadOnlyMany)
    }

    pub fn allows_multi_pod(self) -> bool {
        // RWOP restricts to a single pod even on the same node.
        !matches!(self, AccessMode::ReadWriteOncePod)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeMode {
    /// Mounted filesystem (default).
    Filesystem,
    /// Raw block device (`volumeMode: Block`).
    Block,
}

impl Default for VolumeMode {
    fn default() -> Self {
        VolumeMode::Filesystem
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsGroupPolicy {
    /// CSI driver does not support fsGroup; kubelet skips application.
    None,
    /// kubelet always applies fsGroup ownership recursively.
    File,
    /// Apply fsGroup only when access mode is RWO and fsType is set.
    ReadWriteOnceWithFSType,
}

impl Default for FsGroupPolicy {
    fn default() -> Self {
        FsGroupPolicy::ReadWriteOnceWithFSType
    }
}

/// Whether the kubelet should apply a given fsGroup for this volume given
/// the driver-declared policy and the volume's access mode and fsType.
pub fn should_apply_fs_group(
    policy: FsGroupPolicy,
    access_mode: AccessMode,
    fs_type: Option<&str>,
) -> bool {
    match policy {
        FsGroupPolicy::None => false,
        FsGroupPolicy::File => true,
        FsGroupPolicy::ReadWriteOnceWithFSType => {
            access_mode == AccessMode::ReadWriteOnce
                && fs_type.map(|s| !s.is_empty()).unwrap_or(false)
        }
    }
}

/// Whether the supplied list of access modes is internally consistent.
/// (RWOP cannot be combined with anything that allows multi-pod.)
pub fn validate_access_mode_set(modes: &[AccessMode]) -> Result<(), CsiError> {
    if modes.is_empty() {
        return Err(CsiError::InvalidArgument(
            "at least one access mode required".into(),
        ));
    }
    let has_rwop = modes.iter().any(|m| *m == AccessMode::ReadWriteOncePod);
    if has_rwop && modes.len() > 1 {
        return Err(CsiError::InvalidArgument(
            "ReadWriteOncePod cannot be combined with other access modes".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeCapability {
    pub access_mode: AccessMode,
    pub volume_mode: VolumeMode,
    pub fs_type: Option<String>,
    pub mount_options: Vec<String>,
}

impl VolumeCapability {
    pub fn fs(access: AccessMode, fs_type: &str) -> Self {
        Self {
            access_mode: access,
            volume_mode: VolumeMode::Filesystem,
            fs_type: Some(fs_type.to_string()),
            mount_options: Vec::new(),
        }
    }

    pub fn block(access: AccessMode) -> Self {
        Self {
            access_mode: access,
            volume_mode: VolumeMode::Block,
            fs_type: None,
            mount_options: Vec::new(),
        }
    }

    pub fn with_mount_options(mut self, opts: Vec<String>) -> Self {
        self.mount_options = opts;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeStage {
    NotStaged,
    Staged,
    Published,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedVolume {
    pub volume_id: String,
    pub staging_target_path: String,
    pub capability: VolumeCapability,
    pub volume_context: BTreeMap<String, String>,
    pub readonly: bool,
    pub published_targets: Vec<PublishedMount>,
    pub staged_at: DateTime<Utc>,
    /// Size requested in PVC.spec.resources.requests.storage (bytes).
    pub size_bytes: u64,
    /// Number of pods that requested this RWOP volume on this node.
    pub rwop_holder_pod_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedMount {
    pub pod_uid: String,
    pub target_path: String,
    pub readonly: bool,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineEphemeralVolume {
    pub pod_uid: String,
    pub volume_name: String,
    pub driver: String,
    pub volume_attributes: BTreeMap<String, String>,
    pub fs_type: Option<String>,
    pub target_path: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshot {
    pub snapshot_id: String,
    pub source_volume_id: String,
    pub size_bytes: u64,
    pub creation_time: DateTime<Utc>,
    pub ready_to_use: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CsiError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("failed precondition: {0}")]
    FailedPrecondition(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("aborted: {0}")]
    Aborted(String),
    #[error("out of range: {0}")]
    OutOfRange(String),
}

pub type CsiResult<T> = Result<T, CsiError>;

#[derive(Debug, Default)]
pub struct VolumeManager {
    /// Driver-declared fsGroupPolicy keyed by driver name.
    pub fs_group_policies: DashMap<String, FsGroupPolicy>,
    /// Driver-declared supported access modes keyed by driver name.
    pub driver_access_modes: DashMap<String, Vec<AccessMode>>,
    pub staged: DashMap<String, StagedVolume>,
    pub inline_ephemeral: DashMap<String, InlineEphemeralVolume>,
    pub snapshots: DashMap<String, VolumeSnapshot>,
    /// VolumeAttachment status per (node, volume) — multi-attach guard.
    pub attachments: DashMap<String, AttachmentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub volume_id: String,
    pub node_name: String,
    pub attached: bool,
    pub access_mode: AccessMode,
}

impl VolumeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_driver(&self, driver: &str, policy: FsGroupPolicy, modes: Vec<AccessMode>) {
        self.fs_group_policies.insert(driver.to_string(), policy);
        self.driver_access_modes.insert(driver.to_string(), modes);
    }

    pub fn driver_supports_access_mode(&self, driver: &str, mode: AccessMode) -> bool {
        self.driver_access_modes
            .get(driver)
            .map(|m| m.contains(&mode))
            .unwrap_or(false)
    }

    /// CSI NodeStageVolume — mount to staging path. Idempotent.
    pub fn node_stage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
        capability: VolumeCapability,
        volume_context: BTreeMap<String, String>,
        readonly: bool,
        size_bytes: u64,
    ) -> CsiResult<()> {
        if volume_id.is_empty() {
            return Err(CsiError::InvalidArgument("volume_id empty".into()));
        }
        if staging_target_path.is_empty() {
            return Err(CsiError::InvalidArgument(
                "staging_target_path empty".into(),
            ));
        }
        // Block-mode volumes ignore mount options / fsType but everything else is shared.
        if capability.volume_mode == VolumeMode::Filesystem
            && capability
                .fs_type
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
        {
            return Err(CsiError::InvalidArgument(
                "fsType required for filesystem volume mode".into(),
            ));
        }
        if let Some(existing) = self.staged.get(volume_id) {
            // Idempotency: same path & capability → ok; differ → AlreadyExists.
            if existing.staging_target_path != staging_target_path {
                return Err(CsiError::AlreadyExists(format!(
                    "volume {} already staged at different path",
                    volume_id
                )));
            }
            if existing.capability != capability {
                return Err(CsiError::AlreadyExists(format!(
                    "volume {} already staged with different capability",
                    volume_id
                )));
            }
            return Ok(());
        }
        self.staged.insert(
            volume_id.to_string(),
            StagedVolume {
                volume_id: volume_id.to_string(),
                staging_target_path: staging_target_path.to_string(),
                capability,
                volume_context,
                readonly,
                published_targets: Vec::new(),
                staged_at: Utc::now(),
                size_bytes,
                rwop_holder_pod_uid: None,
            },
        );
        Ok(())
    }

    /// CSI NodeUnstageVolume. Idempotent: missing volume → ok.
    pub fn node_unstage_volume(&self, volume_id: &str, staging_target_path: &str) -> CsiResult<()> {
        if volume_id.is_empty() || staging_target_path.is_empty() {
            return Err(CsiError::InvalidArgument(
                "volume_id and staging_target_path required".into(),
            ));
        }
        match self.staged.get(volume_id) {
            Some(v) => {
                if v.staging_target_path != staging_target_path {
                    return Err(CsiError::FailedPrecondition("staging path mismatch".into()));
                }
                if !v.published_targets.is_empty() {
                    return Err(CsiError::FailedPrecondition(format!(
                        "volume {} still has {} published target(s)",
                        volume_id,
                        v.published_targets.len()
                    )));
                }
                drop(v);
                self.staged.remove(volume_id);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// CSI NodePublishVolume — bind-mount staging path to per-pod target.
    /// Idempotent for (volume, pod, target).
    pub fn node_publish_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
        target_path: &str,
        pod_uid: &str,
        readonly: bool,
    ) -> CsiResult<()> {
        if target_path.is_empty() {
            return Err(CsiError::InvalidArgument("target_path empty".into()));
        }
        let mut entry = self.staged.get_mut(volume_id).ok_or_else(|| {
            CsiError::FailedPrecondition(format!(
                "volume {} not staged; call NodeStageVolume first",
                volume_id
            ))
        })?;
        if entry.staging_target_path != staging_target_path {
            return Err(CsiError::FailedPrecondition("staging path mismatch".into()));
        }
        // RWOP: only one pod allowed.
        if entry.capability.access_mode == AccessMode::ReadWriteOncePod {
            if let Some(holder) = &entry.rwop_holder_pod_uid {
                if holder != pod_uid {
                    return Err(CsiError::FailedPrecondition(format!(
                        "RWOP volume {} already held by pod {}",
                        volume_id, holder
                    )));
                }
            } else {
                entry.rwop_holder_pod_uid = Some(pod_uid.to_string());
            }
        }
        // Read-only on volume forces all publish-readonly.
        let effective_ro =
            readonly || entry.readonly || entry.capability.access_mode.is_read_only();
        // Idempotency on (pod_uid, target_path).
        if let Some(existing) = entry
            .published_targets
            .iter()
            .find(|p| p.pod_uid == pod_uid && p.target_path == target_path)
        {
            if existing.readonly != effective_ro {
                return Err(CsiError::AlreadyExists(format!(
                    "volume {} already published at {} with different readonly flag",
                    volume_id, target_path
                )));
            }
            return Ok(());
        }
        entry.published_targets.push(PublishedMount {
            pod_uid: pod_uid.to_string(),
            target_path: target_path.to_string(),
            readonly: effective_ro,
            published_at: Utc::now(),
        });
        Ok(())
    }

    /// CSI NodeUnpublishVolume — unbind target path. Idempotent.
    pub fn node_unpublish_volume(&self, volume_id: &str, target_path: &str) -> CsiResult<()> {
        let mut entry = match self.staged.get_mut(volume_id) {
            Some(e) => e,
            None => return Ok(()),
        };
        let before = entry.published_targets.len();
        entry
            .published_targets
            .retain(|p| p.target_path != target_path);
        // If RWOP holder's last mount removed, release.
        if entry.published_targets.is_empty() {
            entry.rwop_holder_pod_uid = None;
        }
        let _ = before;
        Ok(())
    }

    pub fn stage_state(&self, volume_id: &str) -> VolumeStage {
        match self.staged.get(volume_id) {
            None => VolumeStage::NotStaged,
            Some(v) => {
                if v.published_targets.is_empty() {
                    VolumeStage::Staged
                } else {
                    VolumeStage::Published
                }
            }
        }
    }

    pub fn published_count(&self, volume_id: &str) -> usize {
        self.staged
            .get(volume_id)
            .map(|v| v.published_targets.len())
            .unwrap_or(0)
    }

    /// CSI inline ephemeral: NodePublish without prior NodeStage.
    pub fn publish_inline_ephemeral(
        &self,
        pod_uid: &str,
        volume_name: &str,
        driver: &str,
        volume_attributes: BTreeMap<String, String>,
        fs_type: Option<String>,
        target_path: &str,
    ) -> CsiResult<()> {
        if pod_uid.is_empty()
            || volume_name.is_empty()
            || driver.is_empty()
            || target_path.is_empty()
        {
            return Err(CsiError::InvalidArgument("required field empty".into()));
        }
        let key = ephemeral_key(pod_uid, volume_name);
        if self.inline_ephemeral.contains_key(&key) {
            return Ok(());
        }
        self.inline_ephemeral.insert(
            key,
            InlineEphemeralVolume {
                pod_uid: pod_uid.to_string(),
                volume_name: volume_name.to_string(),
                driver: driver.to_string(),
                volume_attributes,
                fs_type,
                target_path: target_path.to_string(),
                published_at: Utc::now(),
            },
        );
        Ok(())
    }

    pub fn unpublish_inline_ephemeral(&self, pod_uid: &str, volume_name: &str) -> CsiResult<()> {
        let key = ephemeral_key(pod_uid, volume_name);
        self.inline_ephemeral.remove(&key);
        Ok(())
    }

    /// Snapshot: CSI CreateSnapshot.
    pub fn create_snapshot(
        &self,
        snapshot_id: &str,
        source_volume_id: &str,
    ) -> CsiResult<VolumeSnapshot> {
        let src = self
            .staged
            .get(source_volume_id)
            .ok_or_else(|| CsiError::NotFound(format!("volume {} not staged", source_volume_id)))?;
        if let Some(existing) = self.snapshots.get(snapshot_id) {
            if existing.source_volume_id != source_volume_id {
                return Err(CsiError::AlreadyExists(format!(
                    "snapshot {} exists with different source",
                    snapshot_id
                )));
            }
            return Ok(existing.clone());
        }
        let snap = VolumeSnapshot {
            snapshot_id: snapshot_id.to_string(),
            source_volume_id: source_volume_id.to_string(),
            size_bytes: src.size_bytes,
            creation_time: Utc::now(),
            ready_to_use: true,
        };
        self.snapshots.insert(snapshot_id.to_string(), snap.clone());
        Ok(snap)
    }

    pub fn delete_snapshot(&self, snapshot_id: &str) -> CsiResult<()> {
        self.snapshots.remove(snapshot_id);
        Ok(())
    }

    /// Restore a snapshot into a new volume (Stage operation).
    pub fn restore_snapshot(
        &self,
        snapshot_id: &str,
        new_volume_id: &str,
        staging_target_path: &str,
        capability: VolumeCapability,
    ) -> CsiResult<()> {
        let snap = self
            .snapshots
            .get(snapshot_id)
            .ok_or_else(|| CsiError::NotFound(format!("snapshot {} not found", snapshot_id)))?;
        if !snap.ready_to_use {
            return Err(CsiError::FailedPrecondition(
                "snapshot not ready_to_use".into(),
            ));
        }
        let size = snap.size_bytes;
        drop(snap);
        self.node_stage_volume(
            new_volume_id,
            staging_target_path,
            capability,
            BTreeMap::new(),
            false,
            size,
        )
    }

    /// Multi-attach guard. RWO/RWOP must not be attached to a second node.
    pub fn record_attachment(
        &self,
        volume_id: &str,
        node_name: &str,
        access_mode: AccessMode,
    ) -> CsiResult<()> {
        // Existing attachment check.
        for r in self.attachments.iter() {
            let rec = r.value();
            if rec.volume_id == volume_id
                && rec.attached
                && rec.node_name != node_name
                && !access_mode.allows_multi_node()
            {
                return Err(CsiError::FailedPrecondition(format!(
                    "volume {} already attached to node {}",
                    volume_id, rec.node_name
                )));
            }
        }
        let key = attachment_key(volume_id, node_name);
        self.attachments.insert(
            key,
            AttachmentRecord {
                volume_id: volume_id.to_string(),
                node_name: node_name.to_string(),
                attached: true,
                access_mode,
            },
        );
        Ok(())
    }

    pub fn detach(&self, volume_id: &str, node_name: &str) {
        let key = attachment_key(volume_id, node_name);
        self.attachments.remove(&key);
    }
}

fn ephemeral_key(pod_uid: &str, volume_name: &str) -> String {
    format!("{}/{}", pod_uid, volume_name)
}

fn attachment_key(volume_id: &str, node_name: &str) -> String {
    format!("{}@{}", volume_id, node_name)
}

/// Validate fsType against a small allowlist that mirrors what the
/// kubelet's mount-utils accepts on Linux for filesystem-mode volumes.
pub fn validate_fs_type(fs_type: &str) -> CsiResult<()> {
    const SUPPORTED: &[&str] = &[
        "ext2", "ext3", "ext4", "xfs", "btrfs", "ntfs", "vfat", "exfat", "zfs",
    ];
    if SUPPORTED.contains(&fs_type) {
        Ok(())
    } else {
        Err(CsiError::InvalidArgument(format!(
            "unsupported fsType: {}",
            fs_type
        )))
    }
}

/// Mount option scrub: kubelet drops options the kernel will reject and
/// flags that conflict with read-only semantics.
pub fn normalize_mount_options(requested: &[String], readonly: bool) -> Vec<String> {
    let mut out: Vec<String> = requested
        .iter()
        .filter(|o| !o.is_empty())
        .map(|o| o.trim().to_string())
        .filter(|o| !(readonly && o == "rw"))
        .collect();
    if readonly && !out.iter().any(|o| o == "ro") {
        out.push("ro".into());
    }
    // Dedup preserving order.
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|o| seen.insert(o.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs_cap(mode: AccessMode) -> VolumeCapability {
        VolumeCapability::fs(mode, "ext4")
    }

    #[test]
    fn stage_then_publish_then_unpublish_then_unstage_happy_path() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/stage/v1",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            1024,
        )
        .unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::Staged);

        m.node_publish_volume("v1", "/stage/v1", "/pods/p1/v1", "p1", false)
            .unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::Published);
        assert_eq!(m.published_count("v1"), 1);

        m.node_unpublish_volume("v1", "/pods/p1/v1").unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::Staged);

        m.node_unstage_volume("v1", "/stage/v1").unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::NotStaged);
    }

    #[test]
    fn stage_is_idempotent_with_same_args() {
        let m = VolumeManager::new();
        let cap = fs_cap(AccessMode::ReadWriteOnce);
        m.node_stage_volume("v1", "/s", cap.clone(), BTreeMap::new(), false, 1024)
            .unwrap();
        m.node_stage_volume("v1", "/s", cap, BTreeMap::new(), false, 1024)
            .unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::Staged);
    }

    #[test]
    fn stage_idempotency_rejects_different_path() {
        let m = VolumeManager::new();
        let cap = fs_cap(AccessMode::ReadWriteOnce);
        m.node_stage_volume("v1", "/s", cap.clone(), BTreeMap::new(), false, 0)
            .unwrap();
        let err = m
            .node_stage_volume("v1", "/other", cap, BTreeMap::new(), false, 0)
            .unwrap_err();
        assert!(matches!(err, CsiError::AlreadyExists(_)));
    }

    #[test]
    fn stage_idempotency_rejects_different_capability() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        let err = m
            .node_stage_volume(
                "v1",
                "/s",
                fs_cap(AccessMode::ReadWriteMany),
                BTreeMap::new(),
                false,
                0,
            )
            .unwrap_err();
        assert!(matches!(err, CsiError::AlreadyExists(_)));
    }

    #[test]
    fn stage_rejects_empty_volume_id() {
        let m = VolumeManager::new();
        let err = m
            .node_stage_volume(
                "",
                "/s",
                fs_cap(AccessMode::ReadWriteOnce),
                BTreeMap::new(),
                false,
                0,
            )
            .unwrap_err();
        assert!(matches!(err, CsiError::InvalidArgument(_)));
    }

    #[test]
    fn stage_rejects_empty_staging_path() {
        let m = VolumeManager::new();
        let err = m
            .node_stage_volume(
                "v1",
                "",
                fs_cap(AccessMode::ReadWriteOnce),
                BTreeMap::new(),
                false,
                0,
            )
            .unwrap_err();
        assert!(matches!(err, CsiError::InvalidArgument(_)));
    }

    #[test]
    fn stage_filesystem_mode_requires_fs_type() {
        let m = VolumeManager::new();
        let cap = VolumeCapability {
            access_mode: AccessMode::ReadWriteOnce,
            volume_mode: VolumeMode::Filesystem,
            fs_type: None,
            mount_options: vec![],
        };
        let err = m
            .node_stage_volume("v1", "/s", cap, BTreeMap::new(), false, 0)
            .unwrap_err();
        assert!(matches!(err, CsiError::InvalidArgument(_)));
    }

    #[test]
    fn stage_block_mode_does_not_require_fs_type() {
        let m = VolumeManager::new();
        let cap = VolumeCapability::block(AccessMode::ReadWriteOnce);
        m.node_stage_volume("v1", "/s", cap, BTreeMap::new(), false, 0)
            .unwrap();
        assert_eq!(m.stage_state("v1"), VolumeStage::Staged);
    }

    #[test]
    fn publish_requires_prior_stage() {
        let m = VolumeManager::new();
        let err = m
            .node_publish_volume("v1", "/s", "/t", "p1", false)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn publish_rejects_staging_path_mismatch() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        let err = m
            .node_publish_volume("v1", "/wrong", "/t", "p1", false)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn publish_is_idempotent_for_same_pod_target() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteMany),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t", "p1", false)
            .unwrap();
        m.node_publish_volume("v1", "/s", "/t", "p1", false)
            .unwrap();
        assert_eq!(m.published_count("v1"), 1);
    }

    #[test]
    fn publish_idempotency_detects_readonly_mismatch() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t", "p1", false)
            .unwrap();
        let err = m
            .node_publish_volume("v1", "/s", "/t", "p1", true)
            .unwrap_err();
        assert!(matches!(err, CsiError::AlreadyExists(_)));
    }

    #[test]
    fn rwop_rejects_second_pod_on_same_node() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOncePod),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "podA", false)
            .unwrap();
        let err = m
            .node_publish_volume("v1", "/s", "/t2", "podB", false)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn rwop_releases_holder_after_last_unpublish() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOncePod),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "podA", false)
            .unwrap();
        m.node_unpublish_volume("v1", "/t1").unwrap();
        m.node_publish_volume("v1", "/s", "/t2", "podB", false)
            .unwrap();
        assert_eq!(m.published_count("v1"), 1);
    }

    #[test]
    fn rwx_allows_multiple_pods() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteMany),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "podA", false)
            .unwrap();
        m.node_publish_volume("v1", "/s", "/t2", "podB", false)
            .unwrap();
        m.node_publish_volume("v1", "/s", "/t3", "podC", false)
            .unwrap();
        assert_eq!(m.published_count("v1"), 3);
    }

    #[test]
    fn unpublish_idempotent_when_volume_unknown() {
        let m = VolumeManager::new();
        m.node_unpublish_volume("ghost", "/t").unwrap();
    }

    #[test]
    fn unpublish_removes_only_matching_target() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteMany),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "podA", false)
            .unwrap();
        m.node_publish_volume("v1", "/s", "/t2", "podB", false)
            .unwrap();
        m.node_unpublish_volume("v1", "/t1").unwrap();
        assert_eq!(m.published_count("v1"), 1);
    }

    #[test]
    fn unstage_blocked_when_published_targets_remain() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t", "p1", false)
            .unwrap();
        let err = m.node_unstage_volume("v1", "/s").unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn unstage_rejects_path_mismatch() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        let err = m.node_unstage_volume("v1", "/wrong").unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn unstage_is_idempotent_when_volume_missing() {
        let m = VolumeManager::new();
        m.node_unstage_volume("ghost", "/s").unwrap();
    }

    #[test]
    fn read_only_volume_forces_publish_readonly() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadOnlyMany),
            BTreeMap::new(),
            true,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "p1", false)
            .unwrap();
        let v = m.staged.get("v1").unwrap();
        assert!(v.published_targets[0].readonly);
    }

    #[test]
    fn rox_access_mode_forces_readonly_even_when_publish_says_rw() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadOnlyMany),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "p1", false)
            .unwrap();
        let v = m.staged.get("v1").unwrap();
        assert!(v.published_targets[0].readonly);
    }

    #[test]
    fn fs_group_policy_none_skips_application() {
        assert!(!should_apply_fs_group(
            FsGroupPolicy::None,
            AccessMode::ReadWriteOnce,
            Some("ext4")
        ));
    }

    #[test]
    fn fs_group_policy_file_always_applies() {
        assert!(should_apply_fs_group(
            FsGroupPolicy::File,
            AccessMode::ReadWriteOnce,
            Some("ext4")
        ));
        assert!(should_apply_fs_group(
            FsGroupPolicy::File,
            AccessMode::ReadWriteMany,
            Some("ext4")
        ));
        assert!(should_apply_fs_group(
            FsGroupPolicy::File,
            AccessMode::ReadOnlyMany,
            None
        ));
    }

    #[test]
    fn fs_group_policy_rwo_with_fstype_only_applies_for_rwo_fs() {
        let p = FsGroupPolicy::ReadWriteOnceWithFSType;
        assert!(should_apply_fs_group(
            p,
            AccessMode::ReadWriteOnce,
            Some("ext4")
        ));
        assert!(!should_apply_fs_group(
            p,
            AccessMode::ReadWriteMany,
            Some("ext4")
        ));
        assert!(!should_apply_fs_group(p, AccessMode::ReadWriteOnce, None));
        assert!(!should_apply_fs_group(
            p,
            AccessMode::ReadWriteOnce,
            Some("")
        ));
    }

    #[test]
    fn access_mode_multi_node_query() {
        assert!(!AccessMode::ReadWriteOnce.allows_multi_node());
        assert!(!AccessMode::ReadWriteOncePod.allows_multi_node());
        assert!(AccessMode::ReadWriteMany.allows_multi_node());
        assert!(AccessMode::ReadOnlyMany.allows_multi_node());
    }

    #[test]
    fn access_mode_multi_pod_query() {
        assert!(AccessMode::ReadWriteOnce.allows_multi_pod());
        assert!(AccessMode::ReadWriteMany.allows_multi_pod());
        assert!(AccessMode::ReadOnlyMany.allows_multi_pod());
        assert!(!AccessMode::ReadWriteOncePod.allows_multi_pod());
    }

    #[test]
    fn validate_access_mode_set_rejects_empty() {
        assert!(validate_access_mode_set(&[]).is_err());
    }

    #[test]
    fn validate_access_mode_set_rejects_rwop_with_others() {
        assert!(validate_access_mode_set(&[
            AccessMode::ReadWriteOncePod,
            AccessMode::ReadWriteOnce
        ])
        .is_err());
    }

    #[test]
    fn validate_access_mode_set_accepts_rwop_alone() {
        assert!(validate_access_mode_set(&[AccessMode::ReadWriteOncePod]).is_ok());
    }

    #[test]
    fn validate_access_mode_set_accepts_combined_non_rwop() {
        assert!(
            validate_access_mode_set(&[AccessMode::ReadOnlyMany, AccessMode::ReadWriteMany])
                .is_ok()
        );
    }

    #[test]
    fn inline_ephemeral_publish_without_stage() {
        let m = VolumeManager::new();
        m.publish_inline_ephemeral(
            "podA",
            "tmp",
            "csi.example.com",
            BTreeMap::new(),
            Some("tmpfs".into()),
            "/pods/podA/tmp",
        )
        .unwrap();
        assert!(m.inline_ephemeral.contains_key("podA/tmp"));
    }

    #[test]
    fn inline_ephemeral_publish_idempotent() {
        let m = VolumeManager::new();
        m.publish_inline_ephemeral("podA", "tmp", "csi", BTreeMap::new(), None, "/t")
            .unwrap();
        m.publish_inline_ephemeral("podA", "tmp", "csi", BTreeMap::new(), None, "/t")
            .unwrap();
        assert_eq!(m.inline_ephemeral.len(), 1);
    }

    #[test]
    fn inline_ephemeral_unpublish_clears_state() {
        let m = VolumeManager::new();
        m.publish_inline_ephemeral("podA", "tmp", "csi", BTreeMap::new(), None, "/t")
            .unwrap();
        m.unpublish_inline_ephemeral("podA", "tmp").unwrap();
        assert!(!m.inline_ephemeral.contains_key("podA/tmp"));
    }

    #[test]
    fn create_snapshot_requires_staged_source() {
        let m = VolumeManager::new();
        let err = m.create_snapshot("snap1", "ghost").unwrap_err();
        assert!(matches!(err, CsiError::NotFound(_)));
    }

    #[test]
    fn create_snapshot_records_size_from_source() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            4096,
        )
        .unwrap();
        let snap = m.create_snapshot("snap1", "v1").unwrap();
        assert_eq!(snap.size_bytes, 4096);
        assert!(snap.ready_to_use);
    }

    #[test]
    fn create_snapshot_idempotent_for_same_source() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            1,
        )
        .unwrap();
        let s1 = m.create_snapshot("snap1", "v1").unwrap();
        let s2 = m.create_snapshot("snap1", "v1").unwrap();
        assert_eq!(s1.snapshot_id, s2.snapshot_id);
    }

    #[test]
    fn create_snapshot_rejects_collision_with_different_source() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            1,
        )
        .unwrap();
        m.node_stage_volume(
            "v2",
            "/s2",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            2,
        )
        .unwrap();
        m.create_snapshot("snap1", "v1").unwrap();
        let err = m.create_snapshot("snap1", "v2").unwrap_err();
        assert!(matches!(err, CsiError::AlreadyExists(_)));
    }

    #[test]
    fn delete_snapshot_idempotent() {
        let m = VolumeManager::new();
        m.delete_snapshot("nope").unwrap();
    }

    #[test]
    fn restore_snapshot_creates_new_volume_with_size() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            8192,
        )
        .unwrap();
        m.create_snapshot("snap1", "v1").unwrap();
        m.restore_snapshot("snap1", "v2", "/s2", fs_cap(AccessMode::ReadWriteOnce))
            .unwrap();
        assert_eq!(m.staged.get("v2").unwrap().size_bytes, 8192);
    }

    #[test]
    fn restore_snapshot_rejects_unknown() {
        let m = VolumeManager::new();
        let err = m
            .restore_snapshot("ghost", "v2", "/s", fs_cap(AccessMode::ReadWriteOnce))
            .unwrap_err();
        assert!(matches!(err, CsiError::NotFound(_)));
    }

    #[test]
    fn restore_snapshot_rejects_not_ready() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            1,
        )
        .unwrap();
        m.create_snapshot("snap1", "v1").unwrap();
        m.snapshots.get_mut("snap1").unwrap().ready_to_use = false;
        let err = m
            .restore_snapshot("snap1", "v2", "/s2", fs_cap(AccessMode::ReadWriteOnce))
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn record_attachment_rejects_rwo_dual_node() {
        let m = VolumeManager::new();
        m.record_attachment("v1", "nodeA", AccessMode::ReadWriteOnce)
            .unwrap();
        let err = m
            .record_attachment("v1", "nodeB", AccessMode::ReadWriteOnce)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn record_attachment_allows_rwx_dual_node() {
        let m = VolumeManager::new();
        m.record_attachment("v1", "nodeA", AccessMode::ReadWriteMany)
            .unwrap();
        m.record_attachment("v1", "nodeB", AccessMode::ReadWriteMany)
            .unwrap();
    }

    #[test]
    fn record_attachment_allows_rox_dual_node() {
        let m = VolumeManager::new();
        m.record_attachment("v1", "nodeA", AccessMode::ReadOnlyMany)
            .unwrap();
        m.record_attachment("v1", "nodeB", AccessMode::ReadOnlyMany)
            .unwrap();
    }

    #[test]
    fn record_attachment_rejects_rwop_dual_node() {
        let m = VolumeManager::new();
        m.record_attachment("v1", "nodeA", AccessMode::ReadWriteOncePod)
            .unwrap();
        let err = m
            .record_attachment("v1", "nodeB", AccessMode::ReadWriteOncePod)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
    }

    #[test]
    fn detach_clears_attachment_record() {
        let m = VolumeManager::new();
        m.record_attachment("v1", "nodeA", AccessMode::ReadWriteOnce)
            .unwrap();
        m.detach("v1", "nodeA");
        m.record_attachment("v1", "nodeB", AccessMode::ReadWriteOnce)
            .unwrap();
    }

    #[test]
    fn driver_registration_records_capabilities() {
        let m = VolumeManager::new();
        m.register_driver(
            "ebs.csi",
            FsGroupPolicy::File,
            vec![AccessMode::ReadWriteOnce, AccessMode::ReadWriteOncePod],
        );
        assert!(m.driver_supports_access_mode("ebs.csi", AccessMode::ReadWriteOnce));
        assert!(!m.driver_supports_access_mode("ebs.csi", AccessMode::ReadWriteMany));
    }

    #[test]
    fn driver_supports_unknown_returns_false() {
        let m = VolumeManager::new();
        assert!(!m.driver_supports_access_mode("nope", AccessMode::ReadWriteOnce));
    }

    #[test]
    fn validate_fs_type_supported() {
        validate_fs_type("ext4").unwrap();
        validate_fs_type("xfs").unwrap();
        validate_fs_type("btrfs").unwrap();
    }

    #[test]
    fn validate_fs_type_unsupported() {
        assert!(validate_fs_type("dragonfly").is_err());
    }

    #[test]
    fn normalize_mount_options_dedups() {
        let opts = vec!["nodev".into(), "nodev".into(), "nosuid".into()];
        let out = normalize_mount_options(&opts, false);
        assert_eq!(out, vec!["nodev".to_string(), "nosuid".into()]);
    }

    #[test]
    fn normalize_mount_options_strips_rw_when_readonly() {
        let opts = vec!["rw".into(), "noatime".into()];
        let out = normalize_mount_options(&opts, true);
        assert!(!out.contains(&"rw".to_string()));
        assert!(out.contains(&"ro".to_string()));
    }

    #[test]
    fn normalize_mount_options_adds_ro_once() {
        let opts = vec!["ro".into(), "ro".into()];
        let out = normalize_mount_options(&opts, true);
        assert_eq!(out.iter().filter(|o| o.as_str() == "ro").count(), 1);
    }

    #[test]
    fn normalize_mount_options_filters_empty() {
        let opts = vec!["".into(), "noatime".into()];
        let out = normalize_mount_options(&opts, false);
        assert_eq!(out, vec!["noatime".to_string()]);
    }

    #[test]
    fn capability_with_mount_options_builder() {
        let cap = VolumeCapability::fs(AccessMode::ReadWriteOnce, "ext4")
            .with_mount_options(vec!["noatime".into()]);
        assert_eq!(cap.mount_options, vec!["noatime".to_string()]);
    }

    #[test]
    fn block_capability_has_no_fs_type() {
        let cap = VolumeCapability::block(AccessMode::ReadWriteOnce);
        assert_eq!(cap.fs_type, None);
        assert_eq!(cap.volume_mode, VolumeMode::Block);
    }

    #[test]
    fn published_count_zero_for_unstaged() {
        let m = VolumeManager::new();
        assert_eq!(m.published_count("ghost"), 0);
    }

    #[test]
    fn snapshot_with_zero_size_is_valid() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        let snap = m.create_snapshot("snap1", "v1").unwrap();
        assert_eq!(snap.size_bytes, 0);
    }

    #[test]
    fn unstage_then_restage_yields_new_record() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            1,
        )
        .unwrap();
        m.node_unstage_volume("v1", "/s").unwrap();
        m.node_stage_volume(
            "v1",
            "/s2",
            fs_cap(AccessMode::ReadWriteOnce),
            BTreeMap::new(),
            false,
            2,
        )
        .unwrap();
        assert_eq!(m.staged.get("v1").unwrap().staging_target_path, "/s2");
    }

    #[test]
    fn publish_records_pod_uid_on_mount() {
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteMany),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t", "podZ", false)
            .unwrap();
        let v = m.staged.get("v1").unwrap();
        assert_eq!(v.published_targets[0].pod_uid, "podZ");
    }

    #[test]
    fn unpublish_releases_rwop_holder_only_when_no_remaining_targets() {
        // Same pod with two targets — RWOP allowed for one pod across multiple mounts.
        let m = VolumeManager::new();
        m.node_stage_volume(
            "v1",
            "/s",
            fs_cap(AccessMode::ReadWriteOncePod),
            BTreeMap::new(),
            false,
            0,
        )
        .unwrap();
        m.node_publish_volume("v1", "/s", "/t1", "podA", false)
            .unwrap();
        m.node_publish_volume("v1", "/s", "/t2", "podA", false)
            .unwrap();
        m.node_unpublish_volume("v1", "/t1").unwrap();
        // Holder should still be podA.
        let err = m
            .node_publish_volume("v1", "/s", "/t3", "podB", false)
            .unwrap_err();
        assert!(matches!(err, CsiError::FailedPrecondition(_)));
        // Release after the last target is gone.
        m.node_unpublish_volume("v1", "/t2").unwrap();
        m.node_publish_volume("v1", "/s", "/t3", "podB", false)
            .unwrap();
    }
}
