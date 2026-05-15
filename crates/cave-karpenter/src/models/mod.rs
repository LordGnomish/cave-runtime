// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Karpenter v1 CRD models.
//!
//! Upstream: kubernetes-sigs/karpenter v1.12.0
//!   pkg/apis/v1/nodepool.go      → NodePool
//!   pkg/apis/v1/nodeclaim.go     → NodeClaim
//!   pkg/apis/v1/nodeclass.go     → NodeClass (provider-specific shape kept opaque)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodePool {
    pub name: String,
    pub namespace: Option<String>,
    pub template: NodeClaimTemplate,
    pub disruption: Option<Disruption>,
    pub limits: Option<Limits>,
    pub weight: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClaimTemplate {
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub spec: NodeClaimSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClaim {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: NodeClaimSpec,
    pub status: Option<NodeClaimStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClaimSpec {
    pub node_class_ref: Option<NodeClassRef>,
    pub requirements: Vec<Requirement>,
    pub taints: Vec<Taint>,
    pub startup_taints: Vec<Taint>,
    pub expire_after: Option<String>,
    pub termination_grace_period: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClaimStatus {
    pub provider_id: Option<String>,
    pub node_name: Option<String>,
    pub allocatable: BTreeMap<String, String>,
    pub capacity: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClassRef {
    pub group: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub key: String,
    pub operator: RequirementOperator,
    pub values: Vec<String>,
    #[serde(default)]
    pub min_values: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequirementOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
    Gt,
    Lt,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Disruption {
    pub consolidation_policy: Option<String>,
    pub consolidate_after: Option<String>,
    pub budgets: Vec<Budget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Budget {
    pub nodes: String,
    pub schedule: Option<String>,
    pub duration: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Limits {
    pub resources: BTreeMap<String, String>,
}

/// Provider-agnostic NodeClass envelope. Concrete shape (e.g. EC2NodeClass for AWS,
/// HetznerNodeClass for the Hetzner provider) is kept as raw JSON until cave-karpenter
/// gains provider modules.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeClass {
    pub group: String,
    pub kind: String,
    pub name: String,
    pub spec: serde_json::Value,
}
