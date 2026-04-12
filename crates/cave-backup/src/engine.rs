//! Backup and restore creation logic.

use crate::models::*;
use chrono::Utc;
use uuid::Uuid;

/// Create a new backup in the InProgress phase from a spec.
pub fn create_backup(name: String, spec: BackupSpec) -> Backup {
    let now = Utc::now();
    let expires_at = if spec.ttl_hours > 0 {
        Some(now + chrono::Duration::hours(spec.ttl_hours as i64))
    } else {
        None
    };
    Backup {
        id: Uuid::new_v4(),
        name,
        phase: BackupPhase::InProgress,
        storage_location: spec.storage_location.clone(),
        included_namespaces: spec.included_namespaces.clone(),
        excluded_namespaces: spec.excluded_namespaces.clone(),
        included_resources: spec.included_resources.clone(),
        excluded_resources: spec.excluded_resources.clone(),
        label_selector: spec.label_selector.clone(),
        include_cluster_resources: spec.include_cluster_resources,
        ttl_hours: spec.ttl_hours,
        hooks: spec.hooks.clone(),
        volume_snapshot_locations: vec![],
        default_volumes_to_fs_backup: spec.default_volumes_to_fs_backup,
        snapshot_move_data: false,
        expires_at,
        started_at: Some(now),
        completed_at: None,
        items_backed_up: 0,
        total_items: 0,
        warnings: 0,
        errors: 0,
        size_bytes: 0,
        logs: vec![format!("[{}] Backup started", now.to_rfc3339())],
        created_at: now,
    }
}

/// Finalize a backup, setting phase, counts, and completion timestamp.
pub fn complete_backup(backup: &mut Backup, items: u64, size_bytes: u64, errors: u64) {
    let now = chrono::Utc::now();
    backup.phase = if errors > 0 {
        BackupPhase::PartiallyFailed
    } else {
        BackupPhase::Completed
    };
    backup.items_backed_up = items;
    backup.total_items = items;
    backup.size_bytes = size_bytes;
    backup.errors = errors;
    backup.completed_at = Some(now);
    backup.logs.push(format!(
        "[{}] Backup completed: {} items, {} bytes",
        now.to_rfc3339(),
        items,
        size_bytes
    ));
}

/// Create a new restore in the InProgress phase.
pub fn create_restore(
    name: String,
    backup_id: Uuid,
    backup_name: String,
    restore_pvs: bool,
    namespace_mappings: std::collections::HashMap<String, String>,
    included_namespaces: Vec<String>,
    excluded_namespaces: Vec<String>,
    included_resources: Vec<String>,
    excluded_resources: Vec<String>,
    existing_resource_policy: ExistingResourcePolicy,
) -> Restore {
    let now = chrono::Utc::now();
    Restore {
        id: Uuid::new_v4(),
        name,
        backup_name,
        backup_id,
        phase: RestorePhase::InProgress,
        included_namespaces,
        excluded_namespaces,
        included_resources,
        excluded_resources,
        namespace_mappings,
        restore_pvs,
        preserve_node_ports: false,
        hooks: vec![],
        existing_resource_policy,
        started_at: Some(now),
        completed_at: None,
        warnings: 0,
        errors: 0,
        logs: vec![format!("[{}] Restore started", now.to_rfc3339())],
        created_at: now,
    }
}

/// Simulate running exec hooks and return log lines.
pub fn run_exec_hooks(hooks: &[ExecHook], phase: &str) -> Vec<String> {
    let now = chrono::Utc::now();
    hooks
        .iter()
        .map(|h| {
            format!(
                "[{}] {} hook: container={} cmd={:?}",
                now.to_rfc3339(),
                phase,
                h.container,
                h.command
            )
        })
        .collect()
}

