// SPDX-License-Identifier: AGPL-3.0-or-later
//! MutatingAdmissionPolicy v2 tests — KEP-3962.

use super::*;
use crate::admission::{AdmissionRequest, Operation};
use crate::resources::{ConfigMap, ObjectMeta, Resource};
use crate::vap_advanced::{
    FixedEvaluator, InMemoryParamResolver, ParamKind, ParamRef,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn map(name: &str, tenant: &str) -> MutatingAdmissionPolicy {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    MutatingAdmissionPolicy {
        api_version: "admissionregistration.k8s.io/v1alpha1".into(),
        kind: "MutatingAdmissionPolicy".into(),
        metadata: meta,
        spec: MutatingAdmissionPolicySpec::default(),
    }
}

fn binding(name: &str, tenant: &str, policy: &str) -> MutatingAdmissionPolicyBinding {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    MutatingAdmissionPolicyBinding {
        api_version: "admissionregistration.k8s.io/v1alpha1".into(),
        kind: "MutatingAdmissionPolicyBinding".into(),
        metadata: meta,
        spec: MutatingAdmissionPolicyBindingSpec {
            policy_name: policy.into(),
            ..Default::default()
        },
    }
}

fn req(op: Operation, ns: &str, tenant: &str) -> AdmissionRequest {
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "ConfigMap".into(), name: "cm".into(),
        operation: op,
        object: Some(Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new("cm", ns), data: HashMap::new(),
        })),
        old_object: None, user: "alice".into(), dry_run: false,
    }
}

fn input<'a>(op: &'a Operation, ns: &'a str, empty: &'a HashMap<String,String>) -> MatchInput<'a> {
    MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm",
        namespace: ns, operation: op,
        object_labels: empty, namespace_labels: empty,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// parse_patch_ops
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn patch_ops_parse_add() {
    let v = json!([{"op":"add","path":"/x","value":1}]);
    let ops = parse_patch_ops(&v).unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op, JsonPatchOp::Add);
    assert_eq!(ops[0].path, "/x");
}

#[test]
fn patch_ops_parse_all_op_kinds() {
    let v = json!([
        {"op":"add","path":"/a","value":1},
        {"op":"remove","path":"/b"},
        {"op":"replace","path":"/c","value":"x"},
        {"op":"move","path":"/d","from":"/e"},
        {"op":"copy","path":"/f","from":"/g"},
        {"op":"test","path":"/h","value":42},
    ]);
    let ops = parse_patch_ops(&v).unwrap();
    assert_eq!(ops.len(), 6);
}

#[test]
fn patch_ops_reject_non_array() {
    assert!(parse_patch_ops(&json!({"op":"add"})).is_err());
}

#[test]
fn patch_ops_reject_unknown_op() {
    let v = json!([{"op":"yeet","path":"/x"}]);
    assert!(parse_patch_ops(&v).is_err());
}

#[test]
fn patch_ops_reject_missing_path() {
    let v = json!([{"op":"add"}]);
    assert!(parse_patch_ops(&v).is_err());
}

#[test]
fn patch_ops_round_trip_to_admission() {
    let r = JsonPatchOpRecord {
        op: JsonPatchOp::Replace,
        path: "/spec/replicas".into(),
        value: Some(json!(3)),
        from: None,
    };
    let p = r.to_admission_patch();
    assert_eq!(p.op, "replace");
    assert_eq!(p.path, "/spec/replicas");
}

// ─────────────────────────────────────────────────────────────────────────────
// apply_config_to_patches — server-side-apply emulation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn apply_config_flat_object() {
    let v = json!({"x": 1, "y": "z"});
    let ops = apply_config_to_patches(&v);
    assert_eq!(ops.len(), 2);
    assert!(ops.iter().all(|o| o.op == JsonPatchOp::Add));
}

#[test]
fn apply_config_nested_object_emits_leaf_paths() {
    let v = json!({"a": {"b": {"c": 1}}});
    let ops = apply_config_to_patches(&v);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].path, "/a/b/c");
    assert_eq!(ops[0].value, Some(json!(1)));
}

#[test]
fn apply_config_escapes_pointer_chars() {
    let v = json!({"a/b": 1});
    let ops = apply_config_to_patches(&v);
    assert_eq!(ops[0].path, "/a~1b");
}

// ─────────────────────────────────────────────────────────────────────────────
// enforce_tenant_invariant
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invariant_blocks_tenant_id_path() {
    let ops = vec![JsonPatchOpRecord {
        op: JsonPatchOp::Replace,
        path: TENANT_ANNOTATION_PATH.into(),
        value: Some(json!("globex")),
        from: None,
    }];
    assert!(enforce_tenant_invariant(ops).is_err());
}

