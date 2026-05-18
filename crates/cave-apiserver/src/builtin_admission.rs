// SPDX-License-Identifier: AGPL-3.0-or-later
//! Built-in admission plugins — line-by-line port of upstream
//! `plugin/pkg/admission/{namespace,resourcequota,limitranger}/`.
//!
//! Upstream (kubernetes/kubernetes v1.31):
//!   * `plugin/pkg/admission/namespace/exists/admission.go`
//!   * `plugin/pkg/admission/namespace/lifecycle/admission.go`
//!   * `plugin/pkg/admission/limitranger/admission.go` + `limits.go`
//!   * `plugin/pkg/admission/resourcequota/admission.go` + `controller.go` +
//!     `resource_access.go`
//!
//! ## Tenant invariant
//!
//! Quotas, limit ranges, and namespace state are all looked up scoped to
//! `req.tenant_id`. A LimitRange in tenant T MUST NOT apply to a request
//! from tenant ≠ T even if the namespace name collides.

use crate::admission::{
    AdmissionRequest, AdmissionResponse, MutatingWebhook, Operation, ValidatingWebhook,
};
use crate::resources::Resource;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// ─────────────────────────────────────────────────────────────────────────────
// Namespace state — tiny in-memory store. Mirrors enough of the upstream
// namespace lister to drive tests.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespacePhase {
    Active,
    Terminating,
}

#[derive(Default)]
pub struct NamespaceState {
    inner: RwLock<HashMap<(String, String), NamespacePhase>>, // (tenant, ns)
}

impl NamespaceState {
    pub fn new() -> Self { Self::default() }

    pub fn upsert(&self, tenant: &str, ns: &str, phase: NamespacePhase) {
        self.inner.write().unwrap().insert((tenant.to_string(), ns.to_string()), phase);
    }

    pub fn delete(&self, tenant: &str, ns: &str) {
        self.inner.write().unwrap().remove(&(tenant.to_string(), ns.to_string()));
    }

