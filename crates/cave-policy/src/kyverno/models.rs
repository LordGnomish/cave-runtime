// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno CRD data models.
//!
//! Covers: ClusterPolicy, Policy, all rule types (validate/mutate/generate/verifyImages),
//! PolicyReport, ClusterPolicyReport, CleanupPolicy, PolicyException.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Core Policy Types ────────────────────────────────────────────────────────

/// ClusterPolicy — cluster-scoped policy (no namespace restriction).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterPolicy {
    #[serde(rename = "apiVersion", default = "default_api_version")]
    pub api_version: String,
    #[serde(rename = "kind", default = "default_cluster_policy_kind")]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PolicySpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PolicyStatus>,
}

/// Policy — namespace-scoped policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    #[serde(rename = "apiVersion", default = "default_api_version")]
    pub api_version: String,
    #[serde(rename = "kind", default = "default_policy_kind")]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PolicySpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PolicyStatus>,
}

fn default_api_version() -> String {
    "kyverno.io/v1".into()
}
fn default_cluster_policy_kind() -> String {
    "ClusterPolicy".into()
}
fn default_policy_kind() -> String {
    "Policy".into()
}

/// Common Kubernetes object metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub annotations: HashMap<String, String>,
    #[serde(rename = "resourceVersion", skip_serializing_if = "Option::is_none")]
    pub resource_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(rename = "creationTimestamp", skip_serializing_if = "Option::is_none")]
    pub creation_timestamp: Option<DateTime<Utc>>,
}

/// Policy spec — contains validation failure action and rules.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicySpec {
    pub rules: Vec<KyvernoRule>,
    #[serde(rename = "validationFailureAction", default)]
    pub validation_failure_action: ValidationFailureAction,
    #[serde(
        rename = "validationFailureActionOverrides",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub validation_failure_action_overrides: Vec<ValidationFailureActionOverride>,
    #[serde(default = "default_true")]
    pub background: bool,
    #[serde(rename = "schemaValidation", default = "default_true")]
    pub schema_validation: bool,
    #[serde(rename = "generateExistingOnPolicyUpdate", default)]
    pub generate_existing_on_policy_update: bool,
    #[serde(rename = "failurePolicy", default)]
    pub failure_policy: FailurePolicy,
    #[serde(
        rename = "webhookTimeoutSeconds",
        skip_serializing_if = "Option::is_none"
    )]
    pub webhook_timeout_seconds: Option<u32>,
    #[serde(rename = "mutateExistingOnPolicyUpdate", default)]
    pub mutate_existing_on_policy_update: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ValidationFailureAction {
    #[default]
    Audit,
    Enforce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationFailureActionOverride {
    pub action: ValidationFailureAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum FailurePolicy {
    #[default]
    Fail,
    Ignore,
}

/// A single Kyverno rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KyvernoRule {
    pub name: String,
    #[serde(rename = "match")]
    pub match_resources: MatchResources,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<ExcludeResources>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ContextEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preconditions: Option<Conditions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<Validation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutate: Option<Mutation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate: Option<Generation>,
    #[serde(
        rename = "verifyImages",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub verify_images: Vec<ImageVerification>,
}

impl KyvernoRule {
    pub fn rule_type(&self) -> &'static str {
        if self.validate.is_some() {
            "validate"
        } else if self.mutate.is_some() {
            "mutate"
        } else if self.generate.is_some() {
            "generate"
        } else if !self.verify_images.is_empty() {
            "verifyImages"
        } else {
            "unknown"
        }
    }
}