#[test]
fn invariant_allows_unrelated_paths() {
    let ops = vec![JsonPatchOpRecord {
        op: JsonPatchOp::Add,
        path: "/metadata/labels/app".into(),
        value: Some(json!("web")),
        from: None,
    }];
    assert!(enforce_tenant_invariant(ops).is_ok());
}

#[test]
fn invariant_blocks_remove_on_tenant_id() {
    let ops = vec![JsonPatchOpRecord {
        op: JsonPatchOp::Remove,
        path: TENANT_ANNOTATION_PATH.into(),
        value: None, from: None,
    }];
    assert!(enforce_tenant_invariant(ops).is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// MapStore tenant scope
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn map_store_tenant_isolation() {
    let s = MapStore::new();
    s.put_policy(map("p1", "acme"));
    s.put_policy(map("p2", "globex"));
    assert_eq!(s.list_policies("acme").len(), 1);
    assert_eq!(s.list_policies("globex").len(), 1);
}

#[test]
fn map_store_pairs_skip_orphans() {
    let s = MapStore::new();
    s.put_binding(binding("b", "acme", "missing"));
    assert!(s.pairs_for_tenant("acme").is_empty());
}

#[test]
fn map_store_pairs_match_within_tenant() {
    let s = MapStore::new();
    s.put_policy(map("p", "acme"));
    s.put_binding(binding("b", "acme", "p"));
    assert_eq!(s.pairs_for_tenant("acme").len(), 1);
}

#[test]
fn map_store_pairs_no_cross_tenant() {
    let s = MapStore::new();
    s.put_policy(map("p", "acme"));
    s.put_binding(binding("b", "globex", "p"));
    assert!(s.pairs_for_tenant("acme").is_empty());
    assert!(s.pairs_for_tenant("globex").is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// MapDispatcher.dispatch_one
// ─────────────────────────────────────────────────────────────────────────────

fn dispatcher_with(ev: FixedEvaluator) -> MapDispatcher {
    MapDispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()))
}

fn json_patch_mut(expr: &str) -> Mutation {
    Mutation {
        patch_type: PatchType::JSONPatch,
        json_patch: Some(JSONPatchExpression { expression: expr.into() }),
        apply_configuration: None,
    }
}

fn apply_cfg_mut(expr: &str) -> Mutation {
    Mutation {
        patch_type: PatchType::ApplyConfiguration,
        json_patch: None,
        apply_configuration: Some(ApplyConfigurationExpression { expression: expr.into() }),
    }
}

#[test]
fn dispatch_emits_json_patch_ops() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String(r#"[{"op":"add","path":"/metadata/labels/x","value":"y"}]"#.into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    match d.dispatch_one("acme", &p, &b, &r, &i) {
        MutationOutcome::Patches(ops) => {
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].path, "/metadata/labels/x");
        }
        other => panic!("expected Patches, got {other:?}"),
    }
}

#[test]
fn dispatch_apply_config_emits_leaf_patches() {
    let ev = FixedEvaluator::new().with("apply",
        CelValue::String(r#"{"metadata":{"labels":{"foo":"bar"}}}"#.into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.mutations.push(apply_cfg_mut("apply"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    match d.dispatch_one("acme", &p, &b, &r, &i) {
        MutationOutcome::Patches(ops) => {
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].path, "/metadata/labels/foo");
            assert_eq!(ops[0].value, Some(json!("bar")));
        }
        other => panic!("expected Patches, got {other:?}"),
    }
}

#[test]
fn dispatch_blocks_tenant_mutation() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String(format!(
            r#"[{{"op":"replace","path":"{}","value":"globex"}}]"#,
            TENANT_ANNOTATION_PATH).into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out, MutationOutcome::Error(_)),
        "tenant-id mutation must flip to Error, got {out:?}");
}

#[test]
fn dispatch_match_condition_false_skips() {
    let ev = FixedEvaluator::new().with("cond", CelValue::Bool(false));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.match_conditions.push(MatchCondition {
        name: "n".into(), expression: "cond".into(),
    });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    assert_eq!(d.dispatch_one("acme", &p, &b, &r, &i), MutationOutcome::Skipped);
}

#[test]
fn dispatch_failure_policy_fail_on_cel_error() {
    let ev = FixedEvaluator::new().with_err("expr",
        CelError::Compile("syntax".into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.failure_policy = FailurePolicyType::Fail;
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out, MutationOutcome::Error(_)));
}

#[test]
fn dispatch_failure_policy_ignore_silences_cel_error() {
    let ev = FixedEvaluator::new().with_err("expr",
        CelError::Compile("syntax".into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.failure_policy = FailurePolicyType::Ignore;
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    assert_eq!(d.dispatch_one("acme", &p, &b, &r, &i),
               MutationOutcome::SilencedError);
}

#[test]
fn dispatch_malformed_patch_json_fails() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String("not-json".into()));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    assert!(matches!(d.dispatch_one("acme", &p, &b, &r, &i),
                     MutationOutcome::Error(_)));
}

