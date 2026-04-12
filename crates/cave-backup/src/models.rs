//! Data models for cave-backup.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupPhase {
    New,
    FailedValidation,
    InProgress,
    Completed,
    PartiallyFailed,
    Failed,
    Deleting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub id: Uuid,
    pub name: String,
    pub phase: BackupPhase,
    pub storage_location: String,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
    pub label_selector: HashMap<String, String>,
    pub include_cluster_resources: bool,
    pub ttl_hours: u64,
    pub hooks: Vec<BackupHook>,
    pub volume_snapshot_locations: Vec<String>,
    pub default_volumes_to_fs_backup: bool,
    pub snapshot_move_data: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub items_backed_up: u64,
    pub total_items: u64,
    pub warnings: u64,
    pub errors: u64,
    pub size_bytes: u64,
    pub logs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHook {
    pub name: String,
    pub namespace_selector: Option<String>,
    pub resource_selector: Option<String>,
    pub pre: Vec<ExecHook>,
    pub post: Vec<ExecHook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecHook {
    pub container: String,
    pub command: Vec<String>,
    pub on_error: HookErrorMode,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookErrorMode {
    Continue,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestorePhase {
    New,
    FailedValidation,
    InProgress,
    Completed,
    PartiallyFailed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Restore {
    pub id: Uuid,
    pub name: String,
    pub backup_name: String,
    pub backup_id: Uuid,
    pub phase: RestorePhase,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
    pub namespace_mappings: HashMap<String, String>,
    pub restore_pvs: bool,
    pub preserve_node_ports: bool,
    pub hooks: Vec<RestoreHook>,
    pub existing_resource_policy: ExistingResourcePolicy,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub warnings: u64,
    pub errors: u64,
    pub logs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExistingResourcePolicy {
    None,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreHook {
    pub name: String,
    pub resource_selector: Option<String>,
    pub init_containers: Vec<ExecHook>,
    pub post_hooks: Vec<ExecHook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub name: String,
    pub cron_expression: String,
    pub template: BackupSpec,
    pub paused: bool,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub last_backup_phase: Option<BackupPhase>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackupSpec {
    pub storage_location: String,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
    pub label_selector: HashMap<String, String>,
    pub include_cluster_resources: bool,
    pub ttl_hours: u64,
    pub hooks: Vec<BackupHook>,
    pub default_volumes_to_fs_backup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageProvider {
    S3,
    Gcs,
    Azure,
    Filesystem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStorageLocation {
    pub id: Uuid,
    pub name: String,
    pub provider: StorageProvider,
    pub bucket: String,
    pub prefix: Option<String>,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub access_mode: BslAccessMode,
    pub credential_secret: Option<String>,
    pub ca_bundle: Option<String>,
    pub insecure_skip_tls_verify: bool,
    pub is_default: bool,
    pub phase: BslPhase,
    pub last_validated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BslAccessMode {
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BslPhase {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSnapshotLocation {
    pub id: Uuid,
    pub name: String,
    pub provider: StorageProvider,
    pub region: Option<String>,
    pub credential_secret: Option<String>,
    pub config: HashMap<String, String>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FsBackupMethod {
    Restic,
    Kopia,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsBackupJob {
    pub id: Uuid,
    pub backup_id: Uuid,
    pub method: FsBackupMethod,
    pub namespace: String,
    pub pod: String,
    pub volume: String,
    pub phase: BackupPhase,
    pub snapshot_id: Option<String>,
    pub size_bytes: u64,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub phase: ServerPhase,
    pub plugins: Vec<PluginInfo>,
    pub storage_location_count: usize,
    pub server_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerPhase {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub kind: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_phase_serializes_snake_case() {
        let phase = BackupPhase::InProgress;
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, "\"in_progress\"");

        let phase2: BackupPhase = serde_json::from_str("\"partially_failed\"").unwrap();
        assert_eq!(phase2, BackupPhase::PartiallyFailed);
    }

    #[test]
    fn restore_phase_round_trip() {
        let phase = RestorePhase::FailedValidation;
        let json = serde_json::to_string(&phase).unwrap();
        let back: RestorePhase = serde_json::from_str(&json).unwrap();
        assert_eq!(phase, back);
    }

    #[test]
    fn bsl_round_trip() {
        let bsl = BackupStorageLocation {
            id: Uuid::new_v4(),
            name: "default".into(),
            provider: StorageProvider::S3,
            bucket: "cave-backups".into(),
            prefix: None,
            region: Some("us-east-1".into()),
            endpoint: None,
            access_mode: BslAccessMode::ReadWrite,
            credential_secret: None,
            ca_bundle: None,
            insecure_skip_tls_verify: false,
            is_default: true,
            phase: BslPhase::Available,
            last_validated_at: None,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&bsl).unwrap();
        let back: BackupStorageLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(bsl.name, back.name);
        assert_eq!(bsl.bucket, back.bucket);
        assert_eq!(bsl.phase, back.phase);
    }

    #[test]
    fn existing_resource_policy_serializes() {
        let policy = ExistingResourcePolicy::None;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"none\"");
    }

    #[test]
    fn hook_error_mode_serializes() {
        let mode = HookErrorMode::Fail;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"fail\"");
    }

    #[test]
    fn fs_backup_method_round_trip() {
        let method = FsBackupMethod::Kopia;
        let json = serde_json::to_string(&method).unwrap();
        let back: FsBackupMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(method, back);
    }

    #[test]
    fn backup_spec_default() {
        let spec = BackupSpec::default();
        assert!(spec.included_namespaces.is_empty());
        assert_eq!(spec.ttl_hours, 0);
        assert!(!spec.include_cluster_resources);
    }
}
