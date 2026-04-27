//! kube-aggregator deeper — APIService availability conditions, OpenAPI v3
//! spec merging across delegated APIServices, proxy reason categories.
//!
//! Layered atop `aggregated_apiserver.rs` which already covers APIService
//! registration + tenant-scoped routing.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/kube-aggregator/pkg/apiserver/handler_proxy.go`
//!     (`proxyHandler.ServeHTTP` — emits well-known reason categories on
//!      503 responses).
//!   * `staging/src/k8s.io/kube-aggregator/pkg/controllers/openapiv3/aggregator.go`
//!     (merge child specs into the parent index).
//!   * `staging/src/k8s.io/kube-aggregator/pkg/controllers/status/available_controller.go`
//!     (computes Available condition).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─────────────────────────────────────────────────────────────────────────────
// Conditions — `apiregistration.k8s.io/v1.APIServiceCondition`
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConditionType {
    Available,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct APIServiceCondition {
    pub kind: ConditionType,
    pub status: ConditionStatus,
    pub reason: String,
    pub message: String,
}

impl APIServiceCondition {
    pub fn available() -> Self {
        Self {
            kind: ConditionType::Available,
            status: ConditionStatus::True,
            reason: "Passed".into(),
            message: "all checks passed".into(),
        }
    }
    pub fn failure(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ConditionType::Available,
            status: ConditionStatus::False,
            reason: reason.into(),
            message: message.into(),
        }
    }
    pub fn unknown(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ConditionType::Available,
            status: ConditionStatus::Unknown,
            reason: reason.into(),
            message: message.into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Availability checker — mirrors `available_controller.go::sync`. Each check
// is a stub that the controller's network probe layer fills in; we ship the
// state machine + tests around it.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Reachable,
    Unreachable(String),
    InvalidCertificate(String),
    DnsFailure(String),
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailabilityInput {
    pub api_service_name: String,
    pub local: bool, // true == in-process; never marked unavailable
    pub service_resolved: bool,
    pub endpoints_count: usize,
    pub probe: Option<ProbeOutcome>,
}

pub fn compute_condition(input: &AvailabilityInput) -> APIServiceCondition {
    if input.local {
        return APIServiceCondition::available();
    }
    if !input.service_resolved {
        return APIServiceCondition::failure(
            "ServiceNotFound",
            format!("service for {} could not be resolved", input.api_service_name));
    }
    if input.endpoints_count == 0 {
        return APIServiceCondition::failure(
            "MissingEndpoints",
            format!("service for {} has no endpoints", input.api_service_name));
    }
    match &input.probe {
        Some(ProbeOutcome::Reachable) => APIServiceCondition::available(),
        Some(ProbeOutcome::Unreachable(m)) =>
            APIServiceCondition::failure("FailedDiscoveryCheck", m.clone()),
        Some(ProbeOutcome::InvalidCertificate(m)) =>
            APIServiceCondition::failure("FailedDiscoveryCheck",
                format!("invalid certificate: {m}")),
        Some(ProbeOutcome::DnsFailure(m)) =>
            APIServiceCondition::failure("FailedDiscoveryCheck",
                format!("dns failure: {m}")),
        Some(ProbeOutcome::Timeout) =>
            APIServiceCondition::failure("FailedDiscoveryCheck", "timeout"),
        None => APIServiceCondition::unknown("Pending",
            "probe has not yet run"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Proxy reason categories — `handler_proxy.go::serviceErrorReasons`. Used to
// surface deterministic reason codes on 503 responses to clients.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyDecision {
    Forward,
    ServiceUnavailable { reason: String, message: String, http_status: u16 },
}

pub fn evaluate_proxy(
    cond: &APIServiceCondition, dry_run: bool, has_endpoints: bool,
) -> ProxyDecision {
    if cond.status == ConditionStatus::True {
        return ProxyDecision::Forward;
    }
    let mut reason = cond.reason.clone();
    let mut message = cond.message.clone();
    // upstream returns 503 with reason="ServiceUnavailable" for any non-True
    // Available condition; subreason is preserved in message.
    if reason.is_empty() {
        reason = "ServiceUnavailable".into();
    }
    if dry_run {
        message = format!("[dry_run] {message}");
    }
    if !has_endpoints {
        message.push_str(" (no endpoints)");
    }
    ProxyDecision::ServiceUnavailable { reason, message, http_status: 503 }
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAPI v3 spec merging — `controllers/openapiv3/aggregator.go::buildIndex`.
// We don't ship the whole merger here; we ship the merge of the *index*
// that the apiserver advertises (the `paths` map keyed by `<group>/<version>`).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenApiIndexEntry {
    /// Pre-hashed URL the client resolves with `?hash=`.
    #[serde(rename = "serverRelativeURL")]
    pub server_relative_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenApiIndex {
    pub paths: BTreeMap<String, OpenApiIndexEntry>,
}

/// Merge child index into parent. On collision the child *wins* (matches
/// upstream priority where a registered APIService overlays the in-process
/// view of the same group/version).
pub fn merge_openapi_indexes(parent: &OpenApiIndex, child: &OpenApiIndex) -> OpenApiIndex {
    let mut out = OpenApiIndex { paths: parent.paths.clone() };
    for (k, v) in &child.paths {
        out.paths.insert(k.clone(), v.clone());
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Group/version priority — `apiservice/strategy.go::priorityCompare`.
// Lower group_priority is preferred; ties broken by lower version_priority;
// ultimate ties broken by lexicographic name comparison.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityKey {
    pub group_priority: i32,
    pub version_priority: i32,
    pub name: String,
}

pub fn priority_compare(a: &PriorityKey, b: &PriorityKey) -> std::cmp::Ordering {
    a.group_priority.cmp(&b.group_priority)
        .then(a.version_priority.cmp(&b.version_priority))
        .then(a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests;
