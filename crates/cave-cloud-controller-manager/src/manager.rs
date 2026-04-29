//! CloudControllerManager — host that registers and runs cloud controllers.

use crate::models::{CloudControllerInfo, CloudControllerState};
use crate::provider::{CloudProvider, FakeProvider};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;

pub const BUILTIN_CONTROLLERS: &[&str] = &[
    "node",
    "route",
    "service-lb",
    "node-ipam",
];

/// Manager hosting the cloud controllers and the active CloudProvider.
pub struct CloudControllerManager {
    pub controllers: DashMap<String, CloudControllerInfo>,
    pub provider: Arc<dyn CloudProvider>,
}

impl Default for CloudControllerManager {
    fn default() -> Self {
        new_manager()
    }
}

/// Create a manager pre-registered with the built-in controllers and a
/// `FakeProvider` (suitable for tests + dev). Replace `provider` to target a
/// real cloud.
///
/// Mirrors `cmd/cloud-controller-manager/main.go::NewCloudControllerManager`.
pub fn new_manager() -> CloudControllerManager {
    new_manager_with(Arc::new(FakeProvider::default()))
}

/// Create a manager with a specific provider implementation.
pub fn new_manager_with(provider: Arc<dyn CloudProvider>) -> CloudControllerManager {
    let controllers = DashMap::new();
    for name in BUILTIN_CONTROLLERS {
        controllers.insert(name.to_string(), CloudControllerInfo::new(name));
    }
    CloudControllerManager { controllers, provider }
}

pub fn record_reconcile(mgr: &CloudControllerManager, name: &str, error: bool) {
    if let Some(mut info) = mgr.controllers.get_mut(name) {
        info.reconcile_count += 1;
        info.last_sync = Some(Utc::now());
        if error {
            info.error_count += 1;
            info.state = CloudControllerState::Failed;
        } else {
            info.state = CloudControllerState::Running;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `TestCloudManagerStartsAll` — the manager pre-registers the
    /// canonical built-in cloud controllers.
    #[test]
    fn test_cloud_manager_starts_all() {
        let mgr = new_manager();
        for name in BUILTIN_CONTROLLERS {
            assert!(mgr.controllers.contains_key(*name), "missing {}", name);
        }
        assert_eq!(mgr.controllers.len(), BUILTIN_CONTROLLERS.len());
        assert_eq!(mgr.provider.name(), "fake");
    }

    #[test]
    fn test_record_reconcile_success_marks_running() {
        let mgr = new_manager();
        record_reconcile(&mgr, "node", false);
        let info = mgr.controllers.get("node").unwrap();
        assert_eq!(info.state, CloudControllerState::Running);
        assert_eq!(info.reconcile_count, 1);
    }
}
