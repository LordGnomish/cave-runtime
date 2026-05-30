// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 5 (continuation ray #3): port of
// pkg/apis/v1/nodeclaim_validation.go from kubernetes-sigs/karpenter v1.12.1
// (sha ed490e8). Pure validation logic — no controller-runtime client, no
// cloud dependency. The k8s.io/apimachinery validation helpers it leans on
// (IsQualifiedName / IsValidLabelValue) are ported alongside.

use cave_karpenter::scheduling::requirement::{NodeSelectorRequirement, Operator};
use cave_karpenter::scheduling::taints::{Effect, Taint};
use cave_karpenter::validation::{
    is_qualified_name, is_valid_label_value, validate_requirement, validate_taints,
    SUPPORTED_EVICTION_SIGNALS, SUPPORTED_NODE_SELECTOR_OPS, SUPPORTED_RESERVED_RESOURCES,
};

fn req(key: &str, op: Operator, values: &[&str], min: Option<i64>) -> NodeSelectorRequirement {
    NodeSelectorRequirement {
        key: key.to_string(),
        operator: op,
        values: values.iter().map(|s| s.to_string()).collect(),
        min_values: min,
    }
}

fn taint(key: &str, value: Option<&str>, effect: Effect) -> Taint {
    Taint {
        key: key.to_string(),
        value: value.map(|s| s.to_string()),
        effect,
    }
}

// ── Supported-set constants ──────────────────────────────────────────────────

#[test]
fn supported_node_selector_ops_carries_all_eight_operators() {
    for op in [
        "In",
        "NotIn",
        "Exists",
        "DoesNotExist",
        "Gt",
        "Lt",
        "Gte",
        "Lte",
    ] {
        assert!(
            SUPPORTED_NODE_SELECTOR_OPS.contains(&op),
            "missing supported op {op}"
        );
    }
    assert!(!SUPPORTED_NODE_SELECTOR_OPS.contains(&"Foo"));
}

#[test]
fn supported_reserved_resources_match_upstream() {
    assert!(SUPPORTED_RESERVED_RESOURCES.contains(&"cpu"));
    assert!(SUPPORTED_RESERVED_RESOURCES.contains(&"memory"));
    assert!(SUPPORTED_RESERVED_RESOURCES.contains(&"ephemeral-storage"));
    assert!(SUPPORTED_RESERVED_RESOURCES.contains(&"pid"));
    assert_eq!(SUPPORTED_RESERVED_RESOURCES.len(), 4);
}

#[test]
fn supported_eviction_signals_match_upstream_six() {
    for s in [
        "memory.available",
        "nodefs.available",
        "nodefs.inodesFree",
        "imagefs.available",
        "imagefs.inodesFree",
        "pid.available",
    ] {
        assert!(SUPPORTED_EVICTION_SIGNALS.contains(&s), "missing signal {s}");
    }
    assert_eq!(SUPPORTED_EVICTION_SIGNALS.len(), 6);
}

// ── k8s apimachinery validation helpers ──────────────────────────────────────

#[test]
fn is_qualified_name_accepts_plain_and_prefixed_names() {
    assert!(is_qualified_name("instance-type").is_empty());
    assert!(is_qualified_name("kubernetes.io/arch").is_empty());
    assert!(is_qualified_name("my.domain.com/Some_Name.1").is_empty());
}

#[test]
fn is_qualified_name_rejects_empty_and_bad_chars() {
    assert!(!is_qualified_name("").is_empty());
    // leading dash is not allowed (must start alphanumeric)
    assert!(!is_qualified_name("-bad").is_empty());
    // trailing dot not allowed (must end alphanumeric)
    assert!(!is_qualified_name("bad.").is_empty());
    // space is not a valid char
    assert!(!is_qualified_name("has space").is_empty());
    // two slashes → too many parts
    assert!(!is_qualified_name("a/b/c").is_empty());
}

#[test]
fn is_qualified_name_rejects_over_63_char_name() {
    let long = "a".repeat(64);
    assert!(!is_qualified_name(&long).is_empty());
    let ok = "a".repeat(63);
    assert!(is_qualified_name(&ok).is_empty());
}

#[test]
fn is_qualified_name_rejects_bad_prefix() {
    // uppercase in the DNS subdomain prefix is invalid
    assert!(!is_qualified_name("BadPrefix/name").is_empty());
    // empty prefix is invalid
    assert!(!is_qualified_name("/name").is_empty());
}

#[test]
fn is_valid_label_value_allows_empty_and_simple() {
    assert!(is_valid_label_value("").is_empty());
    assert!(is_valid_label_value("on-demand").is_empty());
    assert!(is_valid_label_value("amd64").is_empty());
}

