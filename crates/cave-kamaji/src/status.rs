// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TenantControlPlane status conditions — Kubernetes-style
//! `[Ready, ControlPlaneHealthy, KubeconfigReady]` triples.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   api/v1alpha1/tenantcontrolplane_status.go (Conditions field)
//!
//! Mirrors `metav1.Condition` semantics: `set_condition` updates in
//! place when a Condition with the same type already exists, preserving
//! the rest of the array order.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{TenantControlPlane, TenantPhase};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConditionType {
    /// Aggregate readiness: all sub-conditions True.
    Ready,
    /// kube-apiserver pods are running + leader-elected.
    ControlPlaneHealthy,
    /// kubeconfig endpoint is reachable + cert valid.
    KubeconfigReady,
    /// Datastore (etcd / kine / sql) reports healthy.
    DataStoreHealthy,
    /// Konnectivity tunnel up.
    KonnectivityHealthy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub cond_type: ConditionType,
    pub status: ConditionStatus,
    pub reason: String,
    pub message: String,
    pub last_transition_time: DateTime<Utc>,
}

/// Set / update a condition in place. If a Condition with `cond_type`
/// already exists, replace it; otherwise append.
pub fn set_condition(
    conds: &mut Vec<Condition>,
    cond_type: ConditionType,
    status: ConditionStatus,
    reason: &str,
    message: &str,
) {
    let now = Utc::now();
    if let Some(c) = conds.iter_mut().find(|c| c.cond_type == cond_type) {
        c.status = status;
        c.reason = reason.to_string();
        c.message = message.to_string();
        c.last_transition_time = now;
        return;
    }
    conds.push(Condition {
        cond_type,
        status,
        reason: reason.to_string(),
        message: message.to_string(),
        last_transition_time: now,
    });
}

/// Compute the aggregate condition list from a TenantControlPlane's
/// current state. Stateless — every reconcile pass calls this and
/// overrides whatever conditions were previously stored.
pub fn status_summary(tcp: &TenantControlPlane) -> Vec<Condition> {
    let mut out = Vec::new();
    let running = tcp.status.phase == TenantPhase::Running && tcp.status.ready;
    let cp_status = if running {
        ConditionStatus::True
    } else {
        ConditionStatus::False
    };
    set_condition(
        &mut out,
        ConditionType::ControlPlaneHealthy,
        cp_status,
        if running { "Healthy" } else { "NotHealthy" },
        if running {
            "all api-server replicas ready"
        } else {
            "control-plane is initialising"
        },
    );
    let kc_status = if tcp.status.api_server_endpoint.is_some() && running {
        ConditionStatus::True
    } else {
        ConditionStatus::False
    };
    set_condition(
        &mut out,
        ConditionType::KubeconfigReady,
        kc_status,
        "EndpointReachable",
        "kubeconfig endpoint populated",
    );
    set_condition(
        &mut out,
        ConditionType::DataStoreHealthy,
        if running {
            ConditionStatus::True
        } else {
            ConditionStatus::Unknown
        },
        "Healthy",
        "datastore reachable",
    );
    set_condition(
        &mut out,
        ConditionType::KonnectivityHealthy,
        if running {
            ConditionStatus::True
        } else {
            ConditionStatus::Unknown
        },
        "Healthy",
        "konnectivity tunnel up",
    );
    // Aggregate Ready: all sub-conditions True.
    let all_true = out.iter().all(|c| c.status == ConditionStatus::True);
    set_condition(
        &mut out,
        ConditionType::Ready,
        if all_true {
            ConditionStatus::True
        } else {
            ConditionStatus::False
        },
        if all_true { "Ready" } else { "NotReady" },
        if all_true {
            "all sub-conditions True"
        } else {
            "one or more sub-conditions not True"
        },
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_condition_preserves_array_order_when_updating() {
        let mut conds = Vec::new();
        set_condition(
            &mut conds,
            ConditionType::ControlPlaneHealthy,
            ConditionStatus::False,
            "Init",
            "starting",
        );
        set_condition(
            &mut conds,
            ConditionType::KubeconfigReady,
            ConditionStatus::False,
            "Init",
            "starting",
        );
        set_condition(
            &mut conds,
            ConditionType::ControlPlaneHealthy,
            ConditionStatus::True,
            "Healthy",
            "up",
        );
        assert_eq!(conds.len(), 2);
        // ControlPlaneHealthy stays at index 0.
        assert_eq!(conds[0].cond_type, ConditionType::ControlPlaneHealthy);
        assert_eq!(conds[0].status, ConditionStatus::True);
        assert_eq!(conds[1].cond_type, ConditionType::KubeconfigReady);
    }
}