/// Returns true if the backup has passed its TTL expiry time.
pub fn check_expiration(backup: &Backup) -> bool {
    if let Some(expires_at) = backup.expires_at {
        chrono::Utc::now() > expires_at
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn default_spec() -> BackupSpec {
        BackupSpec {
            storage_location: "default".into(),
            ttl_hours: 24,
            ..Default::default()
        }
    }

    #[test]
    fn create_backup_starts_in_progress() {
        let backup = create_backup("test-backup".into(), default_spec());
        assert_eq!(backup.phase, BackupPhase::InProgress);
        assert_eq!(backup.name, "test-backup");
        assert!(backup.started_at.is_some());
        assert!(backup.completed_at.is_none());
        assert!(!backup.logs.is_empty());
    }

    #[test]
    fn create_backup_sets_expiry_when_ttl_nonzero() {
        let spec = BackupSpec {
            ttl_hours: 72,
            ..Default::default()
        };
        let backup = create_backup("ttl-test".into(), spec);
        assert!(backup.expires_at.is_some());
    }

    #[test]
    fn create_backup_no_expiry_when_ttl_zero() {
        let spec = BackupSpec {
            ttl_hours: 0,
            ..Default::default()
        };
        let backup = create_backup("no-ttl".into(), spec);
        assert!(backup.expires_at.is_none());
    }

    #[test]
    fn complete_backup_sets_completed_phase() {
        let mut backup = create_backup("complete-test".into(), default_spec());
        complete_backup(&mut backup, 42, 1024, 0);
        assert_eq!(backup.phase, BackupPhase::Completed);
        assert_eq!(backup.items_backed_up, 42);
        assert_eq!(backup.size_bytes, 1024);
        assert!(backup.completed_at.is_some());
        assert_eq!(backup.logs.len(), 2);
    }

    #[test]
    fn complete_backup_with_errors_sets_partially_failed() {
        let mut backup = create_backup("error-test".into(), default_spec());
        complete_backup(&mut backup, 10, 512, 2);
        assert_eq!(backup.phase, BackupPhase::PartiallyFailed);
        assert_eq!(backup.errors, 2);
    }

    #[test]
    fn create_restore_starts_in_progress() {
        let backup_id = Uuid::new_v4();
        let restore = create_restore(
            "my-restore".into(),
            backup_id,
            "my-backup".into(),
            true,
            HashMap::new(),
            vec![],
            vec![],
            vec![],
            vec![],
            ExistingResourcePolicy::None,
        );
        assert_eq!(restore.phase, RestorePhase::InProgress);
        assert_eq!(restore.backup_id, backup_id);
        assert!(restore.started_at.is_some());
        assert!(restore.completed_at.is_none());
    }

    #[test]
    fn run_exec_hooks_returns_log_per_hook() {
        let hooks = vec![
            ExecHook {
                container: "app".into(),
                command: vec!["freeze".into()],
                on_error: HookErrorMode::Continue,
                timeout_seconds: 30,
            },
            ExecHook {
                container: "sidecar".into(),
                command: vec!["flush".into()],
                on_error: HookErrorMode::Fail,
                timeout_seconds: 10,
            },
        ];
        let logs = run_exec_hooks(&hooks, "pre");
        assert_eq!(logs.len(), 2);
        assert!(logs[0].contains("pre"));
        assert!(logs[0].contains("app"));
    }

    #[test]
    fn check_expiration_not_expired_when_future() {
        let spec = BackupSpec {
            ttl_hours: 999,
            ..Default::default()
        };
        let backup = create_backup("future".into(), spec);
        assert!(!check_expiration(&backup));
    }

    #[test]
    fn check_expiration_false_when_no_expiry() {
        let spec = BackupSpec {
            ttl_hours: 0,
            ..Default::default()
        };
        let backup = create_backup("no-expiry".into(), spec);
        assert!(!check_expiration(&backup));
    }

    #[test]
    fn check_expiration_true_when_expired() {
        let mut backup = create_backup("expired".into(), default_spec());
        // Force the expiry into the past
        backup.expires_at = Some(chrono::Utc::now() - chrono::Duration::hours(1));
        assert!(check_expiration(&backup));
    }
}