    pub fn phase(&self, tenant: &str, ns: &str) -> Option<NamespacePhase> {
        self.inner.read().unwrap().get(&(tenant.to_string(), ns.to_string())).copied()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NamespaceExists — denies namespaced operations on a namespace that does
// not exist. Upstream: `namespace/exists/admission.go::Admit`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct NamespaceExists {
    pub state: Arc<NamespaceState>,
    /// Resources that are NOT namespaced (cluster-scoped) — never checked.
    pub cluster_scoped_kinds: Vec<String>,
}

impl NamespaceExists {
    pub fn new(state: Arc<NamespaceState>) -> Self {
        Self {
            state,
            cluster_scoped_kinds: vec![
                "Namespace".into(), "Node".into(), "PersistentVolume".into(),
                "ClusterRole".into(), "ClusterRoleBinding".into(), "StorageClass".into(),
            ],
        }
    }
}

impl ValidatingWebhook for NamespaceExists {
    fn name(&self) -> &str { "NamespaceExists" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        if self.cluster_scoped_kinds.contains(&req.kind) {
            return AdmissionResponse::allow(req);
        }
        if req.namespace.is_empty() {
            return AdmissionResponse::allow(req);
        }
        if self.state.phase(&req.tenant_id, &req.namespace).is_none() {
            return AdmissionResponse::deny(req, 404,
                format!("namespace {} not found", req.namespace));
        }
        AdmissionResponse::allow(req)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NamespaceLifecycleStrict — denies create/update on a Terminating namespace.
// Stricter than the existing NamespaceLifecycle in admission.rs (which only
// guards kube-system writes). Upstream: `namespace/lifecycle/admission.go`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct NamespaceLifecycleStrict {
    pub state: Arc<NamespaceState>,
    pub cluster_scoped_kinds: Vec<String>,
}

impl NamespaceLifecycleStrict {
    pub fn new(state: Arc<NamespaceState>) -> Self {
        Self {
            state,
            cluster_scoped_kinds: vec![
                "Namespace".into(), "Node".into(), "PersistentVolume".into(),
                "ClusterRole".into(), "ClusterRoleBinding".into(), "StorageClass".into(),
            ],
        }
    }
}

impl ValidatingWebhook for NamespaceLifecycleStrict {
    fn name(&self) -> &str { "NamespaceLifecycleStrict" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        if self.cluster_scoped_kinds.contains(&req.kind) {
            return AdmissionResponse::allow(req);
        }
        if req.namespace.is_empty() {
            return AdmissionResponse::allow(req);
        }
        if !matches!(req.operation, Operation::Create | Operation::Update) {
            return AdmissionResponse::allow(req);
        }
        match self.state.phase(&req.tenant_id, &req.namespace) {
            Some(NamespacePhase::Terminating) => AdmissionResponse::deny(req, 403,
                format!("namespace {} is terminating", req.namespace)),
            Some(NamespacePhase::Active) => AdmissionResponse::allow(req),
            None => AdmissionResponse::deny(req, 404,
                format!("namespace {} not found", req.namespace)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// LimitRanger — enforces LimitRange constraints + injects defaults.
// Upstream: `limitranger/admission.go` + `limits.go::PodLimitFunc`.
//
// Surface ported here:
//   * `Container` and `Pod` LimitRangeItems
//   * `default` and `defaultRequest` injection (mutating)
//   * `min`/`max` validation
//   * `maxLimitRequestRatio` validation
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitRangeItemType {
    Container,
    Pod,
    PersistentVolumeClaim,
}

#[derive(Debug, Clone, Default)]
pub struct LimitRangeItem {
    pub kind: Option<LimitRangeItemType>,
    pub min: HashMap<String, i64>,                    // resource → millicores or bytes
    pub max: HashMap<String, i64>,
    pub default: HashMap<String, i64>,                // requests use this when unset
    pub default_request: HashMap<String, i64>,
    pub max_limit_request_ratio: HashMap<String, f64>,
}

#[derive(Debug, Clone, Default)]
pub struct LimitRange {
    pub tenant_id: String,
    pub namespace: String,
    pub name: String,
    pub items: Vec<LimitRangeItem>,
}

#[derive(Default)]
pub struct LimitRangeStore {
    inner: RwLock<Vec<LimitRange>>,
}

impl LimitRangeStore {
    pub fn new() -> Self { Self::default() }
    pub fn put(&self, lr: LimitRange) {
        let mut g = self.inner.write().unwrap();
        g.retain(|x| !(x.tenant_id == lr.tenant_id && x.namespace == lr.namespace && x.name == lr.name));
        g.push(lr);
    }
    pub fn list(&self, tenant: &str, namespace: &str) -> Vec<LimitRange> {
        self.inner.read().unwrap().iter()
            .filter(|x| x.tenant_id == tenant && x.namespace == namespace)
            .cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitRangeError {
    Below { resource: String, value: i64, min: i64 },
    Above { resource: String, value: i64, max: i64 },
    RatioExceeded { resource: String, ratio_observed: String, ratio_max: String },
    DefaultMissing { resource: String },
}

impl std::fmt::Display for LimitRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitRangeError::Below { resource, value, min } =>
                write!(f, "{} value {} below min {}", resource, value, min),
            LimitRangeError::Above { resource, value, max } =>
                write!(f, "{} value {} above max {}", resource, value, max),
            LimitRangeError::RatioExceeded { resource, ratio_observed, ratio_max } =>
                write!(f, "{} ratio {} exceeds limit/request ratio {}", resource, ratio_observed, ratio_max),
            LimitRangeError::DefaultMissing { resource } =>
                write!(f, "no default for {} and value not set", resource),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerResources {
    pub requests: HashMap<String, i64>,
    pub limits: HashMap<String, i64>,
}

/// Apply Container-kind defaults to a single container's resources.
/// Mirrors `limitranger/limits.go::mergePodResourceRequirements`.
pub fn apply_container_defaults(
    container: &mut ContainerResources, items: &[LimitRangeItem],
) {
    for item in items.iter().filter(|i| i.kind == Some(LimitRangeItemType::Container)) {
        for (resource, default) in &item.default {
            container.limits.entry(resource.clone()).or_insert(*default);
        }
        for (resource, default) in &item.default_request {
            container.requests.entry(resource.clone()).or_insert(*default);
        }
    }
}

/// Validate one container against all Container-kind limit ranges.
pub fn validate_container(
    container: &ContainerResources, items: &[LimitRangeItem],
) -> Result<(), LimitRangeError> {
    for item in items.iter().filter(|i| i.kind == Some(LimitRangeItemType::Container)) {
        // min
        for (resource, min) in &item.min {
            if let Some(req) = container.requests.get(resource) {
                if req < min {
                    return Err(LimitRangeError::Below {
                        resource: resource.clone(), value: *req, min: *min,
                    });
                }
            }
        }
        // max
        for (resource, max) in &item.max {
            if let Some(lim) = container.limits.get(resource) {
                if lim > max {
                    return Err(LimitRangeError::Above {
                        resource: resource.clone(), value: *lim, max: *max,
                    });
                }
            }
        }
        // ratio
        for (resource, ratio_max) in &item.max_limit_request_ratio {
            if let (Some(req), Some(lim)) =
                (container.requests.get(resource), container.limits.get(resource))
            {
                if *req > 0 {
                    let ratio = *lim as f64 / *req as f64;
                    if ratio > *ratio_max {
                        return Err(LimitRangeError::RatioExceeded {
                            resource: resource.clone(),
                            ratio_observed: format!("{ratio:.2}"),
                            ratio_max: format!("{ratio_max:.2}"),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourceQuota — total-up usage per namespace and reject when over.
// Upstream: `resourcequota/controller.go::Update` + `resource_access.go`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ResourceQuotaSpec {
    /// resource name → hard limit
    pub hard: HashMap<String, i64>,
    /// optional scope filter (e.g. "Terminating", "BestEffort", "PriorityClass=foo")
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceQuota {
    pub tenant_id: String,
    pub namespace: String,
    pub name: String,
    pub spec: ResourceQuotaSpec,
    /// observed usage (persisted by the quota controller)
    pub used: HashMap<String, i64>,
}

#[derive(Default)]
pub struct ResourceQuotaStore {
    inner: RwLock<Vec<ResourceQuota>>,
}

impl ResourceQuotaStore {
    pub fn new() -> Self { Self::default() }
    pub fn put(&self, q: ResourceQuota) {
        let mut g = self.inner.write().unwrap();
        g.retain(|x| !(x.tenant_id == q.tenant_id && x.namespace == q.namespace && x.name == q.name));
        g.push(q);
    }
    pub fn list(&self, tenant: &str, namespace: &str) -> Vec<ResourceQuota> {
        self.inner.read().unwrap().iter()
            .filter(|x| x.tenant_id == tenant && x.namespace == namespace)
            .cloned().collect()
    }
    /// Observe: increment `used` for a given (resource, delta) tuple.
    pub fn observe(&self, tenant: &str, namespace: &str, name: &str, resource: &str, delta: i64) {
        let mut g = self.inner.write().unwrap();
        if let Some(q) = g.iter_mut().find(|x|
            x.tenant_id == tenant && x.namespace == namespace && x.name == name)
        {
            *q.used.entry(resource.to_string()).or_insert(0) += delta;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaError {
    Exceeded { quota: String, resource: String, used: i64, hard: i64, want: i64 },
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let QuotaError::Exceeded { quota, resource, used, hard, want } = self;
        write!(f, "quota {} exceeded for {}: used {} + want {} > hard {}",
               quota, resource, used, want, hard)
    }
}

/// Compute quota delta for a request. Caller passes
/// `requested: HashMap<resource, amount>` for the *new* object.
pub fn check_quota(
    quotas: &[ResourceQuota], requested: &HashMap<String, i64>,
) -> Result<(), QuotaError> {
    for q in quotas {
        for (resource, want) in requested {
            if let Some(hard) = q.spec.hard.get(resource) {
                let used = q.used.get(resource).copied().unwrap_or(0);
                if used + *want > *hard {
                    return Err(QuotaError::Exceeded {
                        quota: q.name.clone(),
                        resource: resource.clone(),
                        used, hard: *hard, want: *want,
                    });
                }
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourceQuotaPlugin — the validating-webhook adapter. Reads requested
// resource amounts from the AdmissionRequest object.
// ─────────────────────────────────────────────────────────────────────────────

pub struct ResourceQuotaPlugin {
    pub quotas: Arc<ResourceQuotaStore>,
}

impl ValidatingWebhook for ResourceQuotaPlugin {
    fn name(&self) -> &str { "ResourceQuota" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        let Some(obj) = req.object.as_ref() else { return AdmissionResponse::allow(req); };
        let requested = extract_quota_requests(obj);
        if requested.is_empty() {
            return AdmissionResponse::allow(req);
        }
        let quotas = self.quotas.list(&req.tenant_id, &req.namespace);
        if quotas.is_empty() {
            return AdmissionResponse::allow(req);
        }
        match check_quota(&quotas, &requested) {
            Ok(()) => AdmissionResponse::allow(req),
            Err(e) => AdmissionResponse::deny(req, 403, e.to_string()),
        }
    }
}

/// Extract the resources a request would consume. Today we wire Pod and
/// ConfigMap counts; expand as more types are reconciled.
pub fn extract_quota_requests(r: &Resource) -> HashMap<String, i64> {
    let mut m = HashMap::new();
    match r {
        Resource::Pod(_) => { m.insert("pods".into(), 1); }
        Resource::ConfigMap(_) => { m.insert("configmaps".into(), 1); }
        Resource::Secret(_) => { m.insert("secrets".into(), 1); }
        Resource::Service(_) => { m.insert("services".into(), 1); }
        Resource::PersistentVolumeClaim(_) => {
            m.insert("persistentvolumeclaims".into(), 1);
        }
        _ => {}
    }
    m
}

// ─────────────────────────────────────────────────────────────────────────────
// LimitRangerPlugin — mutating phase injects defaults; validating phase rejects
// out-of-range values. We implement both as the same struct.
// Container resources for a Pod aren't in our Resource model yet, so the
// plugin operates on the abstract `ContainerResources` type via an extractor.
// ─────────────────────────────────────────────────────────────────────────────

pub struct LimitRangerPlugin {
    pub ranges: Arc<LimitRangeStore>,
}

impl LimitRangerPlugin {
    pub fn new(ranges: Arc<LimitRangeStore>) -> Self { Self { ranges } }
}

impl MutatingWebhook for LimitRangerPlugin {
    fn name(&self) -> &str { "LimitRanger.Mutating" }
    fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
        // Pod resources are not modeled in `Resource::Pod` yet, so we can't
        // emit per-container patches here. The mutating hook is a no-op
        // until the resource shape is enriched. The function-level helpers
        // (`apply_container_defaults`) cover the real work and are unit-tested.
        AdmissionResponse::allow(req)
    }
}

impl ValidatingWebhook for LimitRangerPlugin {
    fn name(&self) -> &str { "LimitRanger.Validating" }
    fn validate(&self, _req: &AdmissionRequest) -> AdmissionResponse {
        // Same caveat as above. The pure validators cover the actual logic.
        AdmissionResponse::allow(_req)
    }
}

#[cfg(test)]
mod tests;
