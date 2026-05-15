// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! etcd backup and restore per cluster.

use crate::error::{ClusterError, ClusterResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Backup types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BackupStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Deleting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtcdBackup {
    pub id: Uuid,
    pub cluster_name: String,
    pub status: BackupStatus,
    pub size_bytes: Option<u64>,
    /// Storage location (e.g. s3://bucket/prefix/backup.db)
    pub storage_path: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub kubernetes_version: String,
    pub etcd_version: String,
}

impl EtcdBackup {
    pub fn new(cluster_name: String, storage_prefix: &str, k8s_version: &str) -> Self {
        let id = Uuid::new_v4();
        Self {
            id,
            cluster_name: cluster_name.clone(),
            status: BackupStatus::Pending,
            size_bytes: None,
            storage_path: format!("{storage_prefix}/{cluster_name}/{id}.db"),
            created_at: Utc::now(),
            completed_at: None,
            error_message: None,
            kubernetes_version: k8s_version.to_string(),
            etcd_version: etcd_version_for_k8s(k8s_version),
        }
    }
}

fn etcd_version_for_k8s(k8s: &str) -> String {
    // Mapping Kubernetes → bundled etcd version
    match k8s {
        "1.28" => "3.5.9",
        "1.29" => "3.5.10",
        "1.30" => "3.5.12",
        "1.31" => "3.5.15",
        "1.32" => "3.5.16",
        _ => "3.5.0",
    }
    .into()
}

// ── Restore types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RestoreStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtcdRestore {
    pub id: Uuid,
    pub cluster_name: String,
    pub backup_id: Uuid,
    pub status: RestoreStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

// ── Backup store ──────────────────────────────────────────────────────────────

pub struct EtcdBackupStore {
    backups: DashMap<Uuid, EtcdBackup>,
    restores: DashMap<Uuid, EtcdRestore>,
    storage_prefix: String,
}

impl EtcdBackupStore {
    pub fn new(storage_prefix: String) -> Self {
        Self {
            backups: DashMap::new(),
            restores: DashMap::new(),
            storage_prefix,
        }
    }

    // ── Backup operations ─────────────────────────────────────────────────────

    pub fn create_backup(
        &self,
        cluster_name: &str,
        k8s_version: &str,
    ) -> ClusterResult<EtcdBackup> {
        let mut backup = EtcdBackup::new(
            cluster_name.to_string(),
            &self.storage_prefix,
            k8s_version,
        );

        // Simulate backup: transition through Running → Succeeded
        backup.status = BackupStatus::Running;
        backup.status = BackupStatus::Succeeded;
        backup.completed_at = Some(Utc::now());
        backup.size_bytes = Some(52_428_800); // 50 MiB simulated

        let result = backup.clone();
        self.backups.insert(backup.id, backup);
        Ok(result)
    }

    pub fn get_backup(&self, id: Uuid) -> ClusterResult<EtcdBackup> {
        self.backups
            .get(&id)
            .map(|b| b.clone())
            .ok_or_else(|| ClusterError::EtcdBackupFailed(format!("backup {id} not found")))
    }

    pub fn list_backups(&self, cluster_name: &str) -> Vec<EtcdBackup> {
        self.backups
            .iter()
            .filter(|e| e.cluster_name == cluster_name)
            .map(|e| e.clone())
            .collect()
    }

    pub fn delete_backup(&self, id: Uuid) -> ClusterResult<()> {
        self.backups
            .remove(&id)
            .ok_or_else(|| ClusterError::EtcdBackupFailed(format!("backup {id} not found")))?;
        Ok(())
    }

    // ── Restore operations ────────────────────────────────────────────────────

    pub fn restore_from_backup(
        &self,
        cluster_name: &str,
        backup_id: Uuid,
    ) -> ClusterResult<EtcdRestore> {
        // Validate backup exists and belongs to this cluster
        let backup = self.get_backup(backup_id)?;
        if backup.cluster_name != cluster_name {
            return Err(ClusterError::EtcdRestoreFailed(format!(
                "backup {backup_id} belongs to cluster {}, not {cluster_name}",
                backup.cluster_name
            )));
        }
        if backup.status != BackupStatus::Succeeded {
            return Err(ClusterError::EtcdRestoreFailed(format!(
                "backup {backup_id} is not in Succeeded state"
            )));
        }

        let mut restore = EtcdRestore {
            id: Uuid::new_v4(),
            cluster_name: cluster_name.to_string(),
            backup_id,
            status: RestoreStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
            error_message: None,
        };

        // Simulate restore
        restore.status = RestoreStatus::Running;
        restore.status = RestoreStatus::Succeeded;
        restore.completed_at = Some(Utc::now());

        let result = restore.clone();
        self.restores.insert(restore.id, restore);
        Ok(result)
    }

    pub fn list_restores(&self, cluster_name: &str) -> Vec<EtcdRestore> {
        self.restores
            .iter()
            .filter(|e| e.cluster_name == cluster_name)
            .map(|e| e.clone())
            .collect()
    }

    pub fn get_restore(&self, id: Uuid) -> ClusterResult<EtcdRestore> {
        self.restores
            .get(&id)
            .map(|r| r.clone())
            .ok_or_else(|| ClusterError::EtcdRestoreFailed(format!("restore {id} not found")))
    }
}

/// Backup schedule configuration for a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    pub cluster_name: String,
    /// Cron expression (e.g. "0 3 * * *" = 3am daily)
    pub cron_expression: String,
    /// Retention count
    pub retention_count: u32,
    pub enabled: bool,
}

impl BackupSchedule {
    pub fn daily_at_3am(cluster_name: String) -> Self {
        Self {
            cluster_name,
            cron_expression: "0 3 * * *".into(),
            retention_count: 7,
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> EtcdBackupStore {
        EtcdBackupStore::new("s3://cave-backups/etcd".into())
    }

    #[test]
    fn create_and_list_backup() {
        let s = store();
        let backup = s.create_backup("prod", "1.30").unwrap();
        assert_eq!(backup.status, BackupStatus::Succeeded);
        assert!(backup.size_bytes.unwrap() > 0);

        let list = s.list_backups("prod");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, backup.id);
    }

    #[test]
    fn restore_from_backup() {
        let s = store();
        let backup = s.create_backup("prod", "1.30").unwrap();
        let restore = s.restore_from_backup("prod", backup.id).unwrap();
        assert_eq!(restore.status, RestoreStatus::Succeeded);
        assert_eq!(restore.backup_id, backup.id);
    }

    #[test]
    fn restore_wrong_cluster_fails() {
        let s = store();
        let backup = s.create_backup("cluster-a", "1.30").unwrap();
        assert!(s.restore_from_backup("cluster-b", backup.id).is_err());
    }

    #[test]
    fn delete_backup() {
        let s = store();
        let backup = s.create_backup("test", "1.29").unwrap();
        s.delete_backup(backup.id).unwrap();
        assert!(s.get_backup(backup.id).is_err());
    }

    #[test]
    fn etcd_version_mapped_correctly() {
        assert_eq!(etcd_version_for_k8s("1.30"), "3.5.12");
        assert_eq!(etcd_version_for_k8s("1.32"), "3.5.16");
    }
}
