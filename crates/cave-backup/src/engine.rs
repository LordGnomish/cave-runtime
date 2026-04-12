use crate::models::{BackupRecord, BackupSchedule, BackupStatus};
use chrono::Utc;

/// Check if a backup has expired
pub fn is_expired(record: &BackupRecord) -> bool {
    Utc::now() > record.expires_at
}

/// Check if a backup is healthy (completed without failures)
pub fn is_healthy(record: &BackupRecord) -> bool {
    record.status == BackupStatus::Completed && record.warnings.is_empty()
}

/// Calculate backup duration in seconds
pub fn duration_secs(record: &BackupRecord) -> Option<i64> {
    record.completed_at.map(|end| (end - record.started_at).num_seconds())
}

/// Filter enabled schedules
pub fn active_schedules(schedules: &[BackupSchedule]) -> Vec<&BackupSchedule> {
    schedules.iter().filter(|s| s.enabled).collect()
}

/// Total size of all completed backups in bytes
pub fn total_backup_size(records: &[BackupRecord]) -> u64 {
    records.iter()
        .filter(|r| r.status == BackupStatus::Completed)
        .map(|r| r.size_bytes)
        .sum()
}

/// Find the most recent successful backup
pub fn latest_successful<'a>(records: &'a [BackupRecord]) -> Option<&'a BackupRecord> {
    records.iter()
        .filter(|r| r.status == BackupStatus::Completed)
        .max_by_key(|r| r.started_at)
}

/// Validate a cron expression (simple: must have 5 space-separated fields)
pub fn is_valid_cron(expr: &str) -> bool {
    expr.split_whitespace().count() == 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use chrono::{Duration, Utc};

    fn make_record(status: BackupStatus, size_bytes: u64, started_offset_secs: i64) -> BackupRecord {
        let now = Utc::now();
        let started_at = now + Duration::seconds(started_offset_secs);
        BackupRecord {
            id: Uuid::new_v4(),
            schedule_id: None,
            name: "backup-test".to_string(),
            status,
            size_bytes,
            started_at,
            completed_at: Some(started_at + Duration::minutes(5)),
            expires_at: now + Duration::days(30),
            warnings: vec![],
        }
    }

    fn make_schedule(enabled: bool) -> BackupSchedule {
        BackupSchedule {
            id: Uuid::new_v4(),
            name: "schedule-test".to_string(),
            namespace: "default".to_string(),
            cron_expression: "0 2 * * *".to_string(),
            retention_days: 30,
            storage_location: "s3://bucket/backups".to_string(),
            enabled,
            include_volumes: true,
        }
    }

    #[test]
    fn test_is_expired_future() {
        let record = make_record(BackupStatus::Completed, 1024, 0);
        // expires_at is 30 days in the future
        assert!(!is_expired(&record));
    }

    #[test]
    fn test_is_expired_past() {
        let now = Utc::now();
        let mut record = make_record(BackupStatus::Completed, 1024, 0);
        record.expires_at = now - Duration::days(1);
        assert!(is_expired(&record));
    }

    #[test]
    fn test_is_healthy_completed_no_warnings() {
        let record = make_record(BackupStatus::Completed, 1024, 0);
        assert!(is_healthy(&record));
    }

    #[test]
    fn test_is_healthy_with_warnings() {
        let mut record = make_record(BackupStatus::Completed, 1024, 0);
        record.warnings = vec!["Volume pvc-xyz was skipped".to_string()];
        assert!(!is_healthy(&record));
    }

    #[test]
    fn test_is_healthy_failed_status() {
        let record = make_record(BackupStatus::Failed, 0, 0);
        assert!(!is_healthy(&record));
    }

    #[test]
    fn test_active_schedules_filter() {
        let schedules = vec![
            make_schedule(true),
            make_schedule(false),
            make_schedule(true),
        ];
        let active = active_schedules(&schedules);
        assert_eq!(active.len(), 2);
        for s in &active {
            assert!(s.enabled);
        }
    }

    #[test]
    fn test_total_backup_size() {
        let records = vec![
            make_record(BackupStatus::Completed, 100, 0),
            make_record(BackupStatus::Completed, 200, 0),
            make_record(BackupStatus::Failed, 50, 0),
            make_record(BackupStatus::InProgress, 75, 0),
        ];
        // Only completed: 100 + 200 = 300
        assert_eq!(total_backup_size(&records), 300);
    }

    #[test]
    fn test_is_valid_cron() {
        assert!(is_valid_cron("0 2 * * *"));
        assert!(is_valid_cron("*/5 * * * *"));
        assert!(!is_valid_cron("bad"));
        assert!(!is_valid_cron("0 2 * *"));
        assert!(!is_valid_cron("0 2 * * * *"));
    }
}
