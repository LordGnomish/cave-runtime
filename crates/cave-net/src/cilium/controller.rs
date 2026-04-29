//! Controller-runtime style scheduler primitives.
//!
//! Mirrors `pkg/controller/controller.go`. Cilium's controller is the
//! agent's tiny periodic-job runner: each registered job has a `do`
//! function and a `params` (interval, jitter, error backoff). The
//! manager runs the jobs concurrently and tracks their last status.
//!
//! We port the manager + status surface (no real timer; the model
//! drives a single tick at a time so tests are deterministic).

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Success,
    Failure,
}

/// Per-controller parameters. Mirrors `ControllerParams` in upstream.
#[derive(Debug, Clone)]
pub struct ControllerParams {
    pub run_interval: Duration,
    pub error_retry: Duration,
    pub error_retry_base_max: Duration,
    pub max_consecutive_errors: u32,
}

impl Default for ControllerParams {
    fn default() -> Self {
        Self {
            run_interval: Duration::from_secs(60),
            error_retry: Duration::from_secs(1),
            error_retry_base_max: Duration::from_secs(60),
            max_consecutive_errors: 5,
        }
    }
}

/// Per-controller status snapshot.
#[derive(Debug, Clone, Default)]
pub struct ControllerStatus {
    pub success_count: u64,
    pub failure_count: u64,
    pub consecutive_failures: u32,
    pub last_outcome: Option<RunOutcome>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControllerError {
    #[error("controller {0} not found")]
    NotFound(String),
    #[error("controller {0} reached max consecutive errors")]
    MaxErrors(String),
    #[error("tenant {tenant} cannot mutate controller manager")]
    TenantDenied { tenant: TenantId },
}

/// Manager. Mirrors `pkg/controller.Manager`.
#[derive(Debug)]
pub struct ControllerManager {
    pub tenant: TenantId,
    controllers: BTreeMap<String, (ControllerParams, ControllerStatus)>,
}

impl ControllerManager {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, controllers: BTreeMap::new() }
    }

    pub fn register(&mut self, name: &str, params: ControllerParams) {
        self.controllers.insert(name.to_string(), (params, ControllerStatus::default()));
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.controllers.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<&String> { self.controllers.keys().collect() }

    pub fn status(&self, name: &str) -> Option<&ControllerStatus> {
        self.controllers.get(name).map(|(_, s)| s)
    }

    /// Record a single run outcome. Returns the new consecutive_failures
    /// count, or `MaxErrors` if the controller has hit its threshold.
    pub fn record(&mut self, name: &str, outcome: RunOutcome) -> Result<u32, ControllerError> {
        let (params, status) = self.controllers.get_mut(name)
            .ok_or_else(|| ControllerError::NotFound(name.to_string()))?;
        match outcome {
            RunOutcome::Success => {
                status.success_count += 1;
                status.consecutive_failures = 0;
                status.last_outcome = Some(RunOutcome::Success);
                Ok(0)
            }
            RunOutcome::Failure => {
                status.failure_count += 1;
                status.consecutive_failures += 1;
                status.last_outcome = Some(RunOutcome::Failure);
                if status.consecutive_failures >= params.max_consecutive_errors {
                    return Err(ControllerError::MaxErrors(name.to_string()));
                }
                Ok(status.consecutive_failures)
            }
        }
    }

    pub fn len(&self) -> usize { self.controllers.len() }
    pub fn is_empty(&self) -> bool { self.controllers.is_empty() }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/controller/controller.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn default_params_match_upstream_defaults() {
        let (_c, _t) = cilium_test_ctx!("pkg/controller/controller.go", "Params.Default", "tenant-ct-pd");
        let p = ControllerParams::default();
        assert_eq!(p.run_interval, Duration::from_secs(60));
        assert_eq!(p.max_consecutive_errors, 5);
    }

    #[test]
    fn register_then_status_returns_zeroed() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Status.Init", "tenant-ct-si");
        let mut mgr = ControllerManager::new(t);
        mgr.register("conntrack-gc", ControllerParams::default());
        let s = mgr.status("conntrack-gc").unwrap();
        assert_eq!(s.success_count, 0);
        assert_eq!(s.failure_count, 0);
        assert!(s.last_outcome.is_none());
    }

    #[test]
    fn record_success_increments_success_count() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Record.Success", "tenant-ct-rs");
        let mut mgr = ControllerManager::new(t);
        mgr.register("c1", ControllerParams::default());
        let r = mgr.record("c1", RunOutcome::Success).unwrap();
        assert_eq!(r, 0);
        assert_eq!(mgr.status("c1").unwrap().success_count, 1);
        assert_eq!(mgr.status("c1").unwrap().last_outcome, Some(RunOutcome::Success));
    }

    #[test]
    fn record_failure_increments_failure_count() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Record.Failure", "tenant-ct-rf");
        let mut mgr = ControllerManager::new(t);
        mgr.register("c1", ControllerParams::default());
        let r = mgr.record("c1", RunOutcome::Failure).unwrap();
        assert_eq!(r, 1);
        assert_eq!(mgr.status("c1").unwrap().failure_count, 1);
        assert_eq!(mgr.status("c1").unwrap().consecutive_failures, 1);
    }

    #[test]
    fn success_resets_consecutive_failures() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Reset.OnSuccess", "tenant-ct-rcs");
        let mut mgr = ControllerManager::new(t);
        mgr.register("c1", ControllerParams::default());
        mgr.record("c1", RunOutcome::Failure).unwrap();
        mgr.record("c1", RunOutcome::Failure).unwrap();
        mgr.record("c1", RunOutcome::Success).unwrap();
        assert_eq!(mgr.status("c1").unwrap().consecutive_failures, 0);
    }

    #[test]
    fn max_consecutive_errors_returns_error() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "MaxErrors", "tenant-ct-me");
        let mut mgr = ControllerManager::new(t);
        let mut p = ControllerParams::default();
        p.max_consecutive_errors = 2;
        mgr.register("c1", p);
        mgr.record("c1", RunOutcome::Failure).unwrap();
        let r = mgr.record("c1", RunOutcome::Failure);
        assert!(matches!(r, Err(ControllerError::MaxErrors(_))));
    }

    #[test]
    fn unregister_removes_controller() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Unregister", "tenant-ct-un");
        let mut mgr = ControllerManager::new(t);
        mgr.register("c1", ControllerParams::default());
        assert!(mgr.unregister("c1"));
        assert!(mgr.is_empty());
    }

    #[test]
    fn record_unknown_controller_errors() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "Record.Unknown", "tenant-ct-ru");
        let mut mgr = ControllerManager::new(t);
        let e = mgr.record("ghost", RunOutcome::Success).unwrap_err();
        assert_eq!(e, ControllerError::NotFound("ghost".into()));
    }

    #[test]
    fn list_returns_registered_names() {
        let (_c, t) = cilium_test_ctx!("pkg/controller/controller.go", "List", "tenant-ct-l");
        let mut mgr = ControllerManager::new(t);
        mgr.register("a", ControllerParams::default());
        mgr.register("b", ControllerParams::default());
        let l = mgr.list();
        assert_eq!(l.len(), 2);
    }
}
