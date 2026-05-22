// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Data models for cave-admission.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Kubernetes-style resource metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMeta {
    pub name: String,
    pub namespace: Option<String>,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

/// A Kubernetes-style resource being admitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub api_version: String,
    pub kind: String,
    pub metadata: ResourceMeta,
    pub spec: serde_json::Value,
}

/// Admission operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Operation {
    Create,
    Update,
    Delete,
}

/// Criteria for matching a policy to a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMatch {
    /// Resource kinds to match, e.g. `["Pod"]`. Empty = match all.
    pub kinds: Vec<String>,
    /// Namespaces to match. Empty = match all.
    pub namespaces: Vec<String>,
    /// Operations to match. Empty = match all.
    pub operations: Vec<Operation>,
    /// Required label key/value pairs on the resource.
    pub label_selector: Option<HashMap<String, String>>,
}

/// Validation rules checked against a resource spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyRule {
    RequiredLabel {
        key: String,
        /// If non-empty, the label value must be one of these.
        allowed_values: Vec<String>,
    },
    RequiredAnnotation {
        key: String,
    },
    DisallowPrivileged,
    RequireResourceLimits,
    AllowedRegistries {
        registries: Vec<String>,
    },
    MaxReplicas {
        max: u32,
    },
    RequiredNamespace {
        namespaces: Vec<String>,
    },
}

/// A JSON-Patch-style mutation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationPatch {
    pub op: PatchOp,
    pub path: String,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    Add,
    Remove,
    Replace,
}

/// Rule for generating a companion resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRule {
    pub kind: String,
    pub api_version: String,
    /// Template for the generated resource name; `{{name}}` is replaced with the triggering resource's name.
    pub name_template: String,
    pub spec: serde_json::Value,
}

/// Rule for verifying container image signatures (cosign/notation compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyImagesRule {
    pub allowed_registries: Vec<String>,
    pub require_signature: bool,
    pub key_ref: Option<String>,
}

/// The policy-type-specific rule set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "policy_type", rename_all = "snake_case")]
pub enum PolicySpec {
    Validate { rules: Vec<PolicyRule> },
    Mutate { patches: Vec<MutationPatch> },
    Generate { generate: GenerateRule },
    VerifyImages { rule: VerifyImagesRule },
}

/// A policy document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub match_criteria: PolicyMatch,
    pub spec: PolicySpec,
    /// In audit mode, violations are recorded but requests are not blocked.
    pub audit_mode: bool,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single policy violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub policy_name: String,
    pub resource_kind: String,
    pub resource_name: String,
    pub resource_namespace: Option<String>,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

/// Result of evaluating one or more policies against a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionResult {
    pub allowed: bool,
    pub violations: Vec<Violation>,
    pub mutations: Vec<MutationPatch>,
    pub generated_resources: Vec<serde_json::Value>,
}

/// Compliance report summarising policy results across resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyReport {
    pub id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub total_resources_checked: usize,
    pub total_violations: usize,
    pub violations_by_policy: HashMap<String, usize>,
    pub passing_policies: Vec<String>,
    pub failing_policies: Vec<String>,
}
