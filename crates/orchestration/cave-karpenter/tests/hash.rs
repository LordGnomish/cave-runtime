// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of the drift-detection hash in pkg/apis/v1/nodepool.go from
// kubernetes-sigs/karpenter v1.12.1 (sha ed490e8):
//
//   func (in *NodePool) Hash() string {
//       return fmt.Sprint(lo.Must(hashstructure.Hash([]interface{}{
//           in.Spec.Template.Spec,
//           in.Spec.Template.Labels,
//           in.Spec.Template.Annotations,
//       }, hashstructure.FormatV2, &hashstructure.HashOptions{
//           SlicesAsSets:    true,
//           IgnoreZeroValue: true,
//           ZeroNil:         true,
//       })))
//   }
//
// The byte-exact Go value is unreachable from Rust structs (Go reflection vs
// serde projection), so these tests pin the *behavioural contract* of
// mitchellh/hashstructure FormatV2: deterministic, set-semantics for slices
// (order-independent), zero/empty fields ignored, object field order
// irrelevant, decimal-string rendering, and sensitivity to real spec changes.

use cave_karpenter::hash::{hash_value, nodepool_hash, HashOptions};
use cave_karpenter::models::{NodePool, Requirement, RequirementOperator};
use serde_json::json;

// ---- primitive determinism ---------------------------------------------------

#[test]
fn primitive_hash_is_deterministic_and_distinct() {
    let o = HashOptions::format_v2();
    assert_eq!(hash_value(&json!("hello"), &o), hash_value(&json!("hello"), &o));
    assert_ne!(hash_value(&json!("hello"), &o), hash_value(&json!("world"), &o));
    assert_ne!(hash_value(&json!(1), &o), hash_value(&json!(2), &o));
    assert_ne!(hash_value(&json!(true), &o), hash_value(&json!(false), &o));
}

// ---- SlicesAsSets: order-independent -----------------------------------------

#[test]
fn slices_as_sets_are_order_independent() {
    let o = HashOptions::format_v2(); // slices_as_sets = true
    assert_eq!(
        hash_value(&json!(["x", "y", "z"]), &o),
        hash_value(&json!(["z", "y", "x"]), &o),
    );
}

#[test]
fn ordered_slices_respect_order() {
    let o = HashOptions {
        slices_as_sets: false,
        ignore_zero_value: true,
        zero_nil: true,
        ignore_keys: Default::default(),
    };
    assert_ne!(
        hash_value(&json!(["x", "y"]), &o),
        hash_value(&json!(["y", "x"]), &o),
    );
}

// ---- IgnoreZeroValue ---------------------------------------------------------

#[test]
fn ignore_zero_value_skips_empty_fields() {
    let o = HashOptions::format_v2();
    // An empty-string field is a zero value → must not affect the hash.
    assert_eq!(
        hash_value(&json!({"a": "v", "b": ""}), &o),
        hash_value(&json!({"a": "v"}), &o),
    );
    // null + empty array + 0 are all zero values too.
    assert_eq!(
        hash_value(&json!({"a": "v", "n": null, "arr": [], "z": 0}), &o),
        hash_value(&json!({"a": "v"}), &o),
    );
}

#[test]
fn without_ignore_zero_value_empty_fields_count() {
    let o = HashOptions {
        slices_as_sets: true,
        ignore_zero_value: false,
        zero_nil: true,
        ignore_keys: Default::default(),
    };
    assert_ne!(
        hash_value(&json!({"a": "v", "b": ""}), &o),
        hash_value(&json!({"a": "v"}), &o),
    );
}

// ---- object field order irrelevant -------------------------------------------

#[test]
fn object_field_order_is_irrelevant() {
    let o = HashOptions::format_v2();
    assert_eq!(
        hash_value(&json!({"a": 1, "b": 2}), &o),
        hash_value(&json!({"b": 2, "a": 1}), &o),
    );
}

// ---- ignore_keys -------------------------------------------------------------

#[test]
fn ignore_keys_drops_named_fields() {
    let mut ignore = std::collections::HashSet::new();
    ignore.insert("transient".to_string());
    let o = HashOptions {
        slices_as_sets: true,
        ignore_zero_value: true,
        zero_nil: true,
        ignore_keys: ignore,
    };
    assert_eq!(
        hash_value(&json!({"a": "v", "transient": "noise"}), &o),
        hash_value(&json!({"a": "v"}), &o),
    );
}

// ---- NodePool.Hash() ---------------------------------------------------------

#[test]
fn nodepool_hash_is_decimal_string() {
    let h = nodepool_hash(&NodePool::default());
    assert!(!h.is_empty());
    assert!(h.chars().all(|c| c.is_ascii_digit()), "hash must be decimal: {h}");
}

#[test]
fn nodepool_hash_is_deterministic() {
    let mut p = NodePool::default();
    p.name = "p".into();
    p.template.spec.requirements.push(Requirement {
        key: "node.kubernetes.io/instance-type".into(),
        operator: RequirementOperator::In,
        values: vec!["c5.large".into()],
        min_values: None,
    });
    assert_eq!(nodepool_hash(&p), nodepool_hash(&p));
}

#[test]
fn nodepool_hash_excludes_pool_name() {
    // Hash covers only template spec/labels/annotations, never pool metadata.
    let mut a = NodePool::default();
    a.name = "alpha".into();
    let mut b = NodePool::default();
    b.name = "beta".into();
    assert_eq!(nodepool_hash(&a), nodepool_hash(&b));
}

#[test]
fn nodepool_hash_changes_on_spec_change() {
    let mut p = NodePool::default();
    let before = nodepool_hash(&p);
    p.template.spec.requirements.push(Requirement {
        key: "k".into(),
        operator: RequirementOperator::Exists,
        values: vec![],
        min_values: None,
    });
    assert_ne!(before, nodepool_hash(&p));
}

#[test]
fn nodepool_hash_ignores_requirement_order() {
    let mk = |keys: &[&str]| {
        let mut p = NodePool::default();
        p.template.spec.requirements = keys
            .iter()
            .map(|k| Requirement {
                key: k.to_string(),
                operator: RequirementOperator::Exists,
                values: vec![],
                min_values: None,
            })
            .collect();
        nodepool_hash(&p)
    };
    assert_eq!(mk(&["a", "b", "c"]), mk(&["c", "a", "b"]));
}

#[test]
fn nodepool_hash_includes_labels() {
    let a = NodePool::default();
    let mut b = NodePool::default();
    b.template.labels.insert("team".into(), "core".into());
    assert_ne!(nodepool_hash(&a), nodepool_hash(&b));
}

#[test]
fn nodepool_hash_includes_annotations() {
    let a = NodePool::default();
    let mut b = NodePool::default();
    b.template
        .annotations
        .insert("example.com/owner".into(), "team-x".into());
    assert_ne!(nodepool_hash(&a), nodepool_hash(&b));
}
