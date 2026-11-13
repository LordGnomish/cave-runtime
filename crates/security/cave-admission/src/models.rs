// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Data models for cave-admission.
//!
//! Two model layers:
//! 1. High-level Policy layer  — Policy / PolicySpec / PolicyRule / AdmissionResult
//! 2. K8s Webhook layer        — AdmissionRequest / AdmissionResponse / AdmissionPolicy / ViolationLog

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

// ── K8s Webhook layer ─────────────────────────────────────────────────────────
//
// Models for the low-level Kubernetes AdmissionReview protocol (used by
// evaluator.rs and store.rs).  These mirror the structures sent/received by
// the kube-apiserver to validating/mutating admission webhooks.

/// Kubernetes admission operation (CREATE / UPDATE / DELETE / CONNECT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AdmissionOperation {
    Create,
    Update,
    Delete,
    Connect,
}

/// Identifies the API group/version/resource of the incoming object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionResource {
    pub group: String,
    pub version: String,
    pub resource: String,
}

/// Kubernetes user info included in every AdmissionRequest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub uid: String,
    pub groups: Vec<String>,
}

/// The full K8s-style `AdmissionRequest` as sent by the apiserver to a webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRequest {
    /// Random request UID generated by kube-apiserver; echoed in the response.
    pub uid: Uuid,
    pub operation: AdmissionOperation,
    pub resource: GroupVersionResource,
    /// The incoming object (JSON).
    pub object: serde_json::Value,
    /// The existing object (UPDATE/DELETE only).
    pub old_object: Option<serde_json::Value>,
    pub user_info: UserInfo,
    /// If true this is a dry-run; mutations are still returned but not applied.
    pub dry_run: bool,
}

/// HTTP status block included when a request is denied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionStatus {
    pub code: u16,
    pub message: String,
}

/// The K8s-style `AdmissionResponse` returned by the webhook handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionResponse {
    pub uid: Uuid,
    pub allowed: bool,
    /// Present when `allowed == false`.
    pub status: Option<AdmissionStatus>,
    /// Base64-encoded RFC 6902 JSON Patch (mutations only).
    pub patch: Option<String>,
    /// If `patch` is set, this must be `"JSONPatch"`.
    pub patch_type: Option<String>,
    /// Non-blocking advisory messages forwarded to the user.
    pub warnings: Vec<String>,
}

/// Enforcement action for a webhook-layer policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnforcementAction {
    /// Reject the request with HTTP 403.
    Deny,
    /// Allow the request but attach a warning.
    Warn,
    /// Allow the request; record violation for audit only.
    Audit,
}

/// A single rule evaluated against the raw JSON object.
/// The field is specified in dot-notation (`spec.securityContext.privileged`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEvalRule {
    pub field: String,
    pub operator: RuleOperator,
    pub value: serde_json::Value,
}

/// Operators available for `PolicyEvalRule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOperator {
    Exists,
    NotExists,
    Equals,
    NotEquals,
    Contains,
    NotContains,
    GreaterThan,
    LessThan,
}

/// A webhook-layer admission policy stored in the `AdmissionStore`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionPolicy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Operations this policy applies to.  Empty = all.
    pub operation_types: Vec<AdmissionOperation>,
    /// Resource types (e.g. `"pods"`, `"deployments"`).  Empty = all.
    pub resource_types: Vec<String>,
    pub enforcement_action: EnforcementAction,
    pub rules: Vec<PolicyEvalRule>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// An immutable violation record written for every policy match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationLog {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub request_uid: Uuid,
    pub resource_name: String,
    pub resource_namespace: String,
    pub operation: AdmissionOperation,
    pub message: String,
    pub enforcement_action: EnforcementAction,
    pub logged_at: DateTime<Utc>,
}
