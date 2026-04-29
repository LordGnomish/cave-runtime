//! ControllerManager — host that registers and runs the built-in controllers.

use crate::models::{ControllerInfo, ControllerState};
use chrono::Utc;
use dashmap::DashMap;

/// The set of built-in controller names registered by `new_manager`.
pub const BUILTIN_CONTROLLERS: &[&str] = &[
    "deployment",
    "replicaset",
    "node-lifecycle",
    "namespace",
    "garbage-collector",
    "endpoints",
    "service-account",
    "job",
];

/// Manager holding registration + running state for each controller.
pub struct ControllerManager {
    pub controllers: DashMap<String, ControllerInfo>,
}

impl Default for ControllerManager {
    fn default() -> Self {
        new_manager()
    }
}

/// Create a new manager pre-registered with the built-in controllers.
///
/// Mirrors upstream `cmd/kube-controller-manager/app/controllermanager.go::NewControllerManagerCommand`.
pub fn new_manager() -> ControllerManager {
    let controllers = DashMap::new();
    for name in BUILTIN_CONTROLLERS {
        controllers.insert(name.to_string(), ControllerInfo::new(name));
    }
    ControllerManager { controllers }
}

/// Mark a controller as Running and stamp the start time as its last sync.
pub fn start_controller(mgr: &ControllerManager, name: &str) -> bool {
    if let Some(mut info) = mgr.controllers.get_mut(name) {
        info.state = ControllerState::Running;
        info.last_sync = Some(Utc::now());
        true
    } else {
        false
    }
}

/// Stop a running controller.
pub fn stop_controller(mgr: &ControllerManager, name: &str) -> bool {
    if let Some(mut info) = mgr.controllers.get_mut(name) {
        info.state = ControllerState::Stopped;
        true
    } else {
        false
    }
}

/// Record that a controller's reconcile loop ran (success or failure).
pub fn record_reconcile(mgr: &ControllerManager, name: &str, error: Option<String>) {
    if let Some(mut info) = mgr.controllers.get_mut(name) {
        info.reconcile_count += 1;
        info.last_sync = Some(Utc::now());
        if let Some(msg) = error {
            info.error_count += 1;
            info.last_error = Some(msg);
            info.state = ControllerState::Failed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_manager_registers_all_builtins() {
        let mgr = new_manager();
        for name in BUILTIN_CONTROLLERS {
            assert!(mgr.controllers.contains_key(*name), "missing {}", name);
        }
        assert_eq!(mgr.controllers.len(), BUILTIN_CONTROLLERS.len());
    }

    #[test]
    fn test_start_and_stop_controller() {
        let mgr = new_manager();
        assert!(start_controller(&mgr, "deployment"));
        assert_eq!(
            mgr.controllers.get("deployment").unwrap().state,
            ControllerState::Running,
        );
        assert!(stop_controller(&mgr, "deployment"));
        assert_eq!(
            mgr.controllers.get("deployment").unwrap().state,
            ControllerState::Stopped,
        );
    }

    #[test]
    fn test_record_reconcile_success_increments_count() {
        let mgr = new_manager();
        record_reconcile(&mgr, "job", None);
        record_reconcile(&mgr, "job", None);
        let info = mgr.controllers.get("job").unwrap();
        assert_eq!(info.reconcile_count, 2);
        assert_eq!(info.error_count, 0);
        assert!(info.last_error.is_none());
    }

    #[test]
    fn test_record_reconcile_failure_marks_failed() {
        let mgr = new_manager();
        record_reconcile(&mgr, "job", Some("boom".into()));
        let info = mgr.controllers.get("job").unwrap();
        assert_eq!(info.error_count, 1);
        assert_eq!(info.state, ControllerState::Failed);
        assert_eq!(info.last_error.as_deref(), Some("boom"));
    }
}
