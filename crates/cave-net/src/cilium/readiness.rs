// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod readiness gate — `cilium-startup-conf` integration.
//!
//! Mirrors `pkg/k8s/podreadinessgate.go` and the readiness-gate
//! controller in `pkg/k8s/watchers/podreadiness.go`. cilium-agent
//! flips the `network.cilium.io/cilium-agent-pod-ready` condition on
//! pods after their endpoint has been programmed, so kubelet doesn't
//! mark them Ready before BPF policy is in place.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateStatus {
    /// Endpoint not yet programmed.
    Pending,
    /// Endpoint programmed, condition flipped True.
    Ready,
    /// Endpoint programming failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    pub pod_namespace: String,
    pub pod_name: String,
    pub status: GateStatus,
    pub last_update_ns: u64,
    pub message: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadinessError {
    #[error("pod `{0}/{1}` not registered for readiness gate")]
    NotRegistered(String, String),
    #[error("invalid transition {from:?} → {to:?}")]
    BadTransition { from: GateStatus, to: GateStatus },
    #[error("tenant {tenant} cannot mutate readiness state owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ReadinessGateController {
    pub tenant: TenantId,
    pub condition_name: String,
    states: BTreeMap<String, GateState>,
}

impl ReadinessGateController {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            condition_name: "network.cilium.io/cilium-agent-pod-ready".into(),
            states: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, namespace: impl Into<String>, name: impl Into<String>, now_ns: u64) {
        let ns = namespace.into();
        let n = name.into();
        let key = format!("{ns}/{n}");
        self.states.entry(key).or_insert(GateState {
            pod_namespace: ns,
            pod_name: n,
            status: GateStatus::Pending,
            last_update_ns: now_ns,
            message: "endpoint regeneration pending".into(),
        });
    }

    pub fn set_ready(
        &mut self,
        namespace: &str,
        name: &str,
        now_ns: u64,
    ) -> Result<(), ReadinessError> {
        let key = format!("{namespace}/{name}");
        let s = self.states.get_mut(&key).ok_or_else(|| {
            ReadinessError::NotRegistered(namespace.to_string(), name.to_string())
        })?;
        if matches!(s.status, GateStatus::Failed) {
            return Err(ReadinessError::BadTransition {
                from: GateStatus::Failed,
                to: GateStatus::Ready,
            });
        }
        s.status = GateStatus::Ready;
        s.last_update_ns = now_ns;
        s.message = "endpoint programmed".into();
        Ok(())
    }

    pub fn set_failed(
        &mut self,
        namespace: &str,
        name: &str,
        now_ns: u64,
        reason: impl Into<String>,
    ) -> Result<(), ReadinessError> {
        let key = format!("{namespace}/{name}");
        let s = self.states.get_mut(&key).ok_or_else(|| {
            ReadinessError::NotRegistered(namespace.to_string(), name.to_string())
        })?;
        s.status = GateStatus::Failed;
        s.last_update_ns = now_ns;
        s.message = reason.into();
        Ok(())
    }

    pub fn remove(&mut self, namespace: &str, name: &str) -> Result<(), ReadinessError> {
        let key = format!("{namespace}/{name}");
        self.states.remove(&key).ok_or_else(|| {
            ReadinessError::NotRegistered(namespace.to_string(), name.to_string())
        })?;
        Ok(())
    }

    pub fn status(&self, namespace: &str, name: &str) -> Option<&GateState> {
        let key = format!("{namespace}/{name}");
        self.states.get(&key)
    }

    pub fn count(&self) -> usize {
        self.states.len()
    }

    pub fn ready_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| matches!(s.status, GateStatus::Ready))
            .count()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/k8s/podreadinessgate.go", "ReadinessGate");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ctrl(tenant: TenantId) -> ReadinessGateController {
        ReadinessGateController::new(tenant)
    }

    #[test]
    fn register_starts_pending() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "Register", "tenant-rg-r");
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        assert_eq!(
            c.status("default", "p1").unwrap().status,
            GateStatus::Pending
        );
    }

    #[test]
    fn register_idempotent() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "Register.Idempotent",
            "tenant-rg-ri"
        );
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.register("default", "p1", 200);
        assert_eq!(c.count(), 1);
    }

    #[test]
    fn set_ready_advances_state() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "SetReady", "tenant-rg-sr");
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.set_ready("default", "p1", 200).unwrap();
        assert_eq!(c.status("default", "p1").unwrap().status, GateStatus::Ready);
    }

    #[test]
    fn set_failed_advances_to_failed() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "SetFailed", "tenant-rg-sf");
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.set_failed("default", "p1", 200, "verifier rejected")
            .unwrap();
        let s = c.status("default", "p1").unwrap();
        assert_eq!(s.status, GateStatus::Failed);
        assert_eq!(s.message, "verifier rejected");
    }

    #[test]
    fn set_ready_after_failed_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "SetReady.AfterFailed",
            "tenant-rg-srf"
        );
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.set_failed("default", "p1", 200, "x").unwrap();
        let err = c.set_ready("default", "p1", 300).unwrap_err();
        assert!(matches!(err, ReadinessError::BadTransition { .. }));
    }

    #[test]
    fn set_ready_unknown_returns_not_registered() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "SetReady.NotRegistered",
            "tenant-rg-snr"
        );
        let mut c = ctrl(tenant);
        let err = c.set_ready("default", "p1", 100).unwrap_err();
        assert!(matches!(err, ReadinessError::NotRegistered(_, _)));
    }

    #[test]
    fn set_failed_unknown_returns_not_registered() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "SetFailed.NotRegistered",
            "tenant-rg-sfnr"
        );
        let mut c = ctrl(tenant);
        let err = c.set_failed("default", "p1", 100, "x").unwrap_err();
        assert!(matches!(err, ReadinessError::NotRegistered(_, _)));
    }

    #[test]
    fn remove_drops_state() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "Remove", "tenant-rg-rm");
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.remove("default", "p1").unwrap();
        assert!(c.status("default", "p1").is_none());
    }

    #[test]
    fn remove_unknown_returns_not_registered() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "Remove.NotRegistered",
            "tenant-rg-rmnr"
        );
        let mut c = ctrl(tenant);
        let err = c.remove("default", "p1").unwrap_err();
        assert!(matches!(err, ReadinessError::NotRegistered(_, _)));
    }

    #[test]
    fn status_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "Status.NotFound",
            "tenant-rg-snf"
        );
        let c = ctrl(tenant);
        assert!(c.status("default", "p1").is_none());
    }

    #[test]
    fn count_tracks_register() {
        let (_c, tenant) = cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "Count", "tenant-rg-c");
        let mut c = ctrl(tenant);
        for i in 0..5u8 {
            c.register("default", &format!("p-{i}"), 100);
        }
        assert_eq!(c.count(), 5);
    }

    #[test]
    fn ready_count_only_counts_ready() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/k8s/podreadinessgate.go", "ReadyCount", "tenant-rg-rc");
        let mut c = ctrl(tenant);
        for i in 0..3u8 {
            c.register("default", &format!("p-{i}"), 100);
            c.set_ready("default", &format!("p-{i}"), 200).unwrap();
        }
        c.register("default", "pending", 100);
        assert_eq!(c.ready_count(), 3);
    }

    #[test]
    fn condition_name_matches_upstream() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "ConditionName",
            "tenant-rg-cn"
        );
        let c = ctrl(tenant);
        assert_eq!(c.condition_name, "network.cilium.io/cilium-agent-pod-ready");
    }

    #[test]
    fn set_ready_records_message() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "SetReady.Message",
            "tenant-rg-srm"
        );
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.set_ready("default", "p1", 200).unwrap();
        assert_eq!(
            c.status("default", "p1").unwrap().message,
            "endpoint programmed"
        );
    }

    #[test]
    fn set_ready_updates_timestamp() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "SetReady.Timestamp",
            "tenant-rg-srt"
        );
        let mut c = ctrl(tenant);
        c.register("default", "p1", 100);
        c.set_ready("default", "p1", 999).unwrap();
        assert_eq!(c.status("default", "p1").unwrap().last_update_ns, 999);
    }

    #[test]
    fn gate_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "State.Serde",
            "tenant-rg-sserde"
        );
        let s = GateState {
            pod_namespace: "default".into(),
            pod_name: "p1".into(),
            status: GateStatus::Ready,
            last_update_ns: 100,
            message: "ok".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: GateState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn gate_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/podreadinessgate.go",
            "Status.Serde",
            "tenant-rg-stserde"
        );
        for st in [GateStatus::Pending, GateStatus::Ready, GateStatus::Failed] {
            let s = serde_json::to_string(&st).unwrap();
            let back: GateStatus = serde_json::from_str(&s).unwrap();
            assert_eq!(back, st);
        }
    }
}
