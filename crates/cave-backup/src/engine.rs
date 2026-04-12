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
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }
}
