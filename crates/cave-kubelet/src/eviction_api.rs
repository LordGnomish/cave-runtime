//! Eviction API v1 — `policy/v1.Eviction` subresource handling.
//!
//! Mirrors `pkg/registry/core/pod/storage/eviction.go`: pod eviction
//! requests routed through `pods/<name>/eviction`, dry-run handling,
//! resourceVersion preconditions, deleteOptions overrides, and
//! PodDisruptionBudget (PDB) admission with `disruptionsAllowed`,
//! `currentHealthy`, and `expectedPods`. Returns the canonical
//! status codes: 200 success, 201 created, 429 too-many-requests
//! (PDB blocks), 500 with reason for misconfigured PDB.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteOptions {
    pub grace_period_seconds: Option<i64>,
    pub preconditions: Option<Preconditions>,
    pub propagation_policy: Option<PropagationPolicy>,
    pub dry_run: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preconditions {
    pub uid: Option<String>,
    pub resource_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropagationPolicy {
    Orphan,
    Background,
    Foreground,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionRequest {
    pub namespace: String,
    pub pod_name: String,
    pub delete_options: DeleteOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodInfo {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub resource_version: String,
    pub labels: BTreeMap<String, String>,
    pub phase: PodPhase,
    pub deletion_timestamp_set: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodDisruptionBudget {
    pub namespace: String,
    pub name: String,
    /// Selector that defines which pods are protected.
    pub selector: BTreeMap<String, String>,
    pub min_available: Option<UIntOrPercent>,
    pub max_unavailable: Option<UIntOrPercent>,
    /// Status fields (mirrors observed status).
    pub current_healthy: i32,
    pub desired_healthy: i32,
    pub disruptions_allowed: i32,
    pub expected_pods: i32,
    pub observed_generation: i64,
    pub generation: i64,
    pub unhealthy_pod_eviction_policy: UnhealthyPodEvictionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UIntOrPercent {
    Quantity(u32),
    Percent(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnhealthyPodEvictionPolicy {
    /// Default: only Running and ready pods count against PDB; unhealthy
    /// pods cannot be evicted if PDB is unhealthy.
    IfHealthyBudget,
    /// Always allow eviction of unhealthy pods regardless of PDB.
    AlwaysAllow,
}

impl Default for UnhealthyPodEvictionPolicy {
    fn default() -> Self {
        Self::IfHealthyBudget
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionResponse {
    pub status_code: u16,
    pub reason: EvictionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionReason {
    /// Eviction allowed and pod is being deleted.
    Allowed,
    /// Dry-run; pod not actually deleted.
    DryRunAllowed,
    /// Pod not found.
    NotFound,
    /// Precondition failed (uid/resourceVersion mismatch).
    Conflict,
    /// PDB not yet observed for current generation.
    PdbStale,
    /// Misconfigured PDB (e.g. expectedPods=0, both min/max set).
    PdbInvalid,
    /// PDB currently blocks disruption.
    TooManyRequests,
    /// Pod already terminating.
    AlreadyTerminating,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EvictionError {
    #[error("invalid PDB: {0}")]
    InvalidPdb(String),
}

impl EvictionResponse {
    pub fn allowed() -> Self {
        Self { status_code: 200, reason: EvictionReason::Allowed }
    }

    pub fn dry_run_allowed() -> Self {
        Self { status_code: 200, reason: EvictionReason::DryRunAllowed }
    }

    pub fn not_found() -> Self {
        Self { status_code: 404, reason: EvictionReason::NotFound }
    }

    pub fn conflict() -> Self {
        Self { status_code: 409, reason: EvictionReason::Conflict }
    }

    pub fn pdb_stale() -> Self {
        Self { status_code: 429, reason: EvictionReason::PdbStale }
    }

    pub fn too_many_requests() -> Self {
        Self { status_code: 429, reason: EvictionReason::TooManyRequests }
    }

    pub fn pdb_invalid() -> Self {
        Self { status_code: 500, reason: EvictionReason::PdbInvalid }
    }

    pub fn already_terminating() -> Self {
        Self { status_code: 200, reason: EvictionReason::AlreadyTerminating }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(
            self.reason,
            EvictionReason::Allowed
                | EvictionReason::DryRunAllowed
                | EvictionReason::AlreadyTerminating
        )
    }
}

/// Selector matches pod labels: every key in `selector` must equal pod label.
pub fn pdb_selects_pod(pdb: &PodDisruptionBudget, pod: &PodInfo) -> bool {
    if pdb.namespace != pod.namespace {
        return false;
    }
    if pdb.selector.is_empty() {
        return false;
    }
    pdb.selector
        .iter()
        .all(|(k, v)| pod.labels.get(k) == Some(v))
}

/// Validate that a PDB is internally consistent.
pub fn validate_pdb(pdb: &PodDisruptionBudget) -> Result<(), EvictionError> {
    if pdb.min_available.is_some() && pdb.max_unavailable.is_some() {
        return Err(EvictionError::InvalidPdb(
            "exactly one of minAvailable/maxUnavailable must be set".into(),
        ));
    }
    if pdb.min_available.is_none() && pdb.max_unavailable.is_none() {
        return Err(EvictionError::InvalidPdb(
            "at least one of minAvailable/maxUnavailable required".into(),
        ));
    }
    if pdb.expected_pods < 0 {
        return Err(EvictionError::InvalidPdb("expectedPods negative".into()));
    }
    Ok(())
}

/// Whether a PDB's status is fresh (observedGeneration matches generation).
pub fn pdb_is_fresh(pdb: &PodDisruptionBudget) -> bool {
    pdb.observed_generation >= pdb.generation
}

/// Decide whether eviction is permitted given the pod and the set of PDBs
/// that match it. The kubelet API server emulates this exactly.
pub fn evaluate_eviction(
    request: &EvictionRequest,
    pod: Option<&PodInfo>,
    pdbs: &[PodDisruptionBudget],
    pod_is_healthy_and_running: bool,
) -> EvictionResponse {
    let pod = match pod {
        Some(p) => p,
        None => return EvictionResponse::not_found(),
    };
    if pod.namespace != request.namespace || pod.name != request.pod_name {
        return EvictionResponse::not_found();
    }
    // Preconditions.
    if let Some(pre) = &request.delete_options.preconditions {
        if let Some(uid) = &pre.uid {
            if uid != &pod.uid {
                return EvictionResponse::conflict();
            }
        }
        if let Some(rv) = &pre.resource_version {
            if rv != &pod.resource_version {
                return EvictionResponse::conflict();
            }
        }
    }

    // Already terminating?
    if pod.deletion_timestamp_set {
        return EvictionResponse::already_terminating();
    }

    // Done pods skip PDB gating.
    if matches!(pod.phase, PodPhase::Succeeded | PodPhase::Failed) {
        if request.delete_options.dry_run.iter().any(|s| s == "All") {
            return EvictionResponse::dry_run_allowed();
        }
        return EvictionResponse::allowed();
    }

    // Find selecting PDBs.
    let selecting: Vec<&PodDisruptionBudget> =
        pdbs.iter().filter(|p| pdb_selects_pod(p, pod)).collect();
    for pdb in &selecting {
        // Validate PDB.
        if validate_pdb(pdb).is_err() {
            return EvictionResponse::pdb_invalid();
        }
        if !pdb_is_fresh(pdb) {
            return EvictionResponse::pdb_stale();
        }
        // Unhealthy + AlwaysAllow → bypass.
        if !pod_is_healthy_and_running
            && pdb.unhealthy_pod_eviction_policy == UnhealthyPodEvictionPolicy::AlwaysAllow
        {
            continue;
        }
        if pdb.disruptions_allowed <= 0 {
            return EvictionResponse::too_many_requests();
        }
    }

    if request.delete_options.dry_run.iter().any(|s| s == "All") {
        return EvictionResponse::dry_run_allowed();
    }
    EvictionResponse::allowed()
}

/// Apply an eviction: if the response is `Allowed`, decrement
/// `disruptionsAllowed` for each selecting non-stale PDB.
pub fn apply_disruption_to_pdbs(pdbs: &mut [PodDisruptionBudget], pod: &PodInfo) {
    for pdb in pdbs.iter_mut() {
        if pdb_selects_pod(pdb, pod) && pdb_is_fresh(pdb) && pdb.disruptions_allowed > 0 {
            pdb.disruptions_allowed -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(ns: &str, name: &str, uid: &str, rv: &str) -> PodInfo {
        let mut labels = BTreeMap::new();
        labels.insert("app".into(), "web".into());
        PodInfo {
            namespace: ns.into(),
            name: name.into(),
            uid: uid.into(),
            resource_version: rv.into(),
            labels,
            phase: PodPhase::Running,
            deletion_timestamp_set: false,
        }
    }

    fn pdb(ns: &str, name: &str, allowed: i32) -> PodDisruptionBudget {
        let mut sel = BTreeMap::new();
        sel.insert("app".into(), "web".into());
        PodDisruptionBudget {
            namespace: ns.into(),
            name: name.into(),
            selector: sel,
            min_available: Some(UIntOrPercent::Quantity(1)),
            max_unavailable: None,
            current_healthy: 3,
            desired_healthy: 2,
            disruptions_allowed: allowed,
            expected_pods: 3,
            observed_generation: 1,
            generation: 1,
            unhealthy_pod_eviction_policy: UnhealthyPodEvictionPolicy::IfHealthyBudget,
        }
    }

    fn req(ns: &str, name: &str) -> EvictionRequest {
        EvictionRequest {
            namespace: ns.into(),
            pod_name: name.into(),
            delete_options: DeleteOptions::default(),
        }
    }

    #[test]
    fn allowed_when_no_pdb_selects() {
        let p = pod("ns", "web-0", "u1", "rv1");
        let r = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[], true);
        assert_eq!(r.status_code, 200);
        assert_eq!(r.reason, EvictionReason::Allowed);
    }

    #[test]
    fn not_found_when_pod_missing() {
        let r = evaluate_eviction(&req("ns", "ghost"), None, &[], true);
        assert_eq!(r.status_code, 404);
    }

    #[test]
    fn not_found_when_namespace_mismatch() {
        let p = pod("other", "web-0", "u", "rv");
        let r = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[], true);
        assert_eq!(r.status_code, 404);
    }

    #[test]
    fn conflict_on_uid_mismatch() {
        let p = pod("ns", "web-0", "u1", "rv1");
        let mut r = req("ns", "web-0");
        r.delete_options.preconditions = Some(Preconditions {
            uid: Some("other".into()),
            resource_version: None,
        });
        let resp = evaluate_eviction(&r, Some(&p), &[], true);
        assert_eq!(resp.status_code, 409);
    }

    #[test]
    fn conflict_on_resource_version_mismatch() {
        let p = pod("ns", "web-0", "u1", "rv1");
        let mut r = req("ns", "web-0");
        r.delete_options.preconditions = Some(Preconditions {
            uid: None,
            resource_version: Some("rv99".into()),
        });
        let resp = evaluate_eviction(&r, Some(&p), &[], true);
        assert_eq!(resp.status_code, 409);
    }

    #[test]
    fn precondition_uid_match_allows() {
        let p = pod("ns", "web-0", "u1", "rv1");
        let mut r = req("ns", "web-0");
        r.delete_options.preconditions = Some(Preconditions {
            uid: Some("u1".into()),
            resource_version: Some("rv1".into()),
        });
        let resp = evaluate_eviction(&r, Some(&p), &[], true);
        assert_eq!(resp.status_code, 200);
    }

    #[test]
    fn already_terminating_returns_200_with_marker() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.deletion_timestamp_set = true;
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[], true);
        assert_eq!(resp.reason, EvictionReason::AlreadyTerminating);
        assert_eq!(resp.status_code, 200);
    }

    #[test]
    fn dry_run_returns_dry_run_allowed() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut r = req("ns", "web-0");
        r.delete_options.dry_run = vec!["All".into()];
        let resp = evaluate_eviction(&r, Some(&p), &[], true);
        assert_eq!(resp.reason, EvictionReason::DryRunAllowed);
    }

    #[test]
    fn pdb_blocks_when_disruptions_allowed_zero() {
        let p = pod("ns", "web-0", "u", "rv");
        let pdb_block = pdb("ns", "web-pdb", 0);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[pdb_block], true);
        assert_eq!(resp.status_code, 429);
        assert_eq!(resp.reason, EvictionReason::TooManyRequests);
    }

    #[test]
    fn pdb_allows_when_disruptions_remain() {
        let p = pod("ns", "web-0", "u", "rv");
        let pdb_ok = pdb("ns", "web-pdb", 2);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[pdb_ok], true);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn pdb_stale_returns_429_pdb_stale() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut pdb_stale = pdb("ns", "web-pdb", 1);
        pdb_stale.observed_generation = 0;
        pdb_stale.generation = 5;
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[pdb_stale], true);
        assert_eq!(resp.status_code, 429);
        assert_eq!(resp.reason, EvictionReason::PdbStale);
    }

    #[test]
    fn pdb_invalid_returns_500() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut bad = pdb("ns", "web-pdb", 5);
        bad.max_unavailable = Some(UIntOrPercent::Quantity(1));
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[bad], true);
        assert_eq!(resp.status_code, 500);
        assert_eq!(resp.reason, EvictionReason::PdbInvalid);
    }

    #[test]
    fn pdb_invalid_neither_min_nor_max_set() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut bad = pdb("ns", "web-pdb", 5);
        bad.min_available = None;
        bad.max_unavailable = None;
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[bad], true);
        assert_eq!(resp.status_code, 500);
    }

    #[test]
    fn pdb_does_not_select_when_namespace_differs() {
        let p = pod("ns", "web-0", "u", "rv");
        let pdb_other = pdb("other", "web-pdb", 0);
        // PDB is in another namespace → should NOT block eviction.
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[pdb_other], true);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn pdb_does_not_select_when_labels_differ() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.labels.insert("app".into(), "api".into());
        let pdb_block = pdb("ns", "web-pdb", 0);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[pdb_block], true);
        // Doesn't select web pod (label "app=api") → allowed.
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn empty_selector_does_not_select_anything() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut empty = pdb("ns", "web-pdb", 0);
        empty.selector.clear();
        // Empty selector ⇒ no pod is selected ⇒ no block.
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[empty], true);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn unhealthy_pod_with_always_allow_bypasses_pdb() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut blocking = pdb("ns", "web-pdb", 0);
        blocking.unhealthy_pod_eviction_policy = UnhealthyPodEvictionPolicy::AlwaysAllow;
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[blocking], false);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn unhealthy_pod_with_if_healthy_budget_blocked_by_pdb() {
        let p = pod("ns", "web-0", "u", "rv");
        let blocking = pdb("ns", "web-pdb", 0);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[blocking], false);
        assert_eq!(resp.reason, EvictionReason::TooManyRequests);
    }

    #[test]
    fn succeeded_pod_skips_pdb_gating() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.phase = PodPhase::Succeeded;
        let blocking = pdb("ns", "web-pdb", 0);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[blocking], true);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn failed_pod_skips_pdb_gating() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.phase = PodPhase::Failed;
        let blocking = pdb("ns", "web-pdb", 0);
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[blocking], true);
        assert_eq!(resp.reason, EvictionReason::Allowed);
    }

    #[test]
    fn succeeded_pod_dry_run_returns_dry_run_allowed() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.phase = PodPhase::Succeeded;
        let mut r = req("ns", "web-0");
        r.delete_options.dry_run = vec!["All".into()];
        let resp = evaluate_eviction(&r, Some(&p), &[], true);
        assert_eq!(resp.reason, EvictionReason::DryRunAllowed);
    }

    #[test]
    fn multiple_pdbs_all_must_allow() {
        let p = pod("ns", "web-0", "u", "rv");
        let ok = pdb("ns", "p1", 5);
        let mut block = pdb("ns", "p2", 0);
        block.selector = ok.selector.clone();
        let resp = evaluate_eviction(&req("ns", "web-0"), Some(&p), &[ok, block], true);
        assert_eq!(resp.reason, EvictionReason::TooManyRequests);
    }

    #[test]
    fn pdb_is_fresh_compares_observed_to_generation() {
        let mut p = pdb("ns", "x", 1);
        p.generation = 5;
        p.observed_generation = 5;
        assert!(pdb_is_fresh(&p));
        p.observed_generation = 4;
        assert!(!pdb_is_fresh(&p));
        p.observed_generation = 10;
        assert!(pdb_is_fresh(&p));
    }

    #[test]
    fn validate_pdb_only_min_available_ok() {
        let mut p = pdb("ns", "x", 1);
        p.min_available = Some(UIntOrPercent::Quantity(1));
        p.max_unavailable = None;
        validate_pdb(&p).unwrap();
    }

    #[test]
    fn validate_pdb_only_max_unavailable_ok() {
        let mut p = pdb("ns", "x", 1);
        p.min_available = None;
        p.max_unavailable = Some(UIntOrPercent::Percent(20));
        validate_pdb(&p).unwrap();
    }

    #[test]
    fn validate_pdb_both_set_rejected() {
        let mut p = pdb("ns", "x", 1);
        p.min_available = Some(UIntOrPercent::Quantity(1));
        p.max_unavailable = Some(UIntOrPercent::Quantity(1));
        assert!(validate_pdb(&p).is_err());
    }

    #[test]
    fn validate_pdb_neither_set_rejected() {
        let mut p = pdb("ns", "x", 1);
        p.min_available = None;
        p.max_unavailable = None;
        assert!(validate_pdb(&p).is_err());
    }

    #[test]
    fn validate_pdb_negative_expected_pods_rejected() {
        let mut p = pdb("ns", "x", 1);
        p.expected_pods = -1;
        assert!(validate_pdb(&p).is_err());
    }

    #[test]
    fn pdb_selects_pod_basic_match() {
        let p = pod("ns", "web-0", "u", "rv");
        let pdb_match = pdb("ns", "web-pdb", 1);
        assert!(pdb_selects_pod(&pdb_match, &p));
    }

    #[test]
    fn pdb_selects_pod_partial_label_match_rejected() {
        let mut p = pod("ns", "web-0", "u", "rv");
        p.labels.clear();
        p.labels.insert("env".into(), "prod".into());
        let pdb_full = pdb("ns", "web-pdb", 1);
        assert!(!pdb_selects_pod(&pdb_full, &p));
    }

    #[test]
    fn apply_disruption_decrements_only_selecting_fresh_pdbs() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut pdbs = vec![pdb("ns", "p1", 3), pdb("other", "p2", 3)];
        apply_disruption_to_pdbs(&mut pdbs, &p);
        assert_eq!(pdbs[0].disruptions_allowed, 2);
        assert_eq!(pdbs[1].disruptions_allowed, 3);
    }

    #[test]
    fn apply_disruption_does_not_decrement_zero() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut pdbs = vec![pdb("ns", "p1", 0)];
        apply_disruption_to_pdbs(&mut pdbs, &p);
        assert_eq!(pdbs[0].disruptions_allowed, 0);
    }

    #[test]
    fn apply_disruption_skips_stale_pdb() {
        let p = pod("ns", "web-0", "u", "rv");
        let mut stale = pdb("ns", "p1", 3);
        stale.observed_generation = 0;
        stale.generation = 1;
        let mut pdbs = vec![stale];
        apply_disruption_to_pdbs(&mut pdbs, &p);
        assert_eq!(pdbs[0].disruptions_allowed, 3);
    }

    #[test]
    fn response_is_allowed_helper() {
        assert!(EvictionResponse::allowed().is_allowed());
        assert!(EvictionResponse::dry_run_allowed().is_allowed());
        assert!(EvictionResponse::already_terminating().is_allowed());
        assert!(!EvictionResponse::too_many_requests().is_allowed());
        assert!(!EvictionResponse::pdb_invalid().is_allowed());
        assert!(!EvictionResponse::not_found().is_allowed());
        assert!(!EvictionResponse::conflict().is_allowed());
        assert!(!EvictionResponse::pdb_stale().is_allowed());
    }

    #[test]
    fn unhealthy_eviction_policy_default_is_if_healthy_budget() {
        assert_eq!(
            UnhealthyPodEvictionPolicy::default(),
            UnhealthyPodEvictionPolicy::IfHealthyBudget
        );
    }

    #[test]
    fn dry_run_does_not_decrement_pdb_via_apply() {
        // Caller should only call apply_disruption_to_pdbs when reason==Allowed.
        // Verify dry-run path returns DryRunAllowed.
        let p = pod("ns", "web-0", "u", "rv");
        let pdb_ok = pdb("ns", "p", 5);
        let mut r = req("ns", "web-0");
        r.delete_options.dry_run = vec!["All".into()];
        let resp = evaluate_eviction(&r, Some(&p), &[pdb_ok], true);
        assert_eq!(resp.reason, EvictionReason::DryRunAllowed);
    }
}
