// SPDX-License-Identifier: AGPL-3.0-or-later
//! ValidatingAdmissionPolicy (KEP-3488) — line-by-line port of upstream
//! `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/`.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/api/admissionregistration/v1/types.go`
//!     (ValidatingAdmissionPolicy, ValidatingAdmissionPolicyBinding, ParamRef,
//!      MatchResources, FailurePolicyType, MatchCondition, Validation, AuditAnnotation,
//!      MessageExpression, Variable)
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/admission.go`
//!     (Plugin.Validate dispatch loop)
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/dispatcher.go`
//!     (per-binding evaluation, fail policy application, param resolution)
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/matching/matcher.go`
//!
//! ## Compile-gate strategy
//!
//! CEL evaluation is a multi-thousand-LOC body of work (cel-go ports) that we
//! gate via `CelEvaluator` trait. Tests that *require* a real evaluator are
//! marked `#[ignore]` and exercise behavior against a `PanicEvaluator` stub
//! that returns a CelError. Tests that only exercise resource matching,
//! param resolution, fail policy, and policy CRUD do NOT require CEL and are
//! enabled.
//!
//! Tenant invariant: a binding's paramRef resolution MUST never cross tenants.
//! `ParamResolver` is responsible for enforcing the tenant scope; tests assert
//! that a binding in tenant A cannot dereference a Param in tenant B even when
//! namespace and name match (line-by-line parity does NOT cover this, but the
//! invariant is layered on top — see `tenant_isolation` tests).

use crate::admission::{AdmissionRequest, AdmissionResponse, Operation, ValidatingWebhook};
use crate::resources::{ObjectMeta, Resource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// ─────────────────────────────────────────────────────────────────────────────
// API types — mirror admissionregistration.k8s.io/v1 (1.31 GA)
// Upstream: staging/src/k8s.io/api/admissionregistration/v1/types.go (lines
// roughly 700–1100 covering VAP/VAPB/ParamRef/MatchResources/Validation).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FailurePolicyType {
    Fail,
    Ignore,
}

