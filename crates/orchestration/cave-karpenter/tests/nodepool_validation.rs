// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 6 (continuation ray #3): port of
// pkg/apis/v1/nodepool_validation.go from kubernetes-sigs/karpenter v1.12.1
// (sha ed490e8). RuntimeValidate fan-out over labels + taints + requirements,
// plus the nodepool-key-reservation checks. Reuses the cycle-5 validation
// helpers; exercises the new cross-module `validate_requirements` aggregator.

use std::collections::BTreeMap;

use cave_karpenter::nodepool_validation::{
    runtime_validate, validate_labels, validate_requirements_node_pool_key_does_not_exist,
};
use cave_karpenter::scheduling::requirement::{NodeSelectorRequirement, Operator};
use cave_karpenter::scheduling::taints::{Effect, Taint};
use cave_karpenter::validation::validate_requirements;

fn req(key: &str, op: Operator, values: &[&str]) -> NodeSelectorRequirement {
    NodeSelectorRequirement {
        key: key.to_string(),
        operator: op,
        values: values.iter().map(|s| s.to_string()).collect(),
        min_values: None,
    }
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── validate_labels ──────────────────────────────────────────────────────────

#[test]
fn validate_labels_clean_map_ok() {
    let m = labels(&[("team", "platform"), ("env", "prod")]);
    assert!(validate_labels(&m).is_ok());
}

#[test]
fn validate_labels_nodepool_key_is_restricted() {
    let m = labels(&[("karpenter.sh/nodepool", "default")]);
    assert!(validate_labels(&m).is_err());
}

#[test]
fn validate_labels_bad_key_qualified_name_is_error() {
    let m = labels(&[("bad key!", "v")]);
    assert!(validate_labels(&m).is_err());
}

#[test]
fn validate_labels_bad_value_is_error() {
    let m = labels(&[("team", "has space")]);
    assert!(validate_labels(&m).is_err());
}

#[test]
fn validate_labels_restricted_domain_key_is_error() {
    // karpenter.sh domain is restricted (and not a well-known label)
    let m = labels(&[("karpenter.sh/custom", "v")]);
    assert!(validate_labels(&m).is_err());
}

// ── validate_requirements_node_pool_key_does_not_exist ───────────────────────

#[test]
fn requirements_without_nodepool_key_ok() {
    let r = vec![req("kubernetes.io/arch", Operator::In, &["amd64"])];
    assert!(validate_requirements_node_pool_key_does_not_exist(&r).is_ok());
}

#[test]
fn requirements_with_nodepool_key_is_error() {
    let r = vec![req("karpenter.sh/nodepool", Operator::In, &["default"])];
    assert!(validate_requirements_node_pool_key_does_not_exist(&r).is_err());
}

// ── validate_requirements (nodeclaim aggregator) ─────────────────────────────

#[test]
fn validate_requirements_all_clean_ok() {
    let r = vec![
        req("kubernetes.io/arch", Operator::In, &["amd64"]),
        req("custom.io/zone", Operator::In, &["a"]),
    ];
    assert!(validate_requirements(&r).is_ok());
}

#[test]
fn validate_requirements_one_bad_is_error() {
    let r = vec![
        req("kubernetes.io/arch", Operator::In, &["amd64"]),
        req("custom.io/key", Operator::In, &[]), // In with no values
    ];
    assert!(validate_requirements(&r).is_err());
}

// ── runtime_validate ─────────────────────────────────────────────────────────

#[test]
fn runtime_validate_clean_nodepool_ok() {
    let l = labels(&[("team", "platform")]);
    let taints = vec![Taint {
        key: "dedicated".to_string(),
        value: Some("gpu".to_string()),
        effect: Effect::NoSchedule,
    }];
    let reqs = vec![req("kubernetes.io/arch", Operator::In, &["amd64"])];
    assert!(runtime_validate(&l, &taints, &[], &reqs).is_ok());
}

#[test]
fn runtime_validate_surfaces_label_and_requirement_errors() {
    // restricted label + nodepool-key requirement + In-with-no-values
    let l = labels(&[("karpenter.sh/nodepool", "x")]);
    let reqs = vec![req("karpenter.sh/nodepool", Operator::In, &[])];
    let err = runtime_validate(&l, &[], &[], &reqs).unwrap_err();
    assert!(
        err.messages().len() >= 2,
        "expected aggregated errors, got {:?}",
        err.messages()
    );
}
