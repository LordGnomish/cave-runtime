//! MutatingAdmissionPolicy v2 (KEP-3962, alpha 1.32, beta 1.33).
//!
//! Sister module to `vap_advanced` (validating). Reuses the same
//! `CelEvaluator` stub trait so that when the real evaluator lands a
//! single drop-in serves both phases.
//!
//! Upstream sources (kubernetes/kubernetes v1.31+):
//!   * `staging/src/k8s.io/api/admissionregistration/v1alpha1/types.go`
//!     (`MutatingAdmissionPolicy`, `MutatingAdmissionPolicyBinding`,
//!      `Mutation` { patchType, applyConfiguration, jsonPatch },
//!      `JSONPatch.expression` returning `[]JSONPatchOp`,
//!      `ApplyConfiguration.expression` returning a partial object).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/mutating/`
//!     (dispatcher loop, reinvocation, policy/binding pairs).
//!
//! ## Tenant invariant
//!
//! A mutation MUST NOT touch the `cave.runtime/tenant-id` annotation. Any
//! emitted JSONPatch op whose path is the tenant annotation is dropped
//! and the mutation flips to a Failure outcome.

use crate::admission::{AdmissionRequest, AdmissionResponse, JsonPatch, MutatingWebhook};
use crate::resources::ObjectMeta;
use crate::vap_advanced::{
    CelActivation, CelError, CelEvaluator, CelValue, FailurePolicyType,
    LabelSelector, MatchCondition, MatchInput, MatchResources,
    NamedRuleWithOperations, ParamKind, ParamRef, ParamResolveError,
    ParamResolver, RuleWithOperations, ScopeType, match_resources_matches,
    label_selector_matches, named_rule_matches,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PatchType {
    /// CEL returns a `[]JSONPatchOp` that the apiserver applies as RFC 6902.
    JSONPatch,
    /// CEL returns a partial object that is server-side-applied.
    ApplyConfiguration,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JSONPatchExpression {
    /// CEL expression evaluating to `[]JSONPatchOp`.
    pub expression: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyConfigurationExpression {
    /// CEL expression evaluating to a partial object (Object / Map).
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutation {
    pub patch_type: PatchType,
    #[serde(default)]
    pub json_patch: Option<JSONPatchExpression>,
    #[serde(default)]
    pub apply_configuration: Option<ApplyConfigurationExpression>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ReinvocationPolicyType {
    Never,
    IfNeeded,
}

impl Default for ReinvocationPolicyType {
    fn default() -> Self { ReinvocationPolicyType::Never }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicySpec {
    #[serde(default)]
    pub param_kind: Option<ParamKind>,
    #[serde(default)]
    pub match_constraints: Option<MatchResources>,
    #[serde(default)]
    pub mutations: Vec<Mutation>,
    #[serde(default)]
    pub failure_policy: FailurePolicyType,
    #[serde(default)]
    pub reinvocation_policy: ReinvocationPolicyType,
    #[serde(default)]
    pub match_conditions: Vec<MatchCondition>,
    #[serde(default)]
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicy {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: MutatingAdmissionPolicySpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicyBindingSpec {
    pub policy_name: String,
    #[serde(default)]
    pub param_ref: Option<ParamRef>,
    #[serde(default)]
    pub match_resources: Option<MatchResources>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicyBinding {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: MutatingAdmissionPolicyBindingSpec,
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage — tenant-scoped, mirrors vap_advanced::VapStore.
// ─────────────────────────────────────────────────────────────────────────────

fn tenant_of(meta: &ObjectMeta) -> String {
    meta.annotations.get("cave.runtime/tenant-id").cloned().unwrap_or_default()
}

#[derive(Default)]
pub struct MapStore {
    policies: RwLock<HashMap<(String, String), MutatingAdmissionPolicy>>,
    bindings: RwLock<HashMap<(String, String), MutatingAdmissionPolicyBinding>>,
}

impl MapStore {
    pub fn new() -> Self { Self::default() }

    pub fn put_policy(&self, p: MutatingAdmissionPolicy) {
        let key = (tenant_of(&p.metadata), p.metadata.name.clone());
        self.policies.write().unwrap().insert(key, p);
    }
    pub fn put_binding(&self, b: MutatingAdmissionPolicyBinding) {
        let key = (tenant_of(&b.metadata), b.metadata.name.clone());
        self.bindings.write().unwrap().insert(key, b);
    }
    pub fn list_policies(&self, tenant: &str) -> Vec<MutatingAdmissionPolicy> {
        self.policies.read().unwrap().iter()
            .filter(|((t, _), _)| t == tenant).map(|(_, v)| v.clone()).collect()
    }
    pub fn list_bindings(&self, tenant: &str) -> Vec<MutatingAdmissionPolicyBinding> {
        self.bindings.read().unwrap().iter()
            .filter(|((t, _), _)| t == tenant).map(|(_, v)| v.clone()).collect()
    }
    pub fn pairs_for_tenant(&self, tenant: &str)
        -> Vec<(MutatingAdmissionPolicy, MutatingAdmissionPolicyBinding)>
    {
        let policies: HashMap<String, _> = self.list_policies(tenant)
            .into_iter().map(|p| (p.metadata.name.clone(), p)).collect();
        self.list_bindings(tenant).into_iter()
            .filter_map(|b| policies.get(&b.spec.policy_name).cloned().map(|p| (p, b)))
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CEL value → JSONPatchOp coercion. Real CEL returns rich Object/List
// values; we model the wire shape so tests can drive against it.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonPatchOp {
    Add, Remove, Replace, Move, Copy, Test,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonPatchOpRecord {
    pub op: JsonPatchOp,
    pub path: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub from: Option<String>,
}

impl JsonPatchOpRecord {
    pub fn to_admission_patch(&self) -> JsonPatch {
        JsonPatch {
            op: match self.op {
                JsonPatchOp::Add => "add".into(),
                JsonPatchOp::Remove => "remove".into(),
                JsonPatchOp::Replace => "replace".into(),
                JsonPatchOp::Move => "move".into(),
                JsonPatchOp::Copy => "copy".into(),
                JsonPatchOp::Test => "test".into(),
            },
            path: self.path.clone(),
            value: self.value.clone(),
        }
    }
}

/// Parse a JSON value (the CEL output for a JSONPatch expression) into a
/// list of patch ops. Returns Err on malformed shapes.
pub fn parse_patch_ops(v: &serde_json::Value) -> Result<Vec<JsonPatchOpRecord>, String> {
    let arr = v.as_array().ok_or_else(|| "expected array".to_string())?;
    let mut out = vec![];
    for el in arr {
        let obj = el.as_object().ok_or_else(|| "patch op must be object".to_string())?;
        let op_s = obj.get("op").and_then(|s| s.as_str())
            .ok_or_else(|| "missing op".to_string())?;
        let op = match op_s {
            "add" => JsonPatchOp::Add,
            "remove" => JsonPatchOp::Remove,
            "replace" => JsonPatchOp::Replace,
            "move" => JsonPatchOp::Move,
            "copy" => JsonPatchOp::Copy,
            "test" => JsonPatchOp::Test,
            other => return Err(format!("unknown op {other}")),
        };
        let path = obj.get("path").and_then(|s| s.as_str())
            .ok_or_else(|| "missing path".to_string())?.to_string();
        let value = obj.get("value").cloned();
        let from = obj.get("from").and_then(|s| s.as_str()).map(String::from);
        out.push(JsonPatchOpRecord { op, path, value, from });
    }
    Ok(out)
}

/// Convert an ApplyConfiguration object into JSONPatch ops (additive only).
/// This is a simplified server-side-apply emulation — every leaf field becomes
/// an `add` patch at its full path.
pub fn apply_config_to_patches(v: &serde_json::Value) -> Vec<JsonPatchOpRecord> {
    let mut out = vec![];
    let mut stack: Vec<(String, &serde_json::Value)> = vec![("".into(), v)];
    while let Some((prefix, val)) = stack.pop() {
        match val {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    let path = format!("{prefix}/{}", k.replace('~', "~0").replace('/', "~1"));
                    stack.push((path, v));
                }
            }
            other => {
                if !prefix.is_empty() {
                    out.push(JsonPatchOpRecord {
                        op: JsonPatchOp::Add,
                        path: prefix,
                        value: Some(other.clone()),
                        from: None,
                    });
                }
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Tenant invariant filter — drops any patch op that would touch the
// tenant-id annotation. Returns Err when this would have happened so the
// caller can flip the outcome.
// ─────────────────────────────────────────────────────────────────────────────

pub const TENANT_ANNOTATION_PATH: &str =
    "/metadata/annotations/cave.runtime~1tenant-id";

pub fn enforce_tenant_invariant(
    ops: Vec<JsonPatchOpRecord>,
) -> Result<Vec<JsonPatchOpRecord>, String> {
    for o in &ops {
        if o.path == TENANT_ANNOTATION_PATH {
            return Err(format!(
                "mutation forbidden: op {:?} on tenant-id annotation", o.op));
        }
    }
    Ok(ops)
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MutationOutcome {
    Patches(Vec<JsonPatchOpRecord>),
    Skipped, // matchConstraints / matchConditions filtered
    SilencedError,
    Error(String),
}

pub struct MapDispatcher {
    pub evaluator: Arc<dyn CelEvaluator>,
    pub params: Arc<dyn ParamResolver>,
}

impl MapDispatcher {
    pub fn new(evaluator: Arc<dyn CelEvaluator>, params: Arc<dyn ParamResolver>) -> Self {
        Self { evaluator, params }
    }

    pub fn dispatch_one(
        &self, tenant: &str,
        policy: &MutatingAdmissionPolicy,
        binding: &MutatingAdmissionPolicyBinding,
        req: &AdmissionRequest, input: &MatchInput,
    ) -> MutationOutcome {
        // 1. matchConstraints + matchResources
        if let Some(mc) = &policy.spec.match_constraints {
            if !match_resources_matches(mc, input) { return MutationOutcome::Skipped; }
        }
        if let Some(mr) = &binding.spec.match_resources {
            if !match_resources_matches(mr, input) { return MutationOutcome::Skipped; }
        }
        // 2. matchConditions
        for cond in &policy.spec.match_conditions {
            let act = self.activation_for(req, None);
            match self.evaluator.evaluate(&cond.expression, &act) {
                Ok(CelValue::Bool(true)) => continue,
                Ok(CelValue::Bool(false)) => return MutationOutcome::Skipped,
                Ok(_) => return self.fail_outcome(policy,
                    "matchCondition returned non-bool".to_string()),
                Err(e) => return self.fail_outcome(policy,
                    format!("matchCondition error: {e}")),
            }
        }
        // 3. resolve params
        let params: Option<serde_json::Value> = if let (Some(kind), Some(pref)) =
            (&policy.spec.param_kind, &binding.spec.param_ref)
        {
            match self.params.resolve(tenant, kind, pref) {
                Ok(v) if !v.is_empty() => Some(serde_json::Value::Array(v)),
                Ok(_) | Err(ParamResolveError::NotFound) | Err(_) => None,
            }
        } else { None };
        // 4. evaluate mutations in order
        let act = self.activation_for(req, params);
        let mut all_ops = vec![];
        for m in &policy.spec.mutations {
            match m.patch_type {
                PatchType::JSONPatch => {
                    let Some(jp) = &m.json_patch else {
                        return self.fail_outcome(policy,
                            "mutation type=JSONPatch requires json_patch field".into());
                    };
                    let val = match self.evaluator.evaluate(&jp.expression, &act) {
                        Ok(CelValue::String(s)) => match serde_json::from_str(&s) {
                            Ok(v) => v,
                            Err(e) => return self.fail_outcome(policy,
                                format!("malformed JSONPatch JSON: {e}")),
                        },
                        Ok(_) => return self.fail_outcome(policy,
                            "JSONPatch CEL must return JSON string".into()),
                        Err(e) => return self.fail_outcome(policy, format!("CEL error: {e}")),
                    };
                    let ops = match parse_patch_ops(&val) {
                        Ok(o) => o,
                        Err(e) => return self.fail_outcome(policy, e),
                    };
                    let ops = match enforce_tenant_invariant(ops) {
                        Ok(o) => o,
                        Err(e) => return MutationOutcome::Error(e),
                    };
                    all_ops.extend(ops);
                }
                PatchType::ApplyConfiguration => {
                    let Some(ac) = &m.apply_configuration else {
                        return self.fail_outcome(policy,
                            "mutation type=ApplyConfiguration requires apply_configuration".into());
                    };
                    let val = match self.evaluator.evaluate(&ac.expression, &act) {
                        Ok(CelValue::String(s)) => match serde_json::from_str(&s) {
                            Ok(v) => v,
                            Err(e) => return self.fail_outcome(policy,
                                format!("malformed apply object: {e}")),
                        },
                        Ok(_) => return self.fail_outcome(policy,
                            "ApplyConfig CEL must return JSON string".into()),
                        Err(e) => return self.fail_outcome(policy, format!("CEL error: {e}")),
                    };
                    let ops = apply_config_to_patches(&val);
                    let ops = match enforce_tenant_invariant(ops) {
                        Ok(o) => o,
                        Err(e) => return MutationOutcome::Error(e),
                    };
                    all_ops.extend(ops);
                }
            }
        }
        MutationOutcome::Patches(all_ops)
    }

    fn fail_outcome(&self, policy: &MutatingAdmissionPolicy, msg: String) -> MutationOutcome {
        match policy.spec.failure_policy {
            FailurePolicyType::Fail => MutationOutcome::Error(msg),
            FailurePolicyType::Ignore => MutationOutcome::SilencedError,
        }
    }

    fn activation_for(
        &self, req: &AdmissionRequest, params: Option<serde_json::Value>,
    ) -> CelActivation {
        CelActivation {
            request: serde_json::to_value(req).ok(),
            object: req.object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            old_object: req.old_object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            params, namespace_object: None,
            variables: HashMap::new(),
            authorizer: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MapPlugin — wraps Dispatcher into a MutatingWebhook for the admission chain.
// ─────────────────────────────────────────────────────────────────────────────

pub struct MapPlugin {
    pub store: Arc<MapStore>,
    pub dispatcher: MapDispatcher,
}

impl MutatingWebhook for MapPlugin {
    fn name(&self) -> &str { "mutating-admission-policy" }
    fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
        let mut all_patches: Vec<JsonPatch> = vec![];
        let resource = req.kind.to_lowercase();
        let namespace = req.namespace.clone();
        let operation = req.operation.clone();
        let empty = HashMap::new();
        let input = MatchInput {
            group: "", version: "v1", resource: &resource,
            name: &req.name, namespace: &namespace, operation: &operation,
            object_labels: &empty, namespace_labels: &empty,
        };
        for (p, b) in self.store.pairs_for_tenant(&req.tenant_id) {
            match self.dispatcher.dispatch_one(&req.tenant_id, &p, &b, req, &input) {
                MutationOutcome::Patches(ops) => {
                    for o in ops { all_patches.push(o.to_admission_patch()); }
                }
                MutationOutcome::Skipped | MutationOutcome::SilencedError => {}
                MutationOutcome::Error(m) => {
                    return AdmissionResponse::deny(req, 500, m);
                }
            }
        }
        let mut resp = AdmissionResponse::allow(req);
        resp.patches = all_patches;
        resp
    }
}

// Help mark these as exported so `unused_*` checks don't trip on the
// re-exported types from vap_advanced.
#[allow(dead_code)]
fn unused_keepalive_re_exports() {
    let _ = (
        std::mem::size_of::<ScopeType>(),
        std::mem::size_of::<RuleWithOperations>(),
        std::mem::size_of::<NamedRuleWithOperations>(),
        std::mem::size_of::<LabelSelector>(),
        std::mem::size_of::<CelError>(),
    );
    let _ = label_selector_matches;
    let _ = named_rule_matches;
}

#[cfg(test)]
mod tests;