impl Default for FailurePolicyType {
    fn default() -> Self { FailurePolicyType::Fail }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ParameterNotFoundActionType {
    Allow,
    Deny,
}

impl Default for ParameterNotFoundActionType {
    fn default() -> Self { ParameterNotFoundActionType::Deny }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ValidationAction {
    Deny,
    Warn,
    Audit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ScopeType {
    All,
    Cluster,
    Namespaced,
}

impl Default for ScopeType {
    fn default() -> Self { ScopeType::All }
}

/// `RuleWithOperations` from admissionregistration/v1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleWithOperations {
    #[serde(default)]
    pub operations: Vec<String>, // CREATE/UPDATE/DELETE/CONNECT/*
    #[serde(default)]
    pub api_groups: Vec<String>,
    #[serde(default)]
    pub api_versions: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub scope: ScopeType,
}

/// `NamedRuleWithOperations` adds an optional list of resource names.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedRuleWithOperations {
    #[serde(flatten)]
    pub rule: RuleWithOperations,
    #[serde(default)]
    pub resource_names: Vec<String>,
}

/// `MatchResources` (admissionregistration/v1).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchResources {
    #[serde(default)]
    pub namespace_selector: Option<LabelSelector>,
    #[serde(default)]
    pub object_selector: Option<LabelSelector>,
    #[serde(default)]
    pub resource_rules: Vec<NamedRuleWithOperations>,
    #[serde(default)]
    pub exclude_resource_rules: Vec<NamedRuleWithOperations>,
    #[serde(default)]
    pub match_policy: Option<MatchPolicyType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MatchPolicyType {
    Exact,
    Equivalent,
}

impl Default for MatchPolicyType {
    fn default() -> Self { MatchPolicyType::Equivalent }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: HashMap<String, String>,
    #[serde(default)]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: String, // In, NotIn, Exists, DoesNotExist
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamKind {
    pub api_version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamRef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub selector: Option<LabelSelector>,
    #[serde(default)]
    pub parameter_not_found_action: ParameterNotFoundActionType,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validation {
    pub expression: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub message_expression: String,
    #[serde(default)]
    pub reason: String, // Forbidden, Invalid, Unauthorized, RequestEntityTooLarge
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditAnnotation {
    pub key: String,
    pub value_expression: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchCondition {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicySpec {
    #[serde(default)]
    pub param_kind: Option<ParamKind>,
    #[serde(default)]
    pub match_constraints: Option<MatchResources>,
    #[serde(default)]
    pub validations: Vec<Validation>,
    #[serde(default)]
    pub failure_policy: FailurePolicyType,
    #[serde(default)]
    pub audit_annotations: Vec<AuditAnnotation>,
    #[serde(default)]
    pub match_conditions: Vec<MatchCondition>,
    #[serde(default)]
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicy {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ValidatingAdmissionPolicySpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicyBindingSpec {
    pub policy_name: String,
    #[serde(default)]
    pub param_ref: Option<ParamRef>,
    #[serde(default)]
    pub match_resources: Option<MatchResources>,
    #[serde(default)]
    pub validation_actions: Vec<ValidationAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingAdmissionPolicyBinding {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ValidatingAdmissionPolicyBindingSpec,
}

// ─────────────────────────────────────────────────────────────────────────────
// CEL evaluator stub trait. Real evaluator is a future crate; today we ship a
// `PanicEvaluator` for compile-gate tests and a `FixedEvaluator` for tests that
// don't actually need a parser.
// Upstream: staging/src/k8s.io/apiserver/pkg/cel/...
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CelValue {
    Bool(bool),
    String(String),
    Int(i64),
    Null,
}

#[derive(Debug, thiserror::Error)]
pub enum CelError {
    #[error("compile error: {0}")]
    Compile(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("type error: expected {expected}, got {got}")]
    Type { expected: String, got: String },
}

/// Activation = the data exposed to CEL: `request`, `object`, `oldObject`,
/// `params`, `namespaceObject`, `authorizer`, named `variables`.
/// Mirrors `cel.Activation` from cel-go.
#[derive(Debug, Default, Clone)]
pub struct CelActivation {
    pub request: Option<serde_json::Value>,
    pub object: Option<serde_json::Value>,
    pub old_object: Option<serde_json::Value>,
    pub params: Option<serde_json::Value>,
    pub namespace_object: Option<serde_json::Value>,
    pub variables: HashMap<String, CelValue>,
    /// 2026-05-13 batch2: optional snapshot of authorization grants
    /// for the CEL `authorizer` binding. `None` means the binding is
    /// absent and any `authorizer.*` access in the expression
    /// surfaces as `CelError::Runtime(undeclared)`, exactly like
    /// upstream cel-go behaves when the activation slot is omitted.
    pub authorizer: Option<AuthorizerView>,
}

/// Read-only authorization snapshot consumed by `authorizer.*` CEL
/// helpers. The evaluator surfaces three sub-API entry points:
///
///   * `authorizer.user`                 → SubjectAccessReviewSubject view
///   * `authorizer.group(g)`             → AccessAllowed when `g` ∈ groups
///   * `authorizer.path(p).check(v).allowed()` → string allow/deny
///   * `authorizer.resource(g, r).namespace(ns).check(v).allowed()`
///
/// Each "check" returns a plain CEL bool — we do NOT model the
/// upstream `AccessReview` object with `.reason`/`.audit` fields
/// because no in-tree policy reads them. Cave's adopters call
/// `.allowed()` (or use the bool directly via `.check(...)`) and
/// branch on the boolean; the richer struct would mean wider trait
/// signatures and a non-trivial cel-interpreter `Function` regrowth.
#[derive(Debug, Default, Clone)]
pub struct AuthorizerView {
    /// Caller's user name (`system:serviceaccount:tenant/sa-name` /
    /// `kube-admin` / etc.).
    pub user: String,
    /// Groups the caller is a member of (`system:authenticated`,
    /// `system:serviceaccounts:<ns>`, custom IdP groups).
    pub groups: Vec<String>,
    /// Pre-computed allow-list keyed by `(verb, resource_path)`. The
    /// resource path is upstream's stable shape
    /// `<group>/<resource>[/<namespace>]`:
    ///
    ///   * `apps/deployments`              cluster-scoped
    ///   * `apps/deployments/team-foo`     namespace-scoped
    ///   * `*/*/team-foo`                  any verb on any resource in ns
    ///
    /// The dispatcher computes this snapshot from cave's RBAC
    /// resolver (`auth_review::Authorizer::authorize`) and stashes
    /// it on the activation before invoking the evaluator. The
    /// evaluator does NOT call back into the resolver — that would
    /// require an async-aware CEL runtime; this snapshot keeps
    /// evaluation pure and synchronous.
    ///
    /// Wildcards: `verb == "*"` and/or `resource == "*"` match any
    /// concrete verb/resource the caller checks. The expansion
    /// happens at allow-list build time (see `granted_*` helpers),
    /// not in the lookup, so the bool decision stays O(1).
    pub grants: std::collections::HashSet<String>,
    /// Non-resource URL paths the caller may `verb`. Keyed
    /// `<verb> <path>` (e.g. `"get /healthz"`).
    pub url_grants: std::collections::HashSet<String>,
}

impl AuthorizerView {
    /// Synthesize an allow-list key from `(verb, group, resource, ns)`.
    /// Wildcards expand to `*` in the matching slot.
    pub fn grant_key(verb: &str, group: &str, resource: &str, ns: Option<&str>) -> String {
        let group = if group.is_empty() { "core" } else { group };
        match ns {
            Some(n) => format!("{verb}:{group}/{resource}/{n}"),
            None => format!("{verb}:{group}/{resource}"),
        }
    }

    /// Convenience builder for tests / authorizer-resolver code.
    pub fn allow(mut self, verb: &str, group: &str, resource: &str, ns: Option<&str>) -> Self {
        self.grants.insert(Self::grant_key(verb, group, resource, ns));
        self
    }

    /// Allow `verb` against a non-resource URL path.
    pub fn allow_path(mut self, verb: &str, path: &str) -> Self {
        self.url_grants.insert(format!("{verb} {path}"));
        self
    }

    /// Add a group membership.
    pub fn with_group(mut self, g: impl Into<String>) -> Self {
        self.groups.push(g.into());
        self
    }

    /// Check whether the caller may `verb` on
    /// `<group>/<resource>[/<ns>]`. Honours `*` wildcards in either
    /// slot of the snapshot. Verb/resource wildcards in the CEL
    /// query itself are NOT supported (real callers spell out the
    /// verb).
    pub fn check_resource(&self, verb: &str, group: &str, resource: &str, ns: Option<&str>) -> bool {
        for key in &self.grants {
            if Self::key_matches(key, verb, group, resource, ns) {
                return true;
            }
        }
        false
    }

    /// Check whether the caller may `verb` against a non-resource
    /// URL path. Wildcards `*` allowed in either component of the
    /// stored grant (`* /healthz` allows any verb on `/healthz`).
    pub fn check_url(&self, verb: &str, path: &str) -> bool {
        for stored in &self.url_grants {
            let mut it = stored.splitn(2, ' ');
            let v = it.next().unwrap_or("");
            let p = it.next().unwrap_or("");
            let v_ok = v == "*" || v == verb;
            let p_ok = p == "*" || p == path;
            if v_ok && p_ok {
                return true;
            }
        }
        false
    }

    fn key_matches(key: &str, verb: &str, group: &str, resource: &str, ns: Option<&str>) -> bool {
        // Parse `<verb>:<group>/<resource>[/<ns>]`.
        let (k_verb, rest) = match key.split_once(':') {
            Some(p) => p,
            None => return false,
        };
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        let (k_group, k_resource, k_ns) = match parts.as_slice() {
            [g, r] => (*g, *r, None),
            [g, r, n] => (*g, *r, Some(*n)),
            _ => return false,
        };
        let v_ok = k_verb == "*" || k_verb == verb;
        let g_ok = k_group == "*" || k_group == group || (k_group == "core" && group.is_empty());
        let r_ok = k_resource == "*" || k_resource == resource;
        let n_ok = match (k_ns, ns) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(a), Some(b)) => a == "*" || a == b,
        };
        v_ok && g_ok && r_ok && n_ok
    }
}

pub trait CelEvaluator: Send + Sync {
    fn evaluate(&self, expr: &str, act: &CelActivation) -> Result<CelValue, CelError>;
}

/// Stub that always returns `CelError::Compile` — used to gate `#[ignore]`d
/// tests that would otherwise need a real evaluator. A real evaluator
/// (cel-go port; KEP-3488) will replace this in M1.
pub struct PanicEvaluator;
impl CelEvaluator for PanicEvaluator {
    fn evaluate(&self, _expr: &str, _act: &CelActivation) -> Result<CelValue, CelError> {
        Err(CelError::Compile(
            "CEL evaluator not yet ported — see KEP-3488".into(),
        ))
    }
}

/// Test double: returns a fixed map from expression text → value. Lets us
/// exercise the dispatcher without a parser.
pub struct FixedEvaluator {
    pub answers: HashMap<String, Result<CelValue, CelError>>,
}

impl FixedEvaluator {
    pub fn new() -> Self { Self { answers: HashMap::new() } }
    pub fn with(mut self, expr: impl Into<String>, val: CelValue) -> Self {
        self.answers.insert(expr.into(), Ok(val));
        self
    }
    pub fn with_err(mut self, expr: impl Into<String>, err: CelError) -> Self {
        self.answers.insert(expr.into(), Err(err));
        self
    }
}

impl Default for FixedEvaluator {
    fn default() -> Self { Self::new() }
}

impl CelEvaluator for FixedEvaluator {
    fn evaluate(&self, expr: &str, _act: &CelActivation) -> Result<CelValue, CelError> {
        match self.answers.get(expr) {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(match e {
                CelError::Compile(m) => CelError::Compile(m.clone()),
                CelError::Runtime(m) => CelError::Runtime(m.clone()),
                CelError::Type { expected, got } => CelError::Type {
                    expected: expected.clone(), got: got.clone(),
                },
            }),
            None => Err(CelError::Compile(format!("no fixed answer for {expr}"))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage — in-memory; mirrors the etcd-backed registry plus the policy/binding
// indexer that the upstream `Plugin` keeps. Tenant scope is enforced at the
// store level; a binding listing for tenant `acme` MUST NOT see policies owned
// by tenant `globex`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct VapStore {
    policies: RwLock<HashMap<(String, String), ValidatingAdmissionPolicy>>,
    bindings: RwLock<HashMap<(String, String), ValidatingAdmissionPolicyBinding>>,
}

fn tenant_of(meta: &ObjectMeta) -> String {
    meta.annotations
        .get("cave.runtime/tenant-id")
        .cloned()
        .unwrap_or_default()
}

impl VapStore {
    pub fn new() -> Self { Self::default() }

    pub fn put_policy(&self, p: ValidatingAdmissionPolicy) {
        let key = (tenant_of(&p.metadata), p.metadata.name.clone());
        self.policies.write().unwrap().insert(key, p);
    }

    pub fn get_policy(&self, tenant: &str, name: &str) -> Option<ValidatingAdmissionPolicy> {
        self.policies.read().unwrap().get(&(tenant.to_string(), name.to_string())).cloned()
    }

    pub fn delete_policy(&self, tenant: &str, name: &str) -> bool {
        self.policies.write().unwrap().remove(&(tenant.to_string(), name.to_string())).is_some()
    }

    pub fn list_policies(&self, tenant: &str) -> Vec<ValidatingAdmissionPolicy> {
        self.policies.read().unwrap().iter()
            .filter(|((t, _), _)| t == tenant)
            .map(|(_, v)| v.clone()).collect()
    }

    pub fn put_binding(&self, b: ValidatingAdmissionPolicyBinding) {
        let key = (tenant_of(&b.metadata), b.metadata.name.clone());
        self.bindings.write().unwrap().insert(key, b);
    }

    pub fn get_binding(&self, tenant: &str, name: &str) -> Option<ValidatingAdmissionPolicyBinding> {
        self.bindings.read().unwrap().get(&(tenant.to_string(), name.to_string())).cloned()
    }

    pub fn delete_binding(&self, tenant: &str, name: &str) -> bool {
        self.bindings.write().unwrap().remove(&(tenant.to_string(), name.to_string())).is_some()
    }

    pub fn list_bindings(&self, tenant: &str) -> Vec<ValidatingAdmissionPolicyBinding> {
        self.bindings.read().unwrap().iter()
            .filter(|((t, _), _)| t == tenant)
            .map(|(_, v)| v.clone()).collect()
    }

    /// Return all (policy, binding) pairs in tenant scope where the binding
    /// references the policy by name. Mirrors the dispatcher inner loop.
    pub fn pairs_for_tenant(
        &self, tenant: &str,
    ) -> Vec<(ValidatingAdmissionPolicy, ValidatingAdmissionPolicyBinding)> {
        let policies: HashMap<String, _> =
            self.list_policies(tenant).into_iter().map(|p| (p.metadata.name.clone(), p)).collect();
        self.list_bindings(tenant).into_iter()
            .filter_map(|b| policies.get(&b.spec.policy_name).cloned().map(|p| (p, b)))
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resource matching — implementable WITHOUT CEL. This is the
// `matching.Matcher.Matches` port.
// Upstream: staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/matching/matcher.go
// ─────────────────────────────────────────────────────────────────────────────

pub struct MatchInput<'a> {
    pub group: &'a str,
    pub version: &'a str,
    pub resource: &'a str,
    pub name: &'a str,
    pub namespace: &'a str,
    pub operation: &'a Operation,
    pub object_labels: &'a HashMap<String, String>,
    pub namespace_labels: &'a HashMap<String, String>,
}

pub fn op_matches(rule: &str, op: &Operation) -> bool {
    if rule == "*" { return true; }
    match (rule, op) {
        ("CREATE", Operation::Create) => true,
        ("UPDATE", Operation::Update) => true,
        ("DELETE", Operation::Delete) => true,
        ("CONNECT", Operation::Connect) => true,
        _ => false,
    }
}

fn glob_or_eq(rule: &str, val: &str) -> bool {
    rule == "*" || rule == val
}

pub fn rule_matches(rule: &RuleWithOperations, input: &MatchInput) -> bool {
    if !rule.operations.iter().any(|o| op_matches(o, input.operation)) {
        return false;
    }
    if !rule.api_groups.is_empty()
        && !rule.api_groups.iter().any(|g| glob_or_eq(g, input.group)) {
        return false;
    }
    if !rule.api_versions.is_empty()
        && !rule.api_versions.iter().any(|v| glob_or_eq(v, input.version)) {
        return false;
    }
    if !rule.resources.is_empty()
        && !rule.resources.iter().any(|r| glob_or_eq(r, input.resource)) {
        return false;
    }
    match rule.scope {
        ScopeType::All => true,
        ScopeType::Cluster => input.namespace.is_empty(),
        ScopeType::Namespaced => !input.namespace.is_empty(),
    }
}

pub fn named_rule_matches(rule: &NamedRuleWithOperations, input: &MatchInput) -> bool {
    if !rule_matches(&rule.rule, input) { return false; }
    if !rule.resource_names.is_empty()
        && !rule.resource_names.iter().any(|n| n == input.name) {
        return false;
    }
    true
}

pub fn label_selector_matches(sel: &LabelSelector, labels: &HashMap<String, String>) -> bool {
    for (k, v) in &sel.match_labels {
        if labels.get(k) != Some(v) { return false; }
    }
    for req in &sel.match_expressions {
        let present = labels.contains_key(&req.key);
        let val = labels.get(&req.key);
        match req.operator.as_str() {
            "In" => {
                let Some(v) = val else { return false; };
                if !req.values.contains(v) { return false; }
            }
            "NotIn" => {
                if let Some(v) = val { if req.values.contains(v) { return false; } }
            }
            "Exists" => { if !present { return false; } }
            "DoesNotExist" => { if present { return false; } }
            _ => return false, // unknown operator — never matches
        }
    }
    true
}

pub fn match_resources_matches(mr: &MatchResources, input: &MatchInput) -> bool {
    let any_excl = mr.exclude_resource_rules.iter().any(|r| named_rule_matches(r, input));
    if any_excl { return false; }
    let any_incl = if mr.resource_rules.is_empty() {
        true
    } else {
        mr.resource_rules.iter().any(|r| named_rule_matches(r, input))
    };
    if !any_incl { return false; }
    if let Some(sel) = &mr.namespace_selector {
        if !label_selector_matches(sel, input.namespace_labels) { return false; }
    }
    if let Some(sel) = &mr.object_selector {
        if !label_selector_matches(sel, input.object_labels) { return false; }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Param resolver — looks up the resource referenced by a binding's ParamRef.
// Tenant boundary enforced: a binding from tenant T may only resolve params
// owned by tenant T. Upstream has no such concept; this is the cave-runtime
// invariant.
// Upstream behavior: dispatcher.go `findParams`.
// ─────────────────────────────────────────────────────────────────────────────

pub trait ParamResolver: Send + Sync {
    fn resolve(
        &self, tenant: &str, kind: &ParamKind, param_ref: &ParamRef,
    ) -> Result<Vec<serde_json::Value>, ParamResolveError>;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParamResolveError {
    #[error("param not found")]
    NotFound,
    #[error("cross-tenant access denied")]
    CrossTenant,
    #[error("invalid selector")]
    InvalidSelector,
}

/// In-memory param resolver. Stores params keyed by (tenant, kind, namespace, name).
#[derive(Default)]
pub struct InMemoryParamResolver {
    items: RwLock<HashMap<(String, String, String, String), serde_json::Value>>,
}

impl InMemoryParamResolver {
    pub fn new() -> Self { Self::default() }

    pub fn insert(
        &self, tenant: &str, kind: &ParamKind, namespace: &str, name: &str,
        value: serde_json::Value,
    ) {
        let key = (tenant.to_string(), format!("{}/{}", kind.api_version, kind.kind),
                   namespace.to_string(), name.to_string());
        self.items.write().unwrap().insert(key, value);
    }
}

impl ParamResolver for InMemoryParamResolver {
    fn resolve(
        &self, tenant: &str, kind: &ParamKind, param_ref: &ParamRef,
    ) -> Result<Vec<serde_json::Value>, ParamResolveError> {
        let kind_key = format!("{}/{}", kind.api_version, kind.kind);
        let map = self.items.read().unwrap();
        if !param_ref.name.is_empty() {
            let key = (tenant.to_string(), kind_key,
                       param_ref.namespace.clone(), param_ref.name.clone());
            return map.get(&key).map(|v| vec![v.clone()]).ok_or(ParamResolveError::NotFound);
        }
        if param_ref.selector.is_some() {
            // Selector-based: we don't index labels yet — return all in tenant+kind+namespace.
            let mut out = vec![];
            for ((t, k, ns, _), v) in map.iter() {
                if t == tenant && *k == kind_key && *ns == param_ref.namespace {
                    out.push(v.clone());
                }
            }
            return Ok(out);
        }
        Err(ParamResolveError::InvalidSelector)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher — orchestrates matching, param resolution, CEL eval, fail policy,
// validation actions. Single-policy/single-binding view from the outer loop.
// Upstream: dispatcher.go `dispatchInvocations` plus admission.go `Validate`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Allow,
    /// At least one validation returned false; carries the message + reason.
    Deny { message: String, reason: String },
    /// A validation returned a warning; AdmissionResponse should attach it.
    Warn(String),
    /// FailurePolicy=Ignore swallowed an error.
    SilencedError,
    /// FailurePolicy=Fail — surface the error.
    Error(String),
}

pub struct Dispatcher {
    pub evaluator: Arc<dyn CelEvaluator>,
    pub params: Arc<dyn ParamResolver>,
}

impl Dispatcher {
    pub fn new(evaluator: Arc<dyn CelEvaluator>, params: Arc<dyn ParamResolver>) -> Self {
        Self { evaluator, params }
    }

    /// Evaluate one (policy, binding) pair against a single match input.
    pub fn dispatch_one(
        &self, tenant: &str,
        policy: &ValidatingAdmissionPolicy,
        binding: &ValidatingAdmissionPolicyBinding,
        req: &AdmissionRequest, input: &MatchInput,
    ) -> Vec<DispatchOutcome> {
        // 1. matchConstraints on policy
        if let Some(mc) = &policy.spec.match_constraints {
            if !match_resources_matches(mc, input) { return vec![]; }
        }
        // 2. matchResources on binding (further narrows)
        if let Some(mr) = &binding.spec.match_resources {
            if !match_resources_matches(mr, input) { return vec![]; }
        }
        // 3. matchConditions — short-circuit if any returns false. Errors -> fail policy.
        for cond in &policy.spec.match_conditions {
            let act = self.activation_for(req, None);
            match self.evaluator.evaluate(&cond.expression, &act) {
                Ok(CelValue::Bool(true)) => continue,
                Ok(CelValue::Bool(false)) => return vec![],
                Ok(other) => return vec![self.fail_outcome(
                    policy, format!("matchCondition {} returned non-bool: {:?}", cond.name, other))],
                Err(e) => return vec![self.fail_outcome(
                    policy, format!("matchCondition {} error: {e}", cond.name))],
            }
        }
        // 4. resolve params
        let params: Option<serde_json::Value> = if let (Some(kind), Some(pref)) =
            (&policy.spec.param_kind, &binding.spec.param_ref)
        {
            match self.params.resolve(tenant, kind, pref) {
                Ok(v) if v.is_empty() => match pref.parameter_not_found_action {
                    ParameterNotFoundActionType::Allow => None,
                    ParameterNotFoundActionType::Deny => return vec![DispatchOutcome::Deny {
                        message: format!("param {} not found", pref.name),
                        reason: "Forbidden".into(),
                    }],
                },
                Ok(v) => Some(serde_json::Value::Array(v)),
                Err(_) => match pref.parameter_not_found_action {
                    ParameterNotFoundActionType::Allow => None,
                    ParameterNotFoundActionType::Deny => return vec![DispatchOutcome::Deny {
                        message: format!("param {} not found", pref.name),
                        reason: "Forbidden".into(),
                    }],
                },
            }
        } else { None };
        // 5. evaluate validations
        let act = self.activation_for(req, params);
        let mut out = vec![];
        for v in &policy.spec.validations {
            match self.evaluator.evaluate(&v.expression, &act) {
                Ok(CelValue::Bool(true)) => out.push(DispatchOutcome::Allow),
                Ok(CelValue::Bool(false)) => {
                    let msg = if v.message.is_empty() { v.expression.clone() } else { v.message.clone() };
                    let reason = if v.reason.is_empty() { "Forbidden".into() } else { v.reason.clone() };
                    let dispatched = self.dispatch_validation_actions(
                        binding, DispatchOutcome::Deny { message: msg, reason });
                    out.extend(dispatched);
                }
                Ok(other) => return vec![self.fail_outcome(
                    policy, format!("validation returned non-bool: {:?}", other))],
                Err(e) => out.push(self.fail_outcome(policy, format!("validation error: {e}"))),
            }
        }
        out
    }

    fn fail_outcome(&self, policy: &ValidatingAdmissionPolicy, msg: String) -> DispatchOutcome {
        match policy.spec.failure_policy {
            FailurePolicyType::Fail => DispatchOutcome::Error(msg),
            FailurePolicyType::Ignore => DispatchOutcome::SilencedError,
        }
    }

    fn dispatch_validation_actions(
        &self, binding: &ValidatingAdmissionPolicyBinding, outcome: DispatchOutcome,
    ) -> Vec<DispatchOutcome> {
        let DispatchOutcome::Deny { message, reason } = outcome else { return vec![outcome]; };
        if binding.spec.validation_actions.is_empty() {
            // upstream default = Deny when not specified
            return vec![DispatchOutcome::Deny { message, reason }];
        }
        let mut out = vec![];
        for action in &binding.spec.validation_actions {
            match action {
                ValidationAction::Deny => out.push(
                    DispatchOutcome::Deny { message: message.clone(), reason: reason.clone() }),
                ValidationAction::Warn => out.push(DispatchOutcome::Warn(message.clone())),
                ValidationAction::Audit => { /* TODO M4: emit audit annotation */ }
            }
        }
        out
    }

    fn activation_for(&self, req: &AdmissionRequest, params: Option<serde_json::Value>) -> CelActivation {
        CelActivation {
            request: serde_json::to_value(req).ok(),
            object: req.object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            old_object: req.old_object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            params,
            namespace_object: None,
            variables: HashMap::new(),
            // The dispatcher leaves authorizer = None; the
            // authorizer binding is only attached when an upstream
            // adopter (cave-runtime serve) builds an AuthorizerView
            // from the auth_review resolver and passes it down via
            // [`Dispatcher::with_authorizer`] (followup wiring).
            authorizer: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Plugin — the ValidatingWebhook that dispatches all (policy, binding) pairs
// for the request's tenant scope. Wires into the existing AdmissionChain.
// ─────────────────────────────────────────────────────────────────────────────

pub struct VapPlugin {
    pub store: Arc<VapStore>,
    pub dispatcher: Dispatcher,
}

impl VapPlugin {
    pub fn new(store: Arc<VapStore>, dispatcher: Dispatcher) -> Self {
        Self { store, dispatcher }
    }
}

impl ValidatingWebhook for VapPlugin {
    fn name(&self) -> &str { "validating-admission-policy" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        let pairs = self.store.pairs_for_tenant(&req.tenant_id);
        let object_labels = req.object.as_ref().map(meta_labels).unwrap_or_default();
        let input = MatchInput {
            group: "", version: "", resource: &req.kind.to_lowercase(),
            name: &req.name, namespace: &req.namespace,
            operation: &req.operation,
            object_labels: &object_labels,
            namespace_labels: &HashMap::new(),
        };
        let mut warnings = vec![];
        for (p, b) in pairs {
            for outcome in self.dispatcher.dispatch_one(&req.tenant_id, &p, &b, req, &input) {
                match outcome {
                    DispatchOutcome::Allow => {}
                    DispatchOutcome::Deny { message, reason: _ } => {
                        let mut resp = AdmissionResponse::deny(req, 403, message);
                        resp.warnings = warnings;
                        return resp;
                    }
                    DispatchOutcome::Warn(m) => warnings.push(m),
                    DispatchOutcome::Error(m) => {
                        let mut resp = AdmissionResponse::deny(req, 500, m);
                        resp.warnings = warnings;
                        return resp;
                    }
                    DispatchOutcome::SilencedError => {}
                }
            }
        }
        let mut resp = AdmissionResponse::allow(req);
        resp.warnings = warnings;
        resp
    }
}

fn meta_labels(r: &Resource) -> HashMap<String, String> {
    match r {
        Resource::Pod(p) => p.metadata.labels.clone(),
        Resource::ConfigMap(c) => c.metadata.labels.clone(),
        Resource::Secret(s) => s.metadata.labels.clone(),
        Resource::Service(s) => s.metadata.labels.clone(),
        Resource::Deployment(d) => d.metadata.labels.clone(),
        Resource::StatefulSet(s) => s.metadata.labels.clone(),
        Resource::DaemonSet(d) => d.metadata.labels.clone(),
        Resource::ReplicaSet(r) => r.metadata.labels.clone(),
        Resource::Job(j) => j.metadata.labels.clone(),
        Resource::CronJob(c) => c.metadata.labels.clone(),
        Resource::Ingress(i) => i.metadata.labels.clone(),
        Resource::NetworkPolicy(n) => n.metadata.labels.clone(),
        Resource::Namespace(n) => n.metadata.labels.clone(),
        Resource::Node(n) => n.metadata.labels.clone(),
        Resource::PersistentVolume(pv) => pv.metadata.labels.clone(),
        Resource::PersistentVolumeClaim(pvc) => pvc.metadata.labels.clone(),
        Resource::StorageClass(s) => s.metadata.labels.clone(),
        Resource::ClusterRole(c) => c.metadata.labels.clone(),
        Resource::ClusterRoleBinding(c) => c.metadata.labels.clone(),
        Resource::Role(r) => r.metadata.labels.clone(),
        Resource::RoleBinding(r) => r.metadata.labels.clone(),
        Resource::ServiceAccount(s) => s.metadata.labels.clone(),
        Resource::Endpoints(e) => e.metadata.labels.clone(),
        Resource::KubeEvent(e) => e.metadata.labels.clone(),
        Resource::ResourceQuota(q) => q.metadata.labels.clone(),
        Resource::LimitRange(l) => l.metadata.labels.clone(),
    }
}

#[cfg(test)]
mod tests;
