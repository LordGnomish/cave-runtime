// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster-pool subnet refill controller — per-node watermark
//! pre-allocation.
//!
//! Mirrors `pkg/ipam/clusterpool/clusterpool.go::Refill` and the
//! per-node policy in `pkg/ipam/types.go::Refill`. When a node's
//! free-IP count drops below the configured low-watermark, the
//! cilium-operator carves an additional `/24` (or configured prefix
//! length) subnet from the cluster pool and adds it to the node's
//! pod_cidrs list. Conversely, if free count exceeds the
//! high-watermark, an empty subnet may be released back to the pool.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeWatermarkSpec {
    pub node: String,
    pub pre_allocate: u32,
    pub max_above_watermark: u32,
    pub allocated: Vec<String>, // pod CIDRs assigned to the node
    pub used_ips: u32,
    pub capacity_per_subnet: u32,
}

impl NodeWatermarkSpec {
    pub fn free(&self) -> u32 {
        let total = self.allocated.len() as u32 * self.capacity_per_subnet;
        total.saturating_sub(self.used_ips)
    }
    pub fn needs_refill(&self) -> bool {
        self.free() < self.pre_allocate
    }
    pub fn can_release_subnet(&self) -> bool {
        self.free() > self.max_above_watermark + self.capacity_per_subnet
            && self.allocated.len() > 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefillAction {
    pub node: String,
    pub kind: RefillKind,
    pub subnet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefillKind {
    Add,
    Release,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RefillError {
    #[error("node `{0}` not registered")]
    NodeNotFound(String),
    #[error("cluster pool exhausted (allocated {0})")]
    PoolExhausted(usize),
    #[error("tenant {tenant} cannot mutate refill controller owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct RefillController {
    pub tenant: TenantId,
    nodes: BTreeMap<String, NodeWatermarkSpec>,
    /// Available subnets in the cluster pool (FIFO).
    pool_subnets: std::collections::VecDeque<String>,
}

impl RefillController {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            nodes: BTreeMap::new(),
            pool_subnets: std::collections::VecDeque::new(),
        }
    }

    pub fn seed_pool(&mut self, subnets: Vec<String>) {
        for s in subnets {
            self.pool_subnets.push_back(s);
        }
    }

    pub fn pool_remaining(&self) -> usize {
        self.pool_subnets.len()
    }

    pub fn upsert_node(&mut self, spec: NodeWatermarkSpec) {
        self.nodes.insert(spec.node.clone(), spec);
    }

    pub fn remove_node(&mut self, name: &str) -> Result<(), RefillError> {
        let removed = self.nodes.remove(name).ok_or_else(|| RefillError::NodeNotFound(name.to_string()))?;
        for s in removed.allocated {
            self.pool_subnets.push_back(s);
        }
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&NodeWatermarkSpec> {
        self.nodes.get(name)
    }

    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Compute the next refill action for `node`. Returns `None` if no
    /// action is needed.
    pub fn plan(&self, node: &str) -> Result<Option<RefillAction>, RefillError> {
        let spec = self.nodes.get(node).ok_or_else(|| RefillError::NodeNotFound(node.to_string()))?;
        if spec.needs_refill() {
            return Ok(self.pool_subnets.front().map(|s| RefillAction {
                node: node.to_string(),
                kind: RefillKind::Add,
                subnet: s.clone(),
            }));
        }
        if spec.can_release_subnet() {
            // Release the last allocated subnet (the one most recently added).
            if let Some(s) = spec.allocated.last() {
                return Ok(Some(RefillAction {
                    node: node.to_string(),
                    kind: RefillKind::Release,
                    subnet: s.clone(),
                }));
            }
        }
        Ok(None)
    }

    /// Apply a planned action: add removes from the pool and appends to
    /// the node; release removes from the node and pushes back to the pool.
    pub fn apply(&mut self, action: RefillAction) -> Result<(), RefillError> {
        let spec = self.nodes.get_mut(&action.node).ok_or_else(|| RefillError::NodeNotFound(action.node.clone()))?;
        match action.kind {
            RefillKind::Add => {
                let popped = self.pool_subnets.pop_front().ok_or(RefillError::PoolExhausted(spec.allocated.len()))?;
                spec.allocated.push(popped);
            }
            RefillKind::Release => {
                spec.allocated.retain(|s| *s != action.subnet);
                self.pool_subnets.push_back(action.subnet);
            }
        }
        Ok(())
    }

    /// Run a full reconcile pass: for each node, plan + apply zero or
    /// one action. Returns the list of actions taken.
    pub fn reconcile(&mut self) -> Vec<RefillAction> {
        let mut actions = Vec::new();
        let names: Vec<String> = self.nodes.keys().cloned().collect();
        for name in names {
            if let Ok(Some(action)) = self.plan(&name) {
                if self.apply(action.clone()).is_ok() {
                    actions.push(action);
                }
            }
        }
        actions
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipam/clusterpool/clusterpool.go", "Refill");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ctrl(tenant: TenantId) -> RefillController {
        RefillController::new(tenant)
    }

    fn node_spec(name: &str, allocated: &[&str], used: u32) -> NodeWatermarkSpec {
        NodeWatermarkSpec {
            node: name.into(),
            pre_allocate: 8,
            max_above_watermark: 16,
            allocated: allocated.iter().map(|s| (*s).to_string()).collect(),
            used_ips: used,
            capacity_per_subnet: 256,
        }
    }

    // ── Free / watermark math ──────────────────────────────────────────────

    #[test]
    fn free_subtracts_used_from_total() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "Refill.Free", "tenant-cpr-f");
        let s = node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 100);
        assert_eq!(s.free(), 256 * 2 - 100);
    }

