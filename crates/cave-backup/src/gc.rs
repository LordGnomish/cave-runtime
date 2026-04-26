//! Garbage collection for expired backups.

use crate::models::{Backup, BackupPhase};
use std::collections::HashMap;
use uuid::Uuid;

/// Find backups that have expired and should be garbage collected.
pub fn find_expired(backups: &HashMap<Uuid, Backup>) -> Vec<Uuid> {
    backups
        .values()
        .filter(|b| crate::engine::check_expiration(b) && b.phase != BackupPhase::Deleting)
        .map(|b| b.id)
        .collect()
}

/// Mark a backup as being deleted (TTL-triggered GC).
pub fn mark_deleting(backup: &mut Backup) {
    backup.phase = BackupPhase::Deleting;
    backup.logs.push(format!(
        "[{}] Marked for deletion (TTL expired)",
        chrono::Utc::now().to_rfc3339()
    ));
}

/// Return a JSON summary of GC-relevant backup counts.
pub fn gc_stats(backups: &HashMap<Uuid, Backup>) -> serde_json::Value {
    let total = backups.len();
    let expired = find_expired(backups).len();
    let deleting = backups
        .values()
        .filter(|b| b.phase == BackupPhase::Deleting)
        .count();
    serde_json::json!({
        "total": total,
        "expired": expired,
        "deleting": deleting,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{complete_backup, create_backup};
    use crate::models::BackupSpec;

    fn spec() -> BackupSpec {
        BackupSpec {
            storage_location: "default".into(),
            ttl_hours: 1,
            ..Default::default()
        }
    }

    fn expired_backup() -> Backup {
        let mut b = create_backup("expired".into(), spec());
        // force expiry into the past
        b.expires_at = Some(chrono::Utc::now() - chrono::Duration::hours(2));
        b
    }

    fn live_backup() -> Backup {
        let mut b = create_backup("live".into(), spec());
        b.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(48));
        b
    }

    #[test]
    fn find_expired_returns_only_expired() {
        let mut map = HashMap::new();
        let exp = expired_backup();
        let live = live_backup();
        let exp_id = exp.id;
        map.insert(exp.id, exp);
        map.insert(live.id, live);

        let expired = find_expired(&map);
        assert_eq!(expired, vec![exp_id]);
    }

    #[test]
    fn find_expired_excludes_already_deleting() {
        let mut map = HashMap::new();
        let mut b = expired_backup();
        b.phase = BackupPhase::Deleting;
        map.insert(b.id, b);

        assert!(find_expired(&map).is_empty());
    }

    #[test]
    fn find_expired_empty_map() {
        let map: HashMap<Uuid, Backup> = HashMap::new();
        assert!(find_expired(&map).is_empty());
    }

    #[test]
    fn mark_deleting_sets_phase_and_log() {
        let mut b = create_backup("to-delete".into(), spec());
        complete_backup(&mut b, 5, 100, 0);
        let log_count_before = b.logs.len();
        mark_deleting(&mut b);
        assert_eq!(b.phase, BackupPhase::Deleting);
        assert_eq!(b.logs.len(), log_count_before + 1);
        assert!(b.logs.last().unwrap().contains("TTL expired"));
    }

    #[test]
    fn gc_stats_correct_counts() {
        let mut map = HashMap::new();

        let exp = expired_backup();
        map.insert(exp.id, exp);

        let mut del = expired_backup();
        del.phase = BackupPhase::Deleting;
        map.insert(del.id, del);

        let live = live_backup();
        map.insert(live.id, live);

        let stats = gc_stats(&map);
        assert_eq!(stats["total"], 3);
        assert_eq!(stats["expired"], 1); // only the non-Deleting expired one
        assert_eq!(stats["deleting"], 1);
    }
}
