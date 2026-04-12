//! Domain types for cave-backup.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub type BackupId = Uuid;

// ─── Phase ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BackupPhase {
    New,
    FailedValidation,
    InProgress,
    Completed,
    PartiallyFailed,
    Failed,
    Deleting,
}

impl BackupPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BackupPhase::Completed
                | BackupPhase::PartiallyFailed
                | BackupPhase::Failed
                | BackupPhase::FailedValidation
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RestorePhase {
    New,
    FailedValidation,
    InProgress,
    Completed,
    PartiallyFailed,
    Failed,
}

impl RestorePhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RestorePhase::Completed
                | RestorePhase::PartiallyFailed
                | RestorePhase::Failed
                | RestorePhase::FailedValidation
        )
    }
}

// ─── Scope ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupScope {
    FullCluster,
    Namespace {
        namespaces: Vec<String>,
    },
    LabelSelector {
        selector: String,
    },
    ResourceFilter {
        included_resources: Vec<String>,
        excluded_resources: Vec<String>,
        namespaces: Vec<String>,
    },
}

// ─── Target ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupTarget {
    Local {
        path: String,
    },
    S3 {
        bucket: String,
        prefix: String,
        region: String,
        /// For S3-compatible stores (MinIO, Ceph, etc.).
        endpoint: Option<String>,
    },
    AzureBlob {
        account: String,
        container: String,
        prefix: String,
    },
    Gcs {
        bucket: String,
        prefix: String,
    },
}

// ─── Encryption ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EncryptionAlgorithm {
    Aes256Gcm,
    Aes256Cbc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    pub enabled: bool,
    /// Key ID referencing a secret in cave-secrets / vault.
    pub key_id: Option<String>,
    pub algorithm: EncryptionAlgorithm,
}

impl Default for EncryptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            key_id: None,
            algorithm: EncryptionAlgorithm::Aes256Gcm,
        }
    }
}

// ─── Hooks ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookErrorMode {
    Continue,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecHook {
    pub container: String,
    pub command: Vec<String>,
    pub on_error: HookErrorMode,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHook {
    pub name: String,
    pub pod_selector: String,
    pub namespace: String,
    pub pre_hooks: Vec<ExecHook>,
    pub post_hooks: Vec<ExecHook>,
}

// ─── Volume Snapshots ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileBackupTool {
    Restic,
    Kopia,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshotConfig {
    pub enabled: bool,
    pub snapshot_class: Option<String>,
    pub use_file_backup: bool,
    pub file_backup_tool: FileBackupTool,
}

impl Default for VolumeSnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            snapshot_class: None,
            use_file_backup: false,
            file_backup_tool: FileBackupTool::Restic,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshot {
    pub name: String,
    pub namespace: String,
    pub pvc_name: String,
    pub snapshot_class: String,
    pub creation_time: DateTime<Utc>,
    pub restore_size_bytes: u64,
    pub ready: bool,
}

// ─── Backup ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSpec {
    pub scope: BackupScope,
    pub target: BackupTarget,
    pub encryption: EncryptionConfig,
    pub hooks: Vec<BackupHook>,
    pub volume_snapshot: VolumeSnapshotConfig,
    /// Time-to-live in hours before automatic deletion.
    pub ttl_hours: u64,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub id: BackupId,
    pub name: String,
    pub spec: BackupSpec,
    pub phase: BackupPhase,
    pub start_time: Option<DateTime<Utc>>,
    pub completion_time: Option<DateTime<Utc>>,
    pub size_bytes: u64,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub volume_snapshots: Vec<VolumeSnapshot>,
    pub created_at: DateTime<Utc>,
}

impl Backup {
    pub fn new(name: impl Into<String>, spec: BackupSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            spec,
            phase: BackupPhase::New,
            start_time: None,
            completion_time: None,
            size_bytes: 0,
            warnings: Vec::new(),
            errors: Vec::new(),
            volume_snapshots: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Whether this backup has expired based on its TTL.
    pub fn is_expired(&self) -> bool {
        let ttl = self.spec.ttl_hours;
        if ttl == 0 {
            return false;
        }
        let age = Utc::now() - self.created_at;
        age.num_hours() >= ttl as i64
    }
}

// ─── Restore ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceRemap {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageClassRemap {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreSpec {
    pub backup_id: BackupId,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
    pub label_selector: Option<String>,
    pub namespace_remaps: Vec<NamespaceRemap>,
    pub storage_class_remaps: Vec<StorageClassRemap>,
    pub restore_pvs: bool,
    /// For cross-cluster DR: name of a secret containing target kubeconfig.
    pub target_cluster_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreJob {
    pub id: Uuid,
    pub name: String,
    pub spec: RestoreSpec,
    pub phase: RestorePhase,
    pub start_time: Option<DateTime<Utc>>,
    pub completion_time: Option<DateTime<Utc>>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl RestoreJob {
    pub fn new(name: impl Into<String>, spec: RestoreSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            spec,
            phase: RestorePhase::New,
            start_time: None,
            completion_time: None,
            warnings: Vec::new(),
            errors: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

// ─── Schedule ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_backups: Option<u32>,
    pub ttl_hours: Option<u64>,
}

impl RetentionPolicy {
    pub fn new_max(max: u32) -> Self {
        Self {
            max_backups: Some(max),
            ttl_hours: None,
        }
    }

    pub fn new_ttl(hours: u64) -> Self {
        Self {
            max_backups: None,
            ttl_hours: Some(hours),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    pub id: Uuid,
    pub name: String,
    pub cron_expression: String,
    pub backup_spec: BackupSpec,
    pub retention: RetentionPolicy,
    pub paused: bool,
    pub last_backup_id: Option<BackupId>,
    pub last_run_time: Option<DateTime<Utc>>,
    pub next_run_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl BackupSchedule {
    pub fn new(
        name: impl Into<String>,
        cron_expression: impl Into<String>,
        backup_spec: BackupSpec,
        retention: RetentionPolicy,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            cron_expression: cron_expression.into(),
            backup_spec,
            retention,
            paused: false,
            last_backup_id: None,
            last_run_time: None,
            next_run_time: None,
            created_at: Utc::now(),
        }
    }
}

// ─── Download Request ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub id: Uuid,
    pub backup_id: BackupId,
    pub download_url: Option<String>,
    pub ttl_seconds: u64,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl DownloadRequest {
    pub fn new(backup_id: BackupId, ttl_seconds: u64) -> Self {
        let now = Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_seconds as i64);
        Self {
            id: Uuid::new_v4(),
            backup_id,
            download_url: None,
            ttl_seconds,
            created_at: now,
            expires_at: Some(expires),
        }
    }

    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() > exp,
            None => false,
        }
    }
}