    #[test]
    fn needs_refill_true_when_free_below_pre_allocate() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "Refill.Needs", "tenant-cpr-n");
        let mut s = node_spec("a", &["10.244.0.0/24"], 250);
        s.pre_allocate = 8;
        assert!(s.needs_refill());
    }

    #[test]
    fn needs_refill_false_when_free_above_pre_allocate() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "Refill.NoNeed", "tenant-cpr-nn");
        let s = node_spec("a", &["10.244.0.0/24"], 100);
        assert!(!s.needs_refill());
    }

    #[test]
    fn can_release_subnet_when_excess_above_high_watermark() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "Refill.CanRelease", "tenant-cpr-cr");
        let s = node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 5);
        // free = 512 - 5 = 507; max_above_watermark = 16 + capacity = 16 + 256 = 272 → 507 > 272 → true.
        assert!(s.can_release_subnet());
    }

    #[test]
    fn cannot_release_when_only_one_subnet() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "Refill.CanReleaseSingle", "tenant-cpr-crs");
        let s = node_spec("a", &["10.244.0.0/24"], 0);
        assert!(!s.can_release_subnet());
    }

    // ── Pool seeding / accounting ──────────────────────────────────────────

    #[test]
    fn seed_pool_records_subnets() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "SeedPool", "tenant-cpr-sp");
        let mut c = ctrl(tenant);
        c.seed_pool(vec!["10.244.0.0/24".into(), "10.244.1.0/24".into()]);
        assert_eq!(c.pool_remaining(), 2);
    }

    // ── Plan / apply ───────────────────────────────────────────────────────

    #[test]
    fn plan_returns_add_when_node_needs_refill() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Plan.Add", "tenant-cpr-pa");
        let mut c = ctrl(tenant);
        c.seed_pool(vec!["10.244.0.0/24".into(), "10.244.1.0/24".into()]);
        c.upsert_node(node_spec("a", &[], 0));
        let action = c.plan("a").unwrap().unwrap();
        assert_eq!(action.kind, RefillKind::Add);
    }

    #[test]
    fn plan_returns_none_when_within_watermarks() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Plan.None", "tenant-cpr-pn");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &["10.244.0.0/24"], 100));
        let action = c.plan("a").unwrap();
        assert!(action.is_none());
    }

    #[test]
    fn plan_unknown_node_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Plan.NotFound", "tenant-cpr-pnf");
        let c = ctrl(tenant);
        let err = c.plan("ghost").unwrap_err();
        assert!(matches!(err, RefillError::NodeNotFound(_)));
    }

    #[test]
    fn plan_returns_none_when_pool_empty() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Plan.PoolEmpty", "tenant-cpr-ppe");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &[], 0));
        let action = c.plan("a").unwrap();
        // Pool empty + needs refill → no front to add, returns None.
        assert!(action.is_none());
    }

    #[test]
    fn plan_returns_release_when_excess() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Plan.Release", "tenant-cpr-pr");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 5));
        let action = c.plan("a").unwrap().unwrap();
        assert_eq!(action.kind, RefillKind::Release);
    }

    #[test]
    fn apply_add_pops_from_pool_and_appends_to_node() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Apply.Add", "tenant-cpr-aa");
        let mut c = ctrl(tenant);
        c.seed_pool(vec!["10.244.0.0/24".into()]);
        c.upsert_node(node_spec("a", &[], 0));
        let action = c.plan("a").unwrap().unwrap();
        c.apply(action).unwrap();
        assert_eq!(c.pool_remaining(), 0);
        assert_eq!(c.lookup("a").unwrap().allocated.len(), 1);
    }

    #[test]
    fn apply_release_returns_subnet_to_pool() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Apply.Release", "tenant-cpr-ar");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 5));
        let action = c.plan("a").unwrap().unwrap();
        c.apply(action).unwrap();
        assert_eq!(c.pool_remaining(), 1);
        assert_eq!(c.lookup("a").unwrap().allocated.len(), 1);
    }

    #[test]
    fn apply_add_with_empty_pool_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Apply.PoolExhausted", "tenant-cpr-ape");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &[], 0));
        let err = c.apply(RefillAction {
            node: "a".into(), kind: RefillKind::Add, subnet: "".into(),
        }).unwrap_err();
        assert!(matches!(err, RefillError::PoolExhausted(_)));
    }

    #[test]
    fn apply_unknown_node_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Apply.UnknownNode", "tenant-cpr-aun");
        let mut c = ctrl(tenant);
        let err = c.apply(RefillAction {
            node: "ghost".into(), kind: RefillKind::Add, subnet: "".into(),
        }).unwrap_err();
        assert!(matches!(err, RefillError::NodeNotFound(_)));
    }

    // ── Reconcile ──────────────────────────────────────────────────────────

    #[test]
    fn reconcile_refills_all_low_nodes() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Reconcile.RefillAll", "tenant-cpr-rra");
        let mut c = ctrl(tenant);
        c.seed_pool(vec![
            "10.244.0.0/24".into(),
            "10.244.1.0/24".into(),
            "10.244.2.0/24".into(),
        ]);
        c.upsert_node(node_spec("a", &[], 0));
        c.upsert_node(node_spec("b", &[], 0));
        let actions = c.reconcile();
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().all(|a| matches!(a.kind, RefillKind::Add)));
    }

    #[test]
    fn reconcile_no_action_when_all_satisfied() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Reconcile.NoAction", "tenant-cpr-rna");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &["10.244.0.0/24"], 100));
        let actions = c.reconcile();
        assert!(actions.is_empty());
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    #[test]
    fn remove_node_returns_subnets_to_pool() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "RemoveNode", "tenant-cpr-rm");
        let mut c = ctrl(tenant);
        c.upsert_node(node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 0));
        c.remove_node("a").unwrap();
        assert_eq!(c.pool_remaining(), 2);
    }

    #[test]
    fn remove_unknown_node_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "RemoveNode.NotFound", "tenant-cpr-rmnf");
        let mut c = ctrl(tenant);
        let err = c.remove_node("ghost").unwrap_err();
        assert!(matches!(err, RefillError::NodeNotFound(_)));
    }

    #[test]
    fn count_tracks_node_registrations() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Count", "tenant-cpr-c");
        let mut c = ctrl(tenant);
        for i in 0..3u8 {
            c.upsert_node(node_spec(&format!("n-{i}"), &[], 0));
        }
        assert_eq!(c.count(), 3);
    }

    // ── Combined ───────────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_add_then_release() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Lifecycle", "tenant-cpr-lc");
        let mut c = ctrl(tenant);
        // Pre-seed the node with 2 subnets and tiny used count → release.
        c.upsert_node(node_spec("a", &["10.244.0.0/24", "10.244.1.0/24"], 0));
        let actions = c.reconcile();
        assert!(actions.iter().any(|a| matches!(a.kind, RefillKind::Release)));
        assert_eq!(c.lookup("a").unwrap().allocated.len(), 1);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn refill_action_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/clusterpool/clusterpool.go", "Action.Serde", "tenant-cpr-aserde");
        let a = RefillAction { node: "a".into(), kind: RefillKind::Add, subnet: "10.0.0.0/24".into() };
        let s = serde_json::to_string(&a).unwrap();
        let back: RefillAction = serde_json::from_str(&s).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn node_watermark_spec_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/types.go", "NodeWatermark.Serde", "tenant-cpr-nserde");
        let s = node_spec("a", &["10.244.0.0/24"], 100);
        let json = serde_json::to_string(&s).unwrap();
        let back: NodeWatermarkSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
