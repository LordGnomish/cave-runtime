<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/thirsty-lederberg
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
<<<<<<< HEAD
=======
//! Core backup/restore engine logic.
//!
//! Pure functions for phase transitions, validation, and retention enforcement.

use crate::types::{Backup, BackupPhase, RetentionPolicy};

/// Validate a backup name: non-empty, alphanumeric + hyphens, ≤ 63 chars.
pub fn validate_backup_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.ends_with(|c: char| c.is_ascii_alphanumeric())
}

/// Transition a backup to a new phase, enforcing allowed transitions.
/// Returns `true` if the transition was applied.
pub fn transition_phase(backup: &mut Backup, new_phase: BackupPhase) -> bool {
    use BackupPhase::*;
    let allowed = matches!(
        (&backup.phase, &new_phase),
        (New, InProgress)
            | (New, FailedValidation)
            | (InProgress, Completed)
            | (InProgress, PartiallyFailed)
            | (InProgress, Failed)
            | (Completed, Deleting)
            | (PartiallyFailed, Deleting)
            | (Failed, Deleting)
    );
    if allowed {
        backup.phase = new_phase;
        true
    } else {
        false
    }
}

/// Apply a retention policy to a list of completed backups.
/// Backups are sorted oldest-first; the ones that should be deleted are returned.
pub fn apply_retention<'a>(
    backups: &'a [Backup],
    policy: &RetentionPolicy,
) -> Vec<&'a Backup> {
    let mut completed: Vec<&Backup> = backups
        .iter()
        .filter(|b| b.phase == BackupPhase::Completed)
        .collect();

    // Oldest first.
    completed.sort_by_key(|b| b.created_at);

    let mut to_delete: Vec<&Backup> = Vec::new();

    // TTL-based deletion.
    if let Some(ttl_hours) = policy.ttl_hours {
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(ttl_hours as i64);
        for b in &completed {
            if b.created_at < cutoff {
                to_delete.push(b);
            }
        }
    }

    // Count-based deletion (keep newest N).
    if let Some(max) = policy.max_backups {
        let max = max as usize;
        if completed.len() > max {
            let excess = completed.len() - max;
            for b in completed.iter().take(excess) {
                if !to_delete.iter().any(|d| d.id == b.id) {
                    to_delete.push(b);
                }
            }
        }
    }

    to_delete
>>>>>>> claude/gallant-meninsky
=======
>>>>>>> claude/thirsty-lederberg
}

#[cfg(test)]
mod tests {
    use super::*;
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/thirsty-lederberg
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
<<<<<<< HEAD
=======
    use crate::types::{
        BackupScope, BackupSpec, BackupTarget, EncryptionConfig, RetentionPolicy,
        VolumeSnapshotConfig,
    };
    use std::collections::HashMap;

    fn minimal_spec() -> BackupSpec {
        BackupSpec {
            scope: BackupScope::FullCluster,
            target: BackupTarget::Local {
                path: "/tmp/backup".into(),
            },
            encryption: EncryptionConfig::default(),
            hooks: vec![],
            volume_snapshot: VolumeSnapshotConfig::default(),
            ttl_hours: 0,
            labels: HashMap::new(),
        }
    }

    fn completed_backup(name: &str, age_hours: i64) -> Backup {
        let mut b = Backup::new(name, minimal_spec());
        b.phase = BackupPhase::Completed;
        b.created_at = chrono::Utc::now() - chrono::Duration::hours(age_hours);
        b
    }

    #[test]
    fn test_validate_backup_name_valid() {
        assert!(validate_backup_name("daily-backup"));
        assert!(validate_backup_name("backup1"));
        assert!(validate_backup_name("a"));
    }

    #[test]
    fn test_validate_backup_name_invalid_empty() {
        assert!(!validate_backup_name(""));
    }

    #[test]
    fn test_validate_backup_name_invalid_chars() {
        assert!(!validate_backup_name("backup_with_underscores"));
        assert!(!validate_backup_name("-starts-with-dash"));
        assert!(!validate_backup_name("ends-with-dash-"));
        assert!(!validate_backup_name("has spaces"));
    }

    #[test]
    fn test_validate_backup_name_too_long() {
        let long = "a".repeat(64);
        assert!(!validate_backup_name(&long));
        let ok = "a".repeat(63);
        assert!(validate_backup_name(&ok));
    }

    #[test]
    fn test_transition_new_to_in_progress() {
        let mut b = Backup::new("test", minimal_spec());
        assert_eq!(b.phase, BackupPhase::New);
        assert!(transition_phase(&mut b, BackupPhase::InProgress));
        assert_eq!(b.phase, BackupPhase::InProgress);
    }

    #[test]
    fn test_transition_in_progress_to_completed() {
        let mut b = Backup::new("test", minimal_spec());
        transition_phase(&mut b, BackupPhase::InProgress);
        assert!(transition_phase(&mut b, BackupPhase::Completed));
        assert_eq!(b.phase, BackupPhase::Completed);
    }

    #[test]
    fn test_transition_invalid_skipped() {
        let mut b = Backup::new("test", minimal_spec());
        // Can't go New → Completed directly.
        assert!(!transition_phase(&mut b, BackupPhase::Completed));
        assert_eq!(b.phase, BackupPhase::New);
    }

    #[test]
    fn test_backup_phase_is_terminal() {
        assert!(BackupPhase::Completed.is_terminal());
        assert!(BackupPhase::Failed.is_terminal());
        assert!(BackupPhase::FailedValidation.is_terminal());
        assert!(!BackupPhase::InProgress.is_terminal());
        assert!(!BackupPhase::New.is_terminal());
    }

    #[test]
    fn test_apply_retention_max_backups_removes_oldest() {
        let backups = vec![
            completed_backup("b1", 10),
            completed_backup("b2", 5),
            completed_backup("b3", 1),
        ];
        let policy = RetentionPolicy::new_max(2);
        let to_delete = apply_retention(&backups, &policy);
        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0].name, "b1");
    }

    #[test]
    fn test_apply_retention_ttl_removes_expired() {
        let backups = vec![
            completed_backup("old", 50),
            completed_backup("recent", 2),
        ];
        // TTL = 24 hours; "old" is 50h old, should be deleted.
        let policy = RetentionPolicy::new_ttl(24);
        let to_delete = apply_retention(&backups, &policy);
        assert_eq!(to_delete.len(), 1);
        assert_eq!(to_delete[0].name, "old");
    }

    #[test]
    fn test_apply_retention_no_removal_within_ttl() {
        let backups = vec![completed_backup("recent", 1)];
        let policy = RetentionPolicy::new_ttl(24);
        let to_delete = apply_retention(&backups, &policy);
        assert!(to_delete.is_empty());
    }

    #[test]
    fn test_apply_retention_no_policy_removes_nothing() {
        let backups = vec![
            completed_backup("b1", 100),
            completed_backup("b2", 200),
        ];
        let policy = RetentionPolicy {
            max_backups: None,
            ttl_hours: None,
        };
        let to_delete = apply_retention(&backups, &policy);
        assert!(to_delete.is_empty());
>>>>>>> claude/gallant-meninsky
=======
>>>>>>> claude/thirsty-lederberg
    }
}
