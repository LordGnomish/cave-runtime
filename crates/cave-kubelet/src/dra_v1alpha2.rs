// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dynamic Resource Allocation (DRA) — KEP-3063 + KEP-4381 (structured params).
//!
//! Mirrors `pkg/kubelet/cm/dra` and the upstream `resource.k8s.io/v1alpha2`
//! types: ResourceClass, ResourceClaim (with template instantiation),
//! AllocationResult, PodSchedulingContext, ResourceSlice (for structured
//! parameters / KEP-4381). Captures the full claim lifecycle:
//! pending → allocated → reserved-for-pod → in-use → released, including
//! deallocation requests, multi-pod claim sharing semantics, and the
//! NodeSelector that ties an allocation to a node.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClass {
    pub name: String,
    pub driver_name: String,
    pub parameters_ref: Option<ParametersRef>,
    pub suitable_nodes: Option<NodeSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParametersRef {
    pub api_group: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSelector {
    /// node name → must match.
    pub node_names: Vec<String>,
    /// label selector terms, ANDed within a term, ORed across terms.
    pub match_labels: Vec<BTreeMap<String, String>>,
}

impl NodeSelector {
    pub fn matches(&self, node_name: &str, labels: &BTreeMap<String, String>) -> bool {
        if !self.node_names.is_empty() && !self.node_names.iter().any(|n| n == node_name) {
            return false;
        }
        if self.match_labels.is_empty() {
            return true;
        }
        for term in &self.match_labels {
            if term.iter().all(|(k, v)| labels.get(k) == Some(v)) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllocationMode {
    /// Single allocation immediately when claim is created.
    Immediate,
    /// Wait for first consumer (default) — allocation deferred until a pod refs the claim.
    WaitForFirstConsumer,
}

impl Default for AllocationMode {
    fn default() -> Self {
        Self::WaitForFirstConsumer
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClaimSpec {
    pub resource_class_name: String,
    pub allocation_mode: AllocationMode,
    pub parameters_ref: Option<ParametersRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationResult {
    pub driver_name: String,
    pub resource_handles: Vec<ResourceHandle>,
    pub available_on_nodes: Option<NodeSelector>,
    pub shareable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceHandle {
    pub driver_name: String,
    pub data: String,
    pub structured_data: Option<StructuredResourceHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredResourceHandle {
    pub vendor_class_parameters: BTreeMap<String, String>,
    pub vendor_claim_parameters: BTreeMap<String, String>,
    pub node_name: String,
    pub results: Vec<DriverAllocationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverAllocationResult {
    pub request_name: String,
    pub vendor_request_parameters: BTreeMap<String, String>,
    pub allocated_devices: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimPhase {
    Pending,
    Allocated,
    /// Allocated AND reserved for at least one pod — kubelet has set up the resource.
    InUse,
    /// Deallocation requested; will release once consumers drop.
    Deallocating,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClaim {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub spec: ResourceClaimSpec,
    pub allocation: Option<AllocationResult>,
    pub phase: ClaimPhase,
    pub reserved_for: Vec<ConsumerReference>,
    pub deallocation_requested: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ConsumerReference {
    pub api_group: String,
    pub resource: String,
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClaimTemplate {
    pub name: String,
    pub namespace: String,
    pub spec_template: ResourceClaimSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodResourceClaim {
    pub name: String,
    pub source: ClaimSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimSource {
    /// Pod references an existing ResourceClaim by name.
    Existing(String),
    /// Pod references a template; one ResourceClaim is generated per pod.
    Template(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSlice {
    pub name: String,
    pub node_name: String,
    pub driver_name: String,
    pub pool_name: String,
    pub devices: Vec<ResourceDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDevice {
    pub name: String,
    pub attributes: BTreeMap<String, AttributeValue>,
    pub capacity: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributeValue {
    Bool(bool),
    Int(i64),
    String(String),
    Version(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DraError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not allocatable on node: {0}")]
    NotAllocatable(String),
    #[error("already allocated: {0}")]
    AlreadyAllocated(String),
}

pub type DraResult<T> = Result<T, DraError>;

#[derive(Debug, Default)]
pub struct DraManager {
    classes: DashMap<String, ResourceClass>,
    claims: DashMap<String, ResourceClaim>,
    templates: DashMap<String, ResourceClaimTemplate>,
    slices: DashMap<String, ResourceSlice>,
    /// Per-pod claim-name → claim-uid binding (for template-instantiated claims).
    pod_claim_bindings: DashMap<(String, String), String>,
}

impl DraManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── ResourceClass ────────────────────────────────────────────────────

    pub fn create_class(&self, class: ResourceClass) -> DraResult<()> {
        if class.name.is_empty() || class.driver_name.is_empty() {
            return Err(DraError::Invalid(
                "ResourceClass requires name and driverName".into(),
            ));
        }
        self.classes.insert(class.name.clone(), class);
        Ok(())
    }

    pub fn delete_class(&self, name: &str) -> DraResult<()> {
        self.classes.remove(name);
        Ok(())
    }

    pub fn get_class(&self, name: &str) -> Option<ResourceClass> {
        self.classes.get(name).map(|r| r.value().clone())
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    // ── ResourceClaimTemplate ────────────────────────────────────────────

    pub fn create_template(&self, template: ResourceClaimTemplate) -> DraResult<()> {
        if template.name.is_empty() {
            return Err(DraError::Invalid("template name empty".into()));
        }
        if template.spec_template.resource_class_name.is_empty() {
            return Err(DraError::Invalid("template references no class".into()));
        }
        let key = format!("{}/{}", template.namespace, template.name);
        self.templates.insert(key, template);
        Ok(())
    }

    pub fn get_template(&self, namespace: &str, name: &str) -> Option<ResourceClaimTemplate> {
        let key = format!("{}/{}", namespace, name);
        self.templates.get(&key).map(|r| r.value().clone())
    }

    /// Instantiate a per-pod claim from a template; returns the new claim uid.
    pub fn instantiate_template_for_pod(
        &self,
        template_namespace: &str,
        template_name: &str,
        pod_uid: &str,
        pod_claim_name: &str,
    ) -> DraResult<String> {
        let key = (pod_uid.to_string(), pod_claim_name.to_string());
        if let Some(existing) = self.pod_claim_bindings.get(&key) {
            return Ok(existing.value().clone());
        }
        let tpl = self.get_template(template_namespace, template_name).ok_or_else(|| {
            DraError::NotFound(format!(
                "template {}/{} not found",
                template_namespace, template_name
            ))
        })?;
        if !self.classes.contains_key(&tpl.spec_template.resource_class_name) {
            return Err(DraError::NotFound(format!(
                "class {} referenced by template not found",
                tpl.spec_template.resource_class_name
            )));
        }
        let claim_uid = format!(
            "claim-{}-{}-{}",
            pod_uid, pod_claim_name, &tpl.name
        );
        let claim = ResourceClaim {
            name: format!("{}-{}", pod_claim_name, pod_uid),
            namespace: template_namespace.to_string(),
            uid: claim_uid.clone(),
            spec: tpl.spec_template.clone(),
            allocation: None,
            phase: ClaimPhase::Pending,
            reserved_for: Vec::new(),
            deallocation_requested: false,
            created_at: Utc::now(),
        };
        self.claims.insert(claim_uid.clone(), claim);
        self.pod_claim_bindings.insert(key, claim_uid.clone());
        Ok(claim_uid)
    }

    // ── ResourceClaim ────────────────────────────────────────────────────

    pub fn create_claim(&self, claim: ResourceClaim) -> DraResult<()> {
        if claim.uid.is_empty() {
            return Err(DraError::Invalid("claim uid empty".into()));
        }
        if !self.classes.contains_key(&claim.spec.resource_class_name) {
            return Err(DraError::NotFound(format!(
                "class {} not found",
                claim.spec.resource_class_name
            )));
        }
        if self.claims.contains_key(&claim.uid) {
            return Err(DraError::Conflict(format!("claim {} exists", claim.uid)));
        }
        self.claims.insert(claim.uid.clone(), claim);
        Ok(())
    }

    pub fn get_claim(&self, uid: &str) -> Option<ResourceClaim> {
        self.claims.get(uid).map(|r| r.value().clone())
    }

    pub fn claim_count(&self) -> usize {
        self.claims.len()
    }

    /// Driver-side: record the allocation result for a claim. Transitions
    /// Pending → Allocated. Idempotent if already allocated with the same result.
    pub fn allocate_claim(
        &self,
        claim_uid: &str,
        result: AllocationResult,
    ) -> DraResult<()> {
        let mut claim = self
            .claims
            .get_mut(claim_uid)
            .ok_or_else(|| DraError::NotFound(format!("claim {} not found", claim_uid)))?;
        if claim.deallocation_requested {
            return Err(DraError::Forbidden(
                "cannot allocate while deallocation pending".into(),
            ));
        }
        match &claim.allocation {
            Some(existing) if existing == &result => return Ok(()),
            Some(_) => {
                return Err(DraError::AlreadyAllocated(claim_uid.into()));
            }
            None => {}
        }
        claim.allocation = Some(result);
        claim.phase = ClaimPhase::Allocated;
        Ok(())
    }

    /// Reserve the claim for a pod consumer. Allowed only if the claim is
    /// allocated and the consumer matches the shareability rules.
    pub fn reserve_for(
        &self,
        claim_uid: &str,
        consumer: ConsumerReference,
    ) -> DraResult<()> {
        let mut claim = self
            .claims
            .get_mut(claim_uid)
            .ok_or_else(|| DraError::NotFound(format!("claim {} not found", claim_uid)))?;
        let alloc = claim
            .allocation
            .as_ref()
            .ok_or_else(|| DraError::Forbidden("claim not allocated yet".into()))?;
        if claim.deallocation_requested {
            return Err(DraError::Forbidden("deallocation pending".into()));
        }
        // Idempotent.
        if claim.reserved_for.iter().any(|c| c == &consumer) {
            return Ok(());
        }
        if !alloc.shareable && !claim.reserved_for.is_empty() {
            return Err(DraError::Forbidden(
                "non-shareable claim already reserved".into(),
            ));
        }
        claim.reserved_for.push(consumer);
        claim.reserved_for.sort();
        claim.phase = ClaimPhase::InUse;
        Ok(())
    }

    pub fn unreserve(
        &self,
        claim_uid: &str,
        consumer: &ConsumerReference,
    ) -> DraResult<()> {
        let mut claim = self
            .claims
            .get_mut(claim_uid)
            .ok_or_else(|| DraError::NotFound(format!("claim {} not found", claim_uid)))?;
        claim.reserved_for.retain(|c| c != consumer);
        if claim.reserved_for.is_empty() {
            claim.phase = if claim.allocation.is_some() {
                ClaimPhase::Allocated
            } else {
                ClaimPhase::Pending
            };
        }
        Ok(())
    }

    pub fn request_deallocation(&self, claim_uid: &str) -> DraResult<()> {
        let mut claim = self
            .claims
            .get_mut(claim_uid)
            .ok_or_else(|| DraError::NotFound(format!("claim {} not found", claim_uid)))?;
        claim.deallocation_requested = true;
        if claim.reserved_for.is_empty() {
            claim.phase = ClaimPhase::Deallocating;
        }
        Ok(())
    }

    /// Driver-side: confirm deallocation completed; clears the allocation.
    pub fn confirm_deallocation(&self, claim_uid: &str) -> DraResult<()> {
        let mut claim = self
            .claims
            .get_mut(claim_uid)
            .ok_or_else(|| DraError::NotFound(format!("claim {} not found", claim_uid)))?;
        if !claim.reserved_for.is_empty() {
            return Err(DraError::Forbidden(
                "cannot confirm deallocation while consumers exist".into(),
            ));
        }
        claim.allocation = None;
        claim.deallocation_requested = false;
        claim.phase = ClaimPhase::Pending;
        Ok(())
    }

    pub fn delete_claim(&self, claim_uid: &str) -> DraResult<()> {
        if let Some(c) = self.claims.get(claim_uid) {
            if !c.reserved_for.is_empty() {
                return Err(DraError::Forbidden(format!(
                    "claim {} still reserved by {} consumer(s)",
                    claim_uid,
                    c.reserved_for.len()
                )));
            }
        }
        self.claims.remove(claim_uid);
        Ok(())
    }

    // ── ResourceSlice (KEP-4381 structured params) ───────────────────────

    pub fn publish_slice(&self, slice: ResourceSlice) -> DraResult<()> {
        if slice.name.is_empty()
            || slice.node_name.is_empty()
            || slice.driver_name.is_empty()
            || slice.pool_name.is_empty()
        {
            return Err(DraError::Invalid("ResourceSlice requires name+node+driver+pool".into()));
        }
        // Device names within a slice must be unique.
        let mut seen = BTreeSet::new();
        for d in &slice.devices {
            if !seen.insert(&d.name) {
                return Err(DraError::Invalid(format!(
                    "duplicate device {} in slice {}",
                    d.name, slice.name
                )));
            }
        }
        self.slices.insert(slice.name.clone(), slice);
        Ok(())
    }

    pub fn delete_slice(&self, name: &str) -> DraResult<()> {
        self.slices.remove(name);
        Ok(())
    }

    pub fn get_slice(&self, name: &str) -> Option<ResourceSlice> {
        self.slices.get(name).map(|r| r.value().clone())
    }

    pub fn slices_for_node(&self, node_name: &str) -> Vec<ResourceSlice> {
        self.slices
            .iter()
            .filter(|r| r.value().node_name == node_name)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn slice_count(&self) -> usize {
        self.slices.len()
    }

    /// Validate that a pod can be admitted on `node_name` given the claims
    /// it references — each claim must already be allocated and the
    /// allocation must be available on this node.
    pub fn admit_pod_on_node(
        &self,
        node_name: &str,
        node_labels: &BTreeMap<String, String>,
        claim_uids: &[&str],
    ) -> DraResult<()> {
        for uid in claim_uids {
            let claim = self
                .get_claim(uid)
                .ok_or_else(|| DraError::NotFound(format!("claim {}", uid)))?;
            let alloc = claim.allocation.ok_or_else(|| {
                DraError::Forbidden(format!("claim {} not allocated", uid))
            })?;
            if let Some(sel) = &alloc.available_on_nodes {
                if !sel.matches(node_name, node_labels) {
                    return Err(DraError::NotAllocatable(format!(
                        "claim {} not available on node {}",
                        uid, node_name
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, driver: &str) -> ResourceClass {
        ResourceClass {
            name: name.into(),
            driver_name: driver.into(),
            parameters_ref: None,
            suitable_nodes: None,
        }
    }

    fn claim(uid: &str, ns: &str, class_name: &str) -> ResourceClaim {
        ResourceClaim {
            name: format!("c-{}", uid),
            namespace: ns.into(),
            uid: uid.into(),
            spec: ResourceClaimSpec {
                resource_class_name: class_name.into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
            allocation: None,
            phase: ClaimPhase::Pending,
            reserved_for: Vec::new(),
            deallocation_requested: false,
            created_at: Utc::now(),
        }
    }

    fn alloc(driver: &str, shareable: bool, node_filter: Option<&str>) -> AllocationResult {
        AllocationResult {
            driver_name: driver.into(),
            resource_handles: vec![ResourceHandle {
                driver_name: driver.into(),
                data: "{}".into(),
                structured_data: None,
            }],
            available_on_nodes: node_filter.map(|n| NodeSelector {
                node_names: vec![n.into()],
                match_labels: vec![],
            }),
            shareable,
        }
    }

    fn cons(name: &str, uid: &str) -> ConsumerReference {
        ConsumerReference {
            api_group: "".into(),
            resource: "pods".into(),
            name: name.into(),
            uid: uid.into(),
        }
    }

    #[test]
    fn create_class_records() {
        let m = DraManager::new();
        m.create_class(class("gpu-class", "nvidia.com")).unwrap();
        assert!(m.get_class("gpu-class").is_some());
        assert_eq!(m.class_count(), 1);
    }

    #[test]
    fn create_class_rejects_empty() {
        let m = DraManager::new();
        assert!(m.create_class(class("", "drv")).is_err());
        assert!(m.create_class(class("c", "")).is_err());
    }

    #[test]
    fn delete_class_removes() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.delete_class("c").unwrap();
        assert!(m.get_class("c").is_none());
    }

    #[test]
    fn create_claim_requires_known_class() {
        let m = DraManager::new();
        let err = m.create_claim(claim("u1", "ns", "missing-class")).unwrap_err();
        assert!(matches!(err, DraError::NotFound(_)));
    }

    #[test]
    fn create_claim_records_when_class_exists() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        assert_eq!(m.claim_count(), 1);
    }

    #[test]
    fn create_claim_rejects_duplicate_uid() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        let err = m.create_claim(claim("u1", "ns", "c")).unwrap_err();
        assert!(matches!(err, DraError::Conflict(_)));
    }

    #[test]
    fn create_claim_rejects_empty_uid() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        let mut bad = claim("", "ns", "c");
        bad.uid = String::new();
        assert!(m.create_claim(bad).is_err());
    }

    #[test]
    fn allocate_claim_transitions_to_allocated() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        let c = m.get_claim("u1").unwrap();
        assert_eq!(c.phase, ClaimPhase::Allocated);
        assert!(c.allocation.is_some());
    }

    #[test]
    fn allocate_claim_idempotent_for_same_result() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        let a = alloc("drv", false, None);
        m.allocate_claim("u1", a.clone()).unwrap();
        m.allocate_claim("u1", a).unwrap();
    }

    #[test]
    fn allocate_claim_rejects_when_already_with_different_result() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        let err = m.allocate_claim("u1", alloc("drv", true, None)).unwrap_err();
        assert!(matches!(err, DraError::AlreadyAllocated(_)));
    }

    #[test]
    fn allocate_claim_rejects_when_deallocation_pending() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.request_deallocation("u1").unwrap();
        let err = m.allocate_claim("u1", alloc("drv", true, None)).unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn allocate_unknown_claim_errors() {
        let m = DraManager::new();
        let err = m.allocate_claim("ghost", alloc("drv", false, None)).unwrap_err();
        assert!(matches!(err, DraError::NotFound(_)));
    }

    #[test]
    fn reserve_for_requires_allocation() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        let err = m.reserve_for("u1", cons("p1", "p1-uid")).unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn reserve_for_records_consumer_and_transitions_to_in_use() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        let c = m.get_claim("u1").unwrap();
        assert_eq!(c.phase, ClaimPhase::InUse);
        assert_eq!(c.reserved_for.len(), 1);
    }

    #[test]
    fn reserve_for_idempotent_for_same_consumer() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        let consumer = cons("p1", "p1-uid");
        m.reserve_for("u1", consumer.clone()).unwrap();
        m.reserve_for("u1", consumer).unwrap();
        assert_eq!(m.get_claim("u1").unwrap().reserved_for.len(), 1);
    }

    #[test]
    fn reserve_for_non_shareable_rejects_second_consumer() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        let err = m.reserve_for("u1", cons("p2", "p2-uid")).unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn reserve_for_shareable_allows_multiple_consumers() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", true, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        m.reserve_for("u1", cons("p2", "p2-uid")).unwrap();
        m.reserve_for("u1", cons("p3", "p3-uid")).unwrap();
        assert_eq!(m.get_claim("u1").unwrap().reserved_for.len(), 3);
    }

    #[test]
    fn unreserve_drops_consumer() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", true, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        m.unreserve("u1", &cons("p1", "p1-uid")).unwrap();
        assert!(m.get_claim("u1").unwrap().reserved_for.is_empty());
    }

    #[test]
    fn unreserve_last_consumer_returns_phase_to_allocated() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", true, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        m.unreserve("u1", &cons("p1", "p1-uid")).unwrap();
        assert_eq!(m.get_claim("u1").unwrap().phase, ClaimPhase::Allocated);
    }

    #[test]
    fn request_deallocation_marks_flag_only_when_consumers_present() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", true, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        m.request_deallocation("u1").unwrap();
        // Phase stays InUse until consumers go away.
        assert_eq!(m.get_claim("u1").unwrap().phase, ClaimPhase::InUse);
        assert!(m.get_claim("u1").unwrap().deallocation_requested);
    }

    #[test]
    fn request_deallocation_transitions_when_no_consumers() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.request_deallocation("u1").unwrap();
        assert_eq!(m.get_claim("u1").unwrap().phase, ClaimPhase::Deallocating);
    }

    #[test]
    fn confirm_deallocation_clears_allocation() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.request_deallocation("u1").unwrap();
        m.confirm_deallocation("u1").unwrap();
        let c = m.get_claim("u1").unwrap();
        assert!(c.allocation.is_none());
        assert_eq!(c.phase, ClaimPhase::Pending);
        assert!(!c.deallocation_requested);
    }

    #[test]
    fn confirm_deallocation_blocked_by_remaining_consumers() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", true, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        m.request_deallocation("u1").unwrap();
        let err = m.confirm_deallocation("u1").unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn delete_claim_blocked_when_reserved() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, None)).unwrap();
        m.reserve_for("u1", cons("p1", "p1-uid")).unwrap();
        let err = m.delete_claim("u1").unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn delete_claim_succeeds_when_no_consumers() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.delete_claim("u1").unwrap();
        assert_eq!(m.claim_count(), 0);
    }

    #[test]
    fn template_create_records() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        let t = ResourceClaimTemplate {
            name: "t1".into(),
            namespace: "ns".into(),
            spec_template: ResourceClaimSpec {
                resource_class_name: "c".into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
        };
        m.create_template(t).unwrap();
        assert!(m.get_template("ns", "t1").is_some());
    }

    #[test]
    fn template_create_rejects_empty_name() {
        let m = DraManager::new();
        let t = ResourceClaimTemplate {
            name: "".into(),
            namespace: "ns".into(),
            spec_template: ResourceClaimSpec {
                resource_class_name: "c".into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
        };
        assert!(m.create_template(t).is_err());
    }

    #[test]
    fn template_instantiate_creates_per_pod_claim() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        let t = ResourceClaimTemplate {
            name: "t1".into(),
            namespace: "ns".into(),
            spec_template: ResourceClaimSpec {
                resource_class_name: "c".into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
        };
        m.create_template(t).unwrap();
        let uid = m.instantiate_template_for_pod("ns", "t1", "pod-A", "claim-name").unwrap();
        assert!(!uid.is_empty());
        assert_eq!(m.claim_count(), 1);
    }

    #[test]
    fn template_instantiate_idempotent_for_same_pod_claim_name() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        let t = ResourceClaimTemplate {
            name: "t1".into(),
            namespace: "ns".into(),
            spec_template: ResourceClaimSpec {
                resource_class_name: "c".into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
        };
        m.create_template(t).unwrap();
        let a = m.instantiate_template_for_pod("ns", "t1", "pod-A", "claim-name").unwrap();
        let b = m.instantiate_template_for_pod("ns", "t1", "pod-A", "claim-name").unwrap();
        assert_eq!(a, b);
        assert_eq!(m.claim_count(), 1);
    }

    #[test]
    fn template_instantiate_unknown_template_errors() {
        let m = DraManager::new();
        let err = m.instantiate_template_for_pod("ns", "missing", "pod-A", "claim").unwrap_err();
        assert!(matches!(err, DraError::NotFound(_)));
    }

    #[test]
    fn template_instantiate_unknown_class_errors() {
        let m = DraManager::new();
        let t = ResourceClaimTemplate {
            name: "t1".into(),
            namespace: "ns".into(),
            spec_template: ResourceClaimSpec {
                resource_class_name: "ghost-class".into(),
                allocation_mode: AllocationMode::WaitForFirstConsumer,
                parameters_ref: None,
            },
        };
        m.create_template(t).unwrap();
        assert!(m
            .instantiate_template_for_pod("ns", "t1", "pod-A", "claim")
            .is_err());
    }

    #[test]
    fn slice_publish_records_devices() {
        let m = DraManager::new();
        let s = ResourceSlice {
            name: "slice-1".into(),
            node_name: "node-A".into(),
            driver_name: "nvidia.com".into(),
            pool_name: "gpu-pool".into(),
            devices: vec![ResourceDevice {
                name: "dev-0".into(),
                attributes: BTreeMap::new(),
                capacity: BTreeMap::new(),
            }],
        };
        m.publish_slice(s).unwrap();
        assert_eq!(m.slice_count(), 1);
    }

    #[test]
    fn slice_publish_rejects_empty_fields() {
        let m = DraManager::new();
        let mut s = ResourceSlice {
            name: "".into(),
            node_name: "n".into(),
            driver_name: "d".into(),
            pool_name: "p".into(),
            devices: vec![],
        };
        assert!(m.publish_slice(s.clone()).is_err());
        s.name = "ok".into();
        s.node_name = "".into();
        assert!(m.publish_slice(s).is_err());
    }

    #[test]
    fn slice_publish_rejects_duplicate_device_names() {
        let m = DraManager::new();
        let s = ResourceSlice {
            name: "s".into(),
            node_name: "n".into(),
            driver_name: "d".into(),
            pool_name: "p".into(),
            devices: vec![
                ResourceDevice {
                    name: "x".into(),
                    attributes: BTreeMap::new(),
                    capacity: BTreeMap::new(),
                },
                ResourceDevice {
                    name: "x".into(),
                    attributes: BTreeMap::new(),
                    capacity: BTreeMap::new(),
                },
            ],
        };
        assert!(m.publish_slice(s).is_err());
    }

    #[test]
    fn slices_for_node_filters() {
        let m = DraManager::new();
        m.publish_slice(ResourceSlice {
            name: "a".into(),
            node_name: "n1".into(),
            driver_name: "d".into(),
            pool_name: "p".into(),
            devices: vec![],
        })
        .unwrap();
        m.publish_slice(ResourceSlice {
            name: "b".into(),
            node_name: "n2".into(),
            driver_name: "d".into(),
            pool_name: "p".into(),
            devices: vec![],
        })
        .unwrap();
        let v = m.slices_for_node("n1");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "a");
    }

    #[test]
    fn delete_slice_removes() {
        let m = DraManager::new();
        m.publish_slice(ResourceSlice {
            name: "a".into(),
            node_name: "n".into(),
            driver_name: "d".into(),
            pool_name: "p".into(),
            devices: vec![],
        })
        .unwrap();
        m.delete_slice("a").unwrap();
        assert_eq!(m.slice_count(), 0);
    }

    #[test]
    fn admit_pod_requires_allocations() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        let err = m
            .admit_pod_on_node("n1", &BTreeMap::new(), &["u1"])
            .unwrap_err();
        assert!(matches!(err, DraError::Forbidden(_)));
    }

    #[test]
    fn admit_pod_allows_when_node_in_selector() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, Some("n1"))).unwrap();
        m.admit_pod_on_node("n1", &BTreeMap::new(), &["u1"]).unwrap();
    }

    #[test]
    fn admit_pod_denies_when_node_not_in_selector() {
        let m = DraManager::new();
        m.create_class(class("c", "drv")).unwrap();
        m.create_claim(claim("u1", "ns", "c")).unwrap();
        m.allocate_claim("u1", alloc("drv", false, Some("n1"))).unwrap();
        let err = m
            .admit_pod_on_node("n2", &BTreeMap::new(), &["u1"])
            .unwrap_err();
        assert!(matches!(err, DraError::NotAllocatable(_)));
    }

    #[test]
    fn admit_pod_unknown_claim_errors() {
        let m = DraManager::new();
        let err = m.admit_pod_on_node("n", &BTreeMap::new(), &["ghost"]).unwrap_err();
        assert!(matches!(err, DraError::NotFound(_)));
    }

    #[test]
    fn node_selector_matches_label_term() {
        let mut term = BTreeMap::new();
        term.insert("zone".to_string(), "us-west".to_string());
        let sel = NodeSelector {
            node_names: vec![],
            match_labels: vec![term],
        };
        let mut labels = BTreeMap::new();
        labels.insert("zone".into(), "us-west".into());
        assert!(sel.matches("n", &labels));
        labels.insert("zone".into(), "us-east".into());
        assert!(!sel.matches("n", &labels));
    }

    #[test]
    fn node_selector_empty_matches_anything() {
        let sel = NodeSelector::default();
        assert!(sel.matches("any", &BTreeMap::new()));
    }

    #[test]
    fn allocation_mode_default_is_wait_for_first_consumer() {
        assert_eq!(AllocationMode::default(), AllocationMode::WaitForFirstConsumer);
    }

    #[test]
    fn unreserve_unknown_claim_errors() {
        let m = DraManager::new();
        assert!(m.unreserve("ghost", &cons("p", "u")).is_err());
    }

    #[test]
    fn request_deallocation_unknown_claim_errors() {
        let m = DraManager::new();
        assert!(m.request_deallocation("ghost").is_err());
    }

    #[test]
    fn confirm_deallocation_unknown_claim_errors() {
        let m = DraManager::new();
        assert!(m.confirm_deallocation("ghost").is_err());
    }

    #[test]
    fn delete_unknown_claim_is_noop() {
        let m = DraManager::new();
        m.delete_claim("ghost").unwrap();
    }
}