#[test]
fn is_valid_label_value_rejects_bad() {
    assert!(!is_valid_label_value("-leadingdash").is_empty());
    assert!(!is_valid_label_value("has space").is_empty());
    assert!(!is_valid_label_value(&"a".repeat(64)).is_empty());
}

// ── validate_requirement ─────────────────────────────────────────────────────

#[test]
fn validate_requirement_in_with_values_ok() {
    let r = req("kubernetes.io/arch", Operator::In, &["amd64"], None);
    assert!(validate_requirement(&r).is_ok());
}

#[test]
fn validate_requirement_in_without_values_is_error() {
    let r = req("custom.io/key", Operator::In, &[], None);
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_restricted_label_is_error() {
    // hostname is a restricted label
    let r = req("kubernetes.io/hostname", Operator::In, &["node-a"], None);
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_gt_with_single_integer_ok() {
    let r = req("custom.io/cores", Operator::Gt, &["4"], None);
    assert!(validate_requirement(&r).is_ok());
}

#[test]
fn validate_requirement_gt_with_non_integer_is_error() {
    let r = req("custom.io/cores", Operator::Gt, &["four"], None);
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_gt_with_negative_is_error() {
    let r = req("custom.io/cores", Operator::Gt, &["-1"], None);
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_lt_with_multiple_values_is_error() {
    let r = req("custom.io/cores", Operator::Lt, &["1", "2"], None);
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_in_below_min_values_is_error() {
    let r = req("custom.io/key", Operator::In, &["a"], Some(2));
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_well_known_only_invalid_values_is_error() {
    // capacity-type is well-known; "bogus" is not a known value
    let r = req(
        "karpenter.sh/capacity-type",
        Operator::In,
        &["bogus"],
        None,
    );
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_well_known_mixed_values_ok() {
    // one valid value present → only-invalid-values check passes (invalid logged)
    let r = req(
        "karpenter.sh/capacity-type",
        Operator::In,
        &["spot", "bogus"],
        None,
    );
    assert!(validate_requirement(&r).is_ok());
}

#[test]
fn validate_requirement_well_known_below_min_valid_is_error() {
    // 1 valid value but min_values=2 → not enough valid values
    let r = req(
        "karpenter.sh/capacity-type",
        Operator::In,
        &["spot", "bogus"],
        Some(2),
    );
    assert!(validate_requirement(&r).is_err());
}

#[test]
fn validate_requirement_normalizes_beta_label() {
    // beta arch label normalizes to stable; In with a valid value is accepted
    let r = req("beta.kubernetes.io/arch", Operator::In, &["arm64"], None);
    assert!(validate_requirement(&r).is_ok());
}

// ── validate_taints ──────────────────────────────────────────────────────────

#[test]
fn validate_taints_clean_pair_ok() {
    let taints = vec![taint("dedicated", Some("gpu"), Effect::NoSchedule)];
    assert!(validate_taints(&taints, &[]).is_ok());
}

#[test]
fn validate_taints_empty_key_is_error() {
    let taints = vec![taint("", None, Effect::NoSchedule)];
    assert!(validate_taints(&taints, &[]).is_err());
}

#[test]
fn validate_taints_duplicate_key_effect_is_error() {
    let taints = vec![
        taint("k", Some("a"), Effect::NoSchedule),
        taint("k", Some("b"), Effect::NoSchedule),
    ];
    assert!(validate_taints(&taints, &[]).is_err());
}

#[test]
fn validate_taints_same_key_different_effect_ok() {
    let taints = vec![
        taint("k", Some("a"), Effect::NoSchedule),
        taint("k", Some("a"), Effect::NoExecute),
    ];
    assert!(validate_taints(&taints, &[]).is_ok());
}

#[test]
fn validate_taints_duplicate_across_taints_and_startup_is_error() {
    let taints = vec![taint("k", None, Effect::NoSchedule)];
    let startup = vec![taint("k", None, Effect::NoSchedule)];
    assert!(validate_taints(&taints, &startup).is_err());
}

#[test]
fn validate_taints_invalid_key_qualified_name_is_error() {
    let taints = vec![taint("bad key!", None, Effect::NoSchedule)];
    assert!(validate_taints(&taints, &[]).is_err());
}

#[test]
fn validation_error_aggregates_messages() {
    // In with no values AND a restricted label → at least 2 aggregated msgs
    let r = req("kubernetes.io/hostname", Operator::In, &[], None);
    let err = validate_requirement(&r).unwrap_err();
    assert!(
        err.messages().len() >= 2,
        "expected aggregated errors, got {:?}",
        err.messages()
    );
}