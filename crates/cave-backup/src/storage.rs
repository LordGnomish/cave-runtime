//! In-memory backup store (production would persist to PostgreSQL / object storage).

use crate::types::{Backup, BackupId, BackupSchedule, DownloadRequest, RestoreJob};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct BackupStore {
    pub backups: RwLock<Vec<Backup>>,
    pub restores: RwLock<Vec<RestoreJob>>,
    pub schedules: RwLock<Vec<BackupSchedule>>,
    pub downloads: RwLock<Vec<DownloadRequest>>,
}

impl BackupStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Backups ───────────────────────────────────────────────────────────────

    pub async fn insert_backup(&self, backup: Backup) {
        self.backups.write().await.push(backup);
    }

    pub async fn list_backups(&self) -> Vec<Backup> {
        self.backups.read().await.clone()
    }

    pub async fn get_backup(&self, id: BackupId) -> Option<Backup> {
        self.backups
            .read()
            .await
            .iter()
            .find(|b| b.id == id)
            .cloned()
    }

    pub async fn delete_backup(&self, id: BackupId) -> bool {
        let mut backups = self.backups.write().await;
        let before = backups.len();
        backups.retain(|b| b.id != id);
        backups.len() < before
    }

    // ── Restores ──────────────────────────────────────────────────────────────

    pub async fn insert_restore(&self, restore: RestoreJob) {
        self.restores.write().await.push(restore);
    }

    pub async fn list_restores(&self) -> Vec<RestoreJob> {
        self.restores.read().await.clone()
    }

    pub async fn get_restore(&self, id: Uuid) -> Option<RestoreJob> {
        self.restores
            .read()
            .await
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    // ── Schedules ─────────────────────────────────────────────────────────────

    pub async fn insert_schedule(&self, schedule: BackupSchedule) {
        self.schedules.write().await.push(schedule);
    }

    pub async fn list_schedules(&self) -> Vec<BackupSchedule> {
        self.schedules.read().await.clone()
    }

    pub async fn get_schedule(&self, id: Uuid) -> Option<BackupSchedule> {
        self.schedules
            .read()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    pub async fn update_schedule(&self, updated: BackupSchedule) -> bool {
        let mut schedules = self.schedules.write().await;
        if let Some(s) = schedules.iter_mut().find(|s| s.id == updated.id) {
            *s = updated;
            true
        } else {
            false
        }
    }

    pub async fn delete_schedule(&self, id: Uuid) -> bool {
        let mut schedules = self.schedules.write().await;
        let before = schedules.len();
        schedules.retain(|s| s.id != id);
        schedules.len() < before
    }

    // ── Download Requests ─────────────────────────────────────────────────────

    pub async fn insert_download(&self, req: DownloadRequest) {
        self.downloads.write().await.push(req);
    }

    pub async fn get_download(&self, id: Uuid) -> Option<DownloadRequest> {
        self.downloads
            .read()
            .await
            .iter()
            .find(|d| d.id == id)
            .cloned()
    }
}
