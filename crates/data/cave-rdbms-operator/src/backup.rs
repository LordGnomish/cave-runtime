// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{PgError, PgResult};
use crate::types::{BackupRecord, BackupStatus, BackupType, PitrTarget};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub struct BackupManager {
    backups: Arc<RwLock<HashMap<String, BackupRecord>>>,
    wal_archive_path: String,
}

impl BackupManager {
    pub fn new(wal_archive_path: &str) -> Self {
        BackupManager {
            backups: Arc::new(RwLock::new(HashMap::new())),
            wal_archive_path: wal_archive_path.to_string(),
        }
    }

    pub fn start_backup(
        &self,
        instance_id: &str,
        backup_type: BackupType,
    ) -> PgResult<BackupRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let storage_path = format!("{}/{}/{}", self.wal_archive_path, instance_id, id);

        let record = BackupRecord {
            id: id.clone(),
            instance_id: instance_id.to_string(),
            backup_type,
            status: BackupStatus::Running,
            size_bytes: 0,
            started_at: now,
            completed_at: None,
            storage_path,
            wal_start_lsn: Some("0/1000000".to_string()),
            wal_end_lsn: None,
        };

        let mut backups = self.backups.write().unwrap();
        backups.insert(id, record.clone());
        Ok(record)
    }

    pub fn complete_backup(
        &self,
        backup_id: &str,
        size_bytes: u64,
        wal_end_lsn: &str,
    ) -> PgResult<()> {
        let mut backups = self.backups.write().unwrap();
        let backup = backups
            .get_mut(backup_id)
            .ok_or_else(|| PgError::BackupFailed(format!("backup not found: {}", backup_id)))?;
        backup.status = BackupStatus::Completed;
        backup.size_bytes = size_bytes;
        backup.completed_at = Some(Utc::now());
        backup.wal_end_lsn = Some(wal_end_lsn.to_string());
        Ok(())
    }

    pub fn fail_backup(&self, backup_id: &str) -> PgResult<()> {
        let mut backups = self.backups.write().unwrap();
        let backup = backups
            .get_mut(backup_id)
            .ok_or_else(|| PgError::BackupFailed(format!("backup not found: {}", backup_id)))?;
        backup.status = BackupStatus::Failed;
        backup.completed_at = Some(Utc::now());
        Ok(())
    }

    pub fn get_backup(&self, id: &str) -> PgResult<BackupRecord> {
        let backups = self.backups.read().unwrap();
        backups
            .get(id)
            .cloned()
            .ok_or_else(|| PgError::BackupFailed(format!("backup not found: {}", id)))
    }

    pub fn list_backups(&self, instance_id: &str) -> Vec<BackupRecord> {
        let backups = self.backups.read().unwrap();
        backups
            .values()
            .filter(|b| b.instance_id == instance_id)
            .cloned()
            .collect()
    }

    pub fn delete_backup(&self, id: &str) -> PgResult<()> {
        let mut backups = self.backups.write().unwrap();
        if backups.remove(id).is_none() {
            return Err(PgError::BackupFailed(format!("backup not found: {}", id)));
        }
        Ok(())
    }

    /// Find the best completed backup to use as a base for PITR.
    /// Returns the latest completed backup whose started_at <= target.target_time.
    pub fn find_pitr_base(&self, instance_id: &str, target: &PitrTarget) -> PgResult<BackupRecord> {
        let backups = self.backups.read().unwrap();
        let candidates: Vec<&BackupRecord> = backups
            .values()
            .filter(|b| {
                b.instance_id == instance_id
                    && b.status == BackupStatus::Completed
                    && b.started_at <= target.target_time
            })
            .collect();

        candidates
            .into_iter()
            .max_by_key(|b| b.started_at)
            .cloned()
            .ok_or_else(|| PgError::BackupFailed("no suitable backup found for PITR".to_string()))
    }

    /// Delete backups older than retain_days. Returns count of deleted backups.
    pub fn apply_retention(&self, instance_id: &str, retain_days: u32) -> usize {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retain_days));
        let mut backups = self.backups.write().unwrap();
        let before = backups.len();
        backups.retain(|_, b| b.instance_id != instance_id || b.started_at >= cutoff);
        before - backups.len()
    }

    pub fn wal_segment_path(&self, lsn: &str) -> String {
        format!("{}/wal/{}", self.wal_archive_path, lsn.replace('/', "_"))
    }

    pub fn list_wal_segments(&self, instance_id: &str) -> Vec<String> {
        // In real implementation would scan the WAL archive directory
        // For in-memory tests, return based on backups' LSNs
        let backups = self.backups.read().unwrap();
        backups
            .values()
            .filter(|b| b.instance_id == instance_id)
            .filter_map(|b| b.wal_end_lsn.as_ref())
            .map(|lsn| self.wal_segment_path(lsn))
            .collect()
    }
}
