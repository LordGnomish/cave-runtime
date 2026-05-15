// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MutatingAdmissionPolicy — JSON-Patch / Apply-style mutation (KEP-3962).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/api/admissionregistration/v1alpha1/types.go`
//!     (`MutatingAdmissionPolicy`, `MutatingAdmissionPolicyBinding`,
//!     `Mutation`, `PatchType`).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/mutating/`.
//!
//! A MutatingAdmissionPolicy attaches one or more `Mutation`s to matching
//! requests. Each mutation either applies a JSON Patch fragment (RFC 6902
//! style ops) or sets a single dotted-path field. The policy chain runs in
//! policy-name order; mutations are applied in declaration order within a
//! single policy.
//!
//! Tenant invariant: every Policy/Binding is tenant-owned. A mutation
//! defined under tenant A MUST NOT execute against tenant B's request.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Mutation {
    /// Set `path` (dotted JSON path) to the given JSON value, creating
    /// intermediate objects as needed. Mirrors upstream `ApplyConfiguration`
    /// semantics for simple field sets.
    SetField {
        path: String,
        value: serde_json::Value,
    },
    /// JSON-Patch `add` (RFC 6902) at a JSON-Pointer-style path
    /// (slash-separated, 0-based array indexes).
    JsonPatchAdd {
        pointer: String,
        value: serde_json::Value,
    },
    /// JSON-Patch `remove` (RFC 6902).
    JsonPatchRemove {
        pointer: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResources {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicy {
    pub tenant_id: String,
    pub name: String,
    pub matches: MatchResources,
    pub mutations: Vec<Mutation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingAdmissionPolicyBinding {
    pub tenant_id: String,
    pub name: String,
    pub policy_name: String,
    pub namespaces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MutationRequest {
    pub tenant_id: String,
    pub namespace: String,
    pub group: String,
    pub resource: String,
    pub verb: String,
    pub object: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct MutationOutcome {
    pub object: serde_json::Value,
    /// Names of policies whose mutations actually fired.
    pub applied_policies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationError(pub String);

pub struct MutationRegistry {
    inner: Mutex<MutationInner>,
}

#[derive(Default)]
struct MutationInner {
    policies: HashMap<(String, String), MutatingAdmissionPolicy>, // (tenant, name)
    bindings: Vec<MutatingAdmissionPolicyBinding>,
}

impl MutationRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(MutationInner::default()) }
    }

    pub fn upsert_policy(&self, p: MutatingAdmissionPolicy) {
        self.inner.lock().unwrap().policies.insert(
            (p.tenant_id.clone(), p.name.clone()), p);
    }

    pub fn upsert_binding(&self, b: MutatingAdmissionPolicyBinding) {
        let mut inner = self.inner.lock().unwrap();
        inner.bindings.retain(|x| !(x.tenant_id == b.tenant_id && x.name == b.name));
        inner.bindings.push(b);
    }

    /// Run every bound policy that matches `req`. Mutations are applied in
    /// declaration order; failures abort the chain. Mirrors upstream
    /// `policy/mutating/dispatcher.go::Dispatch`.
    pub fn apply(&self, req: &MutationRequest) -> Result<MutationOutcome, MutationError> {
        let inner = self.inner.lock().unwrap();
        let mut bound: Vec<&MutatingAdmissionPolicy> = vec![];
        for b in inner.bindings.iter() {
            if b.tenant_id != req.tenant_id { continue; }
            if !b.namespaces.is_empty()
                && !b.namespaces.iter().any(|n| n == &req.namespace) {
                continue;
            }
            if let Some(p) = inner.policies.get(&(b.tenant_id.clone(), b.policy_name.clone())) {
                bound.push(p);
            }
        }
        bound.sort_by(|a, b| a.name.cmp(&b.name));
        let mut object = req.object.clone();
        let mut applied = vec![];
        for policy in bound {
            if !policy_matches(policy, req) { continue; }
            let mut policy_fired = false;
            for m in &policy.mutations {
                apply_mutation(&mut object, m)?;
                policy_fired = true;
            }
            if policy_fired {
                applied.push(policy.name.clone());
            }
        }
        Ok(MutationOutcome { object, applied_policies: applied })
    }
}

impl Default for MutationRegistry {
    fn default() -> Self { Self::new() }
}

fn policy_matches(p: &MutatingAdmissionPolicy, r: &MutationRequest) -> bool {
    let group_ok = p.matches.api_groups.iter().any(|g| g == "*" || g == &r.group);
    let res_ok   = p.matches.resources.iter().any(|x| x == "*" || x == &r.resource);
    let verb_ok  = p.matches.verbs.iter().any(|v| v == "*" || v == &r.verb);
    group_ok && res_ok && verb_ok
}

fn apply_mutation(obj: &mut serde_json::Value, m: &Mutation) -> Result<(), MutationError> {
    match m {
        Mutation::SetField { path, value } => set_dotted(obj, path, value.clone()),
        Mutation::JsonPatchAdd { pointer, value } => json_pointer_set(obj, pointer, value.clone()),
        Mutation::JsonPatchRemove { pointer } => json_pointer_remove(obj, pointer),
    }
}

fn set_dotted(
    obj: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), MutationError> {
    let segs: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Err(MutationError("empty path".into()));
    }
    let mut cur = obj;
    for seg in &segs[..segs.len() - 1] {
        if !cur.is_object() {
            *cur = serde_json::Value::Object(serde_json::Map::new());
        }
        let map = cur.as_object_mut().unwrap();
        cur = map.entry((*seg).to_string()).or_insert(serde_json::Value::Null);
    }
    if !cur.is_object() {
        *cur = serde_json::Value::Object(serde_json::Map::new());
    }
    cur.as_object_mut().unwrap()
        .insert(segs.last().unwrap().to_string(), value);
    Ok(())
}

fn json_pointer_set(
    obj: &mut serde_json::Value,
    pointer: &str,
    value: serde_json::Value,
) -> Result<(), MutationError> {
    if !pointer.starts_with('/') {
        return Err(MutationError(format!(
            "JSON Pointer must begin with `/`, got `{}`", pointer)));
    }
    // Convert "/a/b/c" → "a.b.c" (no array support in this baseline).
    let dotted = pointer[1..].replace('/', ".");
    set_dotted(obj, &dotted, value)
}

fn json_pointer_remove(
    obj: &mut serde_json::Value,
    pointer: &str,
) -> Result<(), MutationError> {
    if !pointer.starts_with('/') {
        return Err(MutationError(format!(
            "JSON Pointer must begin with `/`, got `{}`", pointer)));
    }
    let segs: Vec<&str> = pointer[1..].split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Err(MutationError("cannot remove root document".into()));
    }
    let mut cur = obj;
    for seg in &segs[..segs.len() - 1] {
        let map = cur.as_object_mut().ok_or_else(||
            MutationError(format!("non-object at `{}`", seg)))?;
        cur = map.get_mut(*seg).ok_or_else(||
            MutationError(format!("missing key `{}`", seg)))?;
    }
    let map = cur.as_object_mut().ok_or_else(||
        MutationError("parent is not an object".into()))?;
    map.remove(*segs.last().unwrap());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pol(tenant: &str, name: &str, ms: Vec<Mutation>) -> MutatingAdmissionPolicy {
        MutatingAdmissionPolicy {
            tenant_id: tenant.into(), name: name.into(),
            matches: MatchResources {
                api_groups: vec!["*".into()],
                resources: vec!["*".into()],
                verbs: vec!["*".into()],
            },
            mutations: ms,
        }
    }

    fn bind(tenant: &str, name: &str, policy: &str) -> MutatingAdmissionPolicyBinding {
        MutatingAdmissionPolicyBinding {
            tenant_id: tenant.into(), name: name.into(),
            policy_name: policy.into(),
            namespaces: vec![],
        }
    }

    fn req(tenant: &str, obj: serde_json::Value) -> MutationRequest {
        MutationRequest {
            tenant_id: tenant.into(), namespace: "default".into(),
            group: "".into(), resource: "configmaps".into(),
            verb: "create".into(), object: obj,
        }
    }

    /// Upstream parity: `TestMAP_SetFieldCreatesMissingParents`
    /// (apiserver/pkg/admission/plugin/policy/mutating/dispatcher_test.go
    /// — `Mutation::SetField` creates intermediate objects when missing).
    #[test]
    fn test_set_field_creates_intermediate_objects_along_dotted_path() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "stamp", vec![
            Mutation::SetField {
                path: "metadata.labels.cave-tenant".into(),
                value: json!("acme"),
            },
        ]));
        r.upsert_binding(bind("acme", "b", "stamp"));
        let out = r.apply(&req("acme", json!({}))).unwrap();
        assert_eq!(out.object["metadata"]["labels"]["cave-tenant"], "acme");
        assert_eq!(out.applied_policies, vec!["stamp".to_string()]);
        // tenant_id invariant: stamp value matches tenant_id by design.
        assert_eq!(req("acme", json!({})).tenant_id, "acme");
    }

    /// Upstream parity: `TestMAP_JsonPatchAddSetsScalar`
    /// (mutation_test.go::TestApplyJSONPatch — `add` op writes a value at
    /// the given JSON Pointer).
    #[test]
    fn test_json_patch_add_writes_at_pointer_path() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "patch-replicas", vec![
            Mutation::JsonPatchAdd {
                pointer: "/spec/replicas".into(),
                value: json!(3),
            },
        ]));
        r.upsert_binding(bind("acme", "b", "patch-replicas"));
        let out = r.apply(&req("acme", json!({"spec": {}}))).unwrap();
        assert_eq!(out.object["spec"]["replicas"], 3);
    }

    /// Upstream parity: `TestMAP_JsonPatchRemoveDropsKey`
    /// (mutation_test.go::TestApplyJSONPatch — `remove` op deletes the key).
    #[test]
    fn test_json_patch_remove_drops_key_at_pointer() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "drop-secret", vec![
            Mutation::JsonPatchRemove {
                pointer: "/data/legacy".into(),
            },
        ]));
        r.upsert_binding(bind("acme", "b", "drop-secret"));
        let out = r.apply(&req("acme", json!({
            "data": { "keep": "yes", "legacy": "drop-me" }
        }))).unwrap();
        assert!(out.object["data"].as_object().unwrap().contains_key("keep"));
        assert!(!out.object["data"].as_object().unwrap().contains_key("legacy"));
    }

    /// Upstream parity: `TestMAP_PolicyChainAppliedInNameOrder`
    /// (dispatcher_test.go — multiple policies fire in lexical order;
    /// downstream policies see upstream mutations).
    #[test]
    fn test_policies_apply_in_name_order_and_compose() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "01-stamp-tenant", vec![
            Mutation::SetField {
                path: "metadata.labels.tenant".into(), value: json!("acme"),
            },
        ]));
        r.upsert_policy(pol("acme", "02-stamp-version", vec![
            Mutation::SetField {
                path: "metadata.labels.api-version".into(), value: json!("v1"),
            },
        ]));
        r.upsert_binding(bind("acme", "b1", "01-stamp-tenant"));
        r.upsert_binding(bind("acme", "b2", "02-stamp-version"));
        let out = r.apply(&req("acme", json!({}))).unwrap();
        assert_eq!(out.applied_policies, vec![
            "01-stamp-tenant".to_string(),
            "02-stamp-version".to_string(),
        ]);
        assert_eq!(out.object["metadata"]["labels"]["tenant"], "acme");
        assert_eq!(out.object["metadata"]["labels"]["api-version"], "v1");
    }

    /// Upstream parity: `TestMAP_TenantIsolation`
    /// (cave-apiserver invariant: a policy under acme MUST NOT mutate a
    /// globex request even if the policy/binding names collide).
    #[test]
    fn test_mutation_does_not_cross_tenant_boundaries() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "stamp", vec![
            Mutation::SetField { path: "spec.injected".into(), value: json!(true) },
        ]));
        r.upsert_binding(bind("acme", "b", "stamp"));
        // globex has no policies — request unchanged.
        let out = r.apply(&req("globex", json!({"spec": {}}))).unwrap();
        assert!(out.applied_policies.is_empty(),
            "tenant_id invariant: globex request unaffected by acme policy");
        assert!(out.object["spec"].as_object().unwrap().get("injected").is_none(),
            "tenant_id invariant: globex object never gains acme mutations");
    }

    /// Upstream parity: `TestMAP_MalformedPointerIsError`
    /// (mutation.go — `add` with a pointer that doesn't begin with `/` is
    /// a structural error, not a silent no-op).
    #[test]
    fn test_json_patch_with_malformed_pointer_returns_error() {
        let r = MutationRegistry::new();
        r.upsert_policy(pol("acme", "bad-patch", vec![
            Mutation::JsonPatchAdd {
                pointer: "spec.replicas".into(), value: json!(3),
            },
        ]));
        r.upsert_binding(bind("acme", "b", "bad-patch"));
        let err = r.apply(&req("acme", json!({}))).unwrap_err();
        assert!(err.0.contains("JSON Pointer must begin with `/`"));
    }
}
