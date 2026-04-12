use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupSchedule {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub cron_expression: String,
    pub retention_days: u32,
    pub storage_location: String,
    pub enabled: bool,
    pub include_volumes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupRecord {
    pub id: Uuid,
    pub schedule_id: Option<Uuid>,
    pub name: String,
    pub status: BackupStatus,
    pub size_bytes: u64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    InProgress,
    Completed,
    Failed,
    PartiallyFailed,
    Deleting,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_record(status: BackupStatus) -> BackupRecord {
        let now = Utc::now();
        BackupRecord {
            id: Uuid::new_v4(),
            schedule_id: None,
            name: "backup-001".to_string(),
            status,
            size_bytes: 1024 * 1024 * 500,
            started_at: now,
            completed_at: Some(now + Duration::minutes(5)),
            expires_at: now + Duration::days(30),
            warnings: vec![],
        }
    }

    #[test]
    fn test_backup_status_serialization() {
        let statuses = vec![
            BackupStatus::InProgress,
            BackupStatus::Completed,
            BackupStatus::Failed,
            BackupStatus::PartiallyFailed,
            BackupStatus::Deleting,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: BackupStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_backup_record_roundtrip() {
        let record = make_record(BackupStatus::Completed);
        let json = serde_json::to_string(&record).unwrap();
        let back: BackupRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, back);
    }

    #[test]
    fn test_backup_schedule_roundtrip() {
        let schedule = BackupSchedule {
            id: Uuid::new_v4(),
            name: "nightly".to_string(),
            namespace: "default".to_string(),
            cron_expression: "0 2 * * *".to_string(),
            retention_days: 30,
            storage_location: "s3://my-bucket/backups".to_string(),
            enabled: true,
            include_volumes: true,
        };
        let json = serde_json::to_string(&schedule).unwrap();
        let back: BackupSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(schedule, back);
    }

    #[test]
    fn test_backup_record_with_warnings() {
        let mut record = make_record(BackupStatus::PartiallyFailed);
        record.warnings = vec!["Volume pvc-xyz skipped".to_string()];
        let json = serde_json::to_string(&record).unwrap();
        let back: BackupRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.warnings.len(), 1);
    }

    #[test]
    fn test_backup_status_in_progress_serializes() {
        let json = serde_json::to_string(&BackupStatus::InProgress).unwrap();
        assert_eq!(json, "\"in_progress\"");
    }
}