#[test]
fn dispatch_non_string_cel_return_fails() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Int(42));
    let d = dispatcher_with(ev);
    let mut p = map("p", "acme");
    p.spec.failure_policy = FailurePolicyType::Fail;
    p.spec.mutations.push(json_patch_mut("expr"));
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "ns", "acme");
    let empty = HashMap::new();
    let op = Operation::Create;
    let i = input(&op, "ns", &empty);
    assert!(matches!(d.dispatch_one("acme", &p, &b, &r, &i),
                     MutationOutcome::Error(_)));
}

// ─────────────────────────────────────────────────────────────────────────────
// MapPlugin (admission chain integration)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn plugin_allows_with_empty_store() {
    let store = Arc::new(MapStore::new());
    let d = MapDispatcher::new(
        Arc::new(FixedEvaluator::new()),
        Arc::new(InMemoryParamResolver::new()));
    let p = MapPlugin { store, dispatcher: d };
    let mut r = req(Operation::Create, "ns", "acme");
    let resp = p.admit(&mut r);
    assert!(resp.allowed);
    assert!(resp.patches.is_empty());
}

#[test]
fn plugin_emits_patches_from_policy() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String(r#"[{"op":"add","path":"/x","value":1}]"#.into()));
    let store = Arc::new(MapStore::new());
    let mut policy = map("p", "acme");
    policy.spec.mutations.push(json_patch_mut("expr"));
    store.put_policy(policy);
    store.put_binding(binding("b", "acme", "p"));
    let d = MapDispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()));
    let plug = MapPlugin { store, dispatcher: d };
    let mut r = req(Operation::Create, "ns", "acme");
    let resp = plug.admit(&mut r);
    assert!(resp.allowed);
    assert_eq!(resp.patches.len(), 1);
    assert_eq!(resp.patches[0].path, "/x");
}

#[test]
fn plugin_does_not_apply_other_tenant() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String(r#"[{"op":"add","path":"/x","value":1}]"#.into()));
    let store = Arc::new(MapStore::new());
    let mut policy = map("p", "globex");
    policy.spec.mutations.push(json_patch_mut("expr"));
    store.put_policy(policy);
    store.put_binding(binding("b", "globex", "p"));
    let d = MapDispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()));
    let plug = MapPlugin { store, dispatcher: d };
    let mut r = req(Operation::Create, "ns", "acme");
    let resp = plug.admit(&mut r);
    assert!(resp.allowed);
    assert!(resp.patches.is_empty(), "globex policy must not run on acme request");
}

#[test]
fn plugin_denies_on_tenant_violation_attempt() {
    let ev = FixedEvaluator::new().with("expr",
        CelValue::String(format!(
            r#"[{{"op":"replace","path":"{}","value":"globex"}}]"#,
            TENANT_ANNOTATION_PATH).into()));
    let store = Arc::new(MapStore::new());
    let mut policy = map("p", "acme");
    policy.spec.mutations.push(json_patch_mut("expr"));
    store.put_policy(policy);
    store.put_binding(binding("b", "acme", "p"));
    let d = MapDispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()));
    let plug = MapPlugin { store, dispatcher: d };
    let mut r = req(Operation::Create, "ns", "acme");
    let resp = plug.admit(&mut r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 500);
    assert!(resp.status_message.contains("tenant-id"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Type round-trips
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn map_type_roundtrip() {
    let p = map("p", "acme");
    let s = serde_json::to_string(&p).unwrap();
    let p2: MutatingAdmissionPolicy = serde_json::from_str(&s).unwrap();
    assert_eq!(p2.metadata.name, "p");
}

#[test]
fn mapb_type_roundtrip() {
    let b = binding("b", "acme", "p");
    let s = serde_json::to_string(&b).unwrap();
    let b2: MutatingAdmissionPolicyBinding = serde_json::from_str(&s).unwrap();
    assert_eq!(b2.spec.policy_name, "p");
}

#[test]
fn reinvocation_default_is_never() {
    assert_eq!(ReinvocationPolicyType::default(), ReinvocationPolicyType::Never);
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real CEL.
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[cfg(feature = "live-integration")]
fn cel_returns_typed_jsonpatch_array() {
    // pending: requires CEL with `JSONPatch.parse` library
}

#[test] #[cfg(feature = "live-integration")]
fn cel_apply_configuration_with_object_construction() {
    // pending: requires CEL Object/Map construction
}

#[test] #[cfg(feature = "live-integration")]
fn reinvocation_if_needed_runs_twice() {
    // pending: requires plugin-level reinvocation pass with object identity tracking
}