// ─── Match / Exclude ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MatchResources {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<ResourceFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<ResourceFilter>,
    /// Shorthand: resources directly in match (no any/all)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceDescription>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<SubjectReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(
        rename = "clusterRoles",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub cluster_roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludeResources {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<ResourceFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<ResourceFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceDescription>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<SubjectReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceDescription>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<SubjectReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(
        rename = "clusterRoles",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub cluster_roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceDescription {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    #[serde(rename = "namespaceSelector", skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<LabelSelector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<LabelSelector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<String>, // CREATE, UPDATE, DELETE, CONNECT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSelector {
    #[serde(
        rename = "matchLabels",
        default,
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub match_labels: HashMap<String, String>,
    #[serde(
        rename = "matchExpressions",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: String, // In, NotIn, Exists, DoesNotExist
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectReference {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub name: String,
}

// ─── Context & Preconditions ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub name: String,
    #[serde(rename = "configMap", skip_serializing_if = "Option::is_none")]
    pub config_map: Option<ConfigMapReference>,
    #[serde(rename = "apiCall", skip_serializing_if = "Option::is_none")]
    pub api_call: Option<ApiCallDescriptor>,
    #[serde(rename = "imageRegistry", skip_serializing_if = "Option::is_none")]
    pub image_registry: Option<ImageRegistryDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapReference {
    pub name: String,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCallDescriptor {
    #[serde(rename = "urlPath")]
    pub url_path: String,
    #[serde(rename = "jmesPath", skip_serializing_if = "Option::is_none")]
    pub jmes_path: Option<String>,
    #[serde(rename = "requestType", default)]
    pub request_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRegistryDescriptor {
    pub reference: String,
    #[serde(rename = "jmesPath", skip_serializing_if = "Option::is_none")]
    pub jmes_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conditions {
    pub any: Option<Vec<Condition>>,
    pub all: Option<Vec<Condition>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub key: serde_json::Value,
    pub operator: ConditionOperator,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    In,
    NotIn,
    GreaterThan,
    GreaterThanOrEquals,
    LessThan,
    LessThanOrEquals,
    DurationGreaterThan,
    DurationLessThan,
    DurationGreaterThanOrEquals,
    DurationLessThanOrEquals,
    Contains,
    NotContains,
    AnyIn,
    AllIn,
    AnyNotIn,
    AllNotIn,
    #[serde(other)]
    Unknown,
}

// ─── Validate ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Validation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "anyPattern", skip_serializing_if = "Option::is_none")]
    pub any_pattern: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<DenyConditions>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreach: Vec<ForEachValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cel: Option<CelValidation>,
    #[serde(rename = "podSecurity", skip_serializing_if = "Option::is_none")]
    pub pod_security: Option<PodSecurityValidation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenyConditions {
    pub conditions: Conditions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachValidation {
    pub list: String, // JMESPath expression
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<serde_json::Value>,
    #[serde(rename = "anyPattern", skip_serializing_if = "Option::is_none")]
    pub any_pattern: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<DenyConditions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContextEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preconditions: Option<Conditions>,
    #[serde(rename = "elementScope", default = "default_true")]
    pub element_scope: bool,
    #[serde(rename = "order", skip_serializing_if = "Option::is_none")]
    pub order: Option<String>, // Ascending | Descending
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelValidation {
    pub expressions: Vec<CelExpression>,
    #[serde(
        rename = "auditAnnotations",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub audit_annotations: Vec<CelAuditAnnotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelExpression {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "messageExpression", skip_serializing_if = "Option::is_none")]
    pub message_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelAuditAnnotation {
    pub key: String,
    #[serde(rename = "valueExpression")]
    pub value_expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSecurityValidation {
    pub level: String, // baseline | restricted | privileged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<PodSecurityExclusion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSecurityExclusion {
    #[serde(rename = "controlName")]
    pub control_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

// ─── Mutate ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutation {
    #[serde(
        rename = "patchStrategicMerge",
        skip_serializing_if = "Option::is_none"
    )]
    pub patch_strategic_merge: Option<serde_json::Value>,
    #[serde(rename = "patchesJson6902", skip_serializing_if = "Option::is_none")]
    pub patches_json6902: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreach: Vec<ForEachMutation>,
    #[serde(rename = "targets", default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetResourceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachMutation {
    pub list: String,
    #[serde(
        rename = "patchStrategicMerge",
        skip_serializing_if = "Option::is_none"
    )]
    pub patch_strategic_merge: Option<serde_json::Value>,
    #[serde(rename = "patchesJson6902", skip_serializing_if = "Option::is_none")]
    pub patches_json6902: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContextEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preconditions: Option<Conditions>,
    #[serde(rename = "order", skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetResourceSpec {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

// ─── Generate ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generation {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default)]
    pub synchronize: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone: Option<CloneSpec>,
    #[serde(rename = "cloneList", skip_serializing_if = "Option::is_none")]
    pub clone_list: Option<CloneListSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneSpec {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneListSpec {
    pub namespace: String,
    pub kinds: Vec<CloneKindSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<LabelSelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneKindSpec {
    pub group: String,
    pub kind: String,
    pub version: String,
}

// ─── Image Verification ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageVerification {
    #[serde(rename = "imageReferences", default)]
    pub image_references: Vec<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub verification_type: Option<String>, // Cosign | Notary
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attestors: Vec<AttestorSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attestations: Vec<AttestationSpec>,
    #[serde(rename = "mutateDigest", default = "default_true")]
    pub mutate_digest: bool,
    #[serde(rename = "verifyDigest", default = "default_true")]
    pub verify_digest: bool,
    #[serde(rename = "required", default = "default_true")]
    pub required: bool,
    #[serde(
        rename = "imageRegistryCredentials",
        skip_serializing_if = "Option::is_none"
    )]
    pub image_registry_credentials: Option<ImageRegistryCredentials>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestorSet {
    pub count: Option<u32>,
    pub entries: Vec<Attestor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<StaticKeyAttestor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificates: Option<CertificateAttestor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyless: Option<KeylessAttestor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticKeyAttestor {
    #[serde(rename = "publicKeys")]
    pub public_keys: String,
    #[serde(rename = "signatureAlgorithm", skip_serializing_if = "Option::is_none")]
    pub signature_algorithm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kms: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateAttestor {
    #[serde(rename = "cert", skip_serializing_if = "Option::is_none")]
    pub cert: Option<String>,
    #[serde(rename = "certChain", skip_serializing_if = "Option::is_none")]
    pub cert_chain: Option<String>,
    #[serde(rename = "rekor", skip_serializing_if = "Option::is_none")]
    pub rekor: Option<RekorConfig>,
    #[serde(rename = "ctlog", skip_serializing_if = "Option::is_none")]
    pub ctlog: Option<CtlogConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeylessAttestor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rekor: Option<RekorConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctlog: Option<CtlogConfig>,
    #[serde(rename = "subjectRegExp", skip_serializing_if = "Option::is_none")]
    pub subject_regexp: Option<String>,
    #[serde(rename = "issuerRegExp", skip_serializing_if = "Option::is_none")]
    pub issuer_regexp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RekorConfig {
    pub url: String,
    #[serde(rename = "ignoreTlog", default)]
    pub ignore_tlog: bool,
    #[serde(rename = "pubkey", skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtlogConfig {
    #[serde(rename = "ignoreSCT", default)]
    pub ignore_sct: bool,
    #[serde(rename = "pubkey", skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretReference {
    pub name: String,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationSpec {
    #[serde(rename = "type")]
    pub attestation_type: String,
    pub conditions: Vec<Conditions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestors: Option<Vec<AttestorSet>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRegistryCredentials {
    #[serde(default)]
    pub allowInsecureRegistry: bool,
    pub providers: Vec<ImageRegistryProvider>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageRegistryProvider {
    Secret,
    ServiceAccount,
    Google,
    Azure,
    Amazon,
    GitHub,
}

// ─── Policy Status ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyStatus {
    #[serde(rename = "ready")]
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<StatusCondition>,
    #[serde(rename = "rulecount", skip_serializing_if = "Option::is_none")]
    pub rule_count: Option<RuleCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCondition {
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "lastTransitionTime", skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleCount {
    pub validate: u32,
    pub mutate: u32,
    pub generate: u32,
    #[serde(rename = "verifyimages")]
    pub verify_images: u32,
}

// ─── Policy Reports ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyReport {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub results: Vec<PolicyReportResult>,
    pub summary: PolicyReportSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterPolicyReport {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub results: Vec<PolicyReportResult>,
    pub summary: PolicyReportSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyReportResult {
    pub policy: String,
    pub rule: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub result: PolicyReportStatus,
    pub scored: bool,
    pub severity: Option<String>,
    pub source: String,
    pub timestamp: DateTime<Utc>,
    pub resources: Vec<ResourceReference>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyReportStatus {
    Pass,
    Fail,
    Warn,
    Error,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceReference {
    #[serde(rename = "apiVersion", skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyReportSummary {
    pub pass: u32,
    pub fail: u32,
    pub warn: u32,
    pub error: u32,
    pub skip: u32,
}

// ─── Cleanup Policy ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPolicy {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String, // CleanupPolicy | ClusterCleanupPolicy
    pub metadata: ObjectMeta,
    pub spec: CleanupPolicySpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CleanupPolicyStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPolicySpec {
    pub schedule: String, // cron expression
    #[serde(rename = "match")]
    pub match_resources: MatchResources,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<ExcludeResources>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Conditions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPolicyStatus {
    #[serde(rename = "lastExecutionTime", skip_serializing_if = "Option::is_none")]
    pub last_execution_time: Option<DateTime<Utc>>,
}

// ─── Policy Exception ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyException {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PolicyExceptionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExceptionSpec {
    pub exceptions: Vec<PolicyExceptionEntry>,
    #[serde(rename = "match")]
    pub match_resources: MatchResources,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Conditions>,
    #[serde(rename = "podSecurity", default, skip_serializing_if = "Vec::is_empty")]
    pub pod_security: Vec<PodSecurityExclusion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExceptionEntry {
    #[serde(rename = "policyName")]
    pub policy_name: String,
    #[serde(rename = "ruleNames")]
    pub rule_names: Vec<String>,
}

// ─── Evaluation result ────────────────────────────────────────────────────────

/// Result of applying Kyverno policies to a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEvalResult {
    pub allowed: bool,
    pub mutations: Vec<serde_json::Value>, // JSON Patch operations
    pub violations: Vec<PolicyViolation>,
    pub warnings: Vec<String>,
    pub generated: Vec<GeneratedResource>,
    pub image_verification_results: Vec<ImageVerificationResult>,
}

impl PolicyEvalResult {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            mutations: vec![],
            violations: vec![],
            warnings: vec![],
            generated: vec![],
            image_verification_results: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub policy: String,
    pub rule: String,
    pub message: String,
    pub severity: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedResource {
    pub policy: String,
    pub rule: String,
    pub resource: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageVerificationResult {
    pub image: String,
    pub verified: bool,
    pub digest: Option<String>,
    pub error: Option<String>,
}
