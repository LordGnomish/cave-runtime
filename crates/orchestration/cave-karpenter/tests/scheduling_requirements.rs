// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/requirements_test.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). Behavioral parity for the Requirements collection:
// label normalization, Add (intersect-on-collide), Get (undefined→Exists),
// Compatible/Intersects with the AllowUndefinedWellKnownLabels option,
// label-hint typo detection, NodeSelectorRequirements conversion, String.

use cave_karpenter::scheduling::{Operator, Requirement, Requirements};

const ZONE: &str = "topology.kubernetes.io/zone";
const BETA_ZONE: &str = "failure-domain.beta.kubernetes.io/zone";

fn r(key: &str, op: Operator, vals: &[&str]) -> Requirement {
    Requirement::new(key, op, &vals.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}
fn reqs(items: Vec<Requirement>) -> Requirements {
    Requirements::new(items)
}

#[test]
fn normalizes_aliased_labels() {
    let rs = reqs(vec![r(BETA_ZONE, Operator::In, &["test"])]);
    assert!(!rs.has(BETA_ZONE));
    assert!(rs.get(ZONE).has("test"));
}

#[test]
fn add_intersects_on_collision() {
    let mut rs = reqs(vec![r(ZONE, Operator::In, &["A"])]);
    rs.add(r(ZONE, Operator::In, &["B"]));
    // In[A] ∩ In[B] = empty → DoesNotExist
    assert_eq!(rs.get(ZONE).operator(), Operator::DoesNotExist);
}

#[test]
fn get_undefined_returns_exists() {
    let rs = reqs(vec![]);
    assert_eq!(rs.get("anything").operator(), Operator::Exists);
    assert!(!rs.has("anything"));
}

#[test]
fn compatible_with_allow_undefined_well_known() {
    let unconstrained = reqs(vec![]);
    let exists = reqs(vec![r(ZONE, Operator::Exists, &[])]);
    let does_not_exist = reqs(vec![r(ZONE, Operator::DoesNotExist, &[])]);
    let in_a = reqs(vec![r(ZONE, Operator::In, &["A"])]);
    let in_b = reqs(vec![r(ZONE, Operator::In, &["B"])]);
    let in_ab = reqs(vec![r(ZONE, Operator::In, &["A", "B"])]);
    let not_in_a = reqs(vec![r(ZONE, Operator::NotIn, &["A"])]);

    let ok = |a: &Requirements, b: &Requirements| a.compatible(b, true).is_ok();
    // unconstrained is compatible with everything (no shared keys)
    assert!(ok(&unconstrained, &exists));
    assert!(ok(&unconstrained, &does_not_exist));
    assert!(ok(&unconstrained, &in_a));
    // exists row
    assert!(ok(&exists, &exists));
    assert!(!ok(&exists, &does_not_exist));
    assert!(ok(&exists, &in_a));
    assert!(ok(&exists, &not_in_a));
    // doesNotExist row
    assert!(!ok(&does_not_exist, &exists));
    assert!(ok(&does_not_exist, &does_not_exist));
    assert!(!ok(&does_not_exist, &in_a));
    assert!(ok(&does_not_exist, &not_in_a));
    // inA row
    assert!(ok(&in_a, &exists));
    assert!(!ok(&in_a, &does_not_exist));
    assert!(ok(&in_a, &in_a));
    assert!(!ok(&in_a, &in_b));
    assert!(ok(&in_a, &in_ab));
    assert!(!ok(&in_a, &not_in_a));
    // inB row
    assert!(ok(&in_b, &not_in_a));
    assert!(ok(&in_b, &in_ab));
}

#[test]
fn strict_compatible_denies_undefined_well_known() {
    let unconstrained = reqs(vec![]);
    let exists = reqs(vec![r(ZONE, Operator::Exists, &[])]);
    let does_not_exist = reqs(vec![r(ZONE, Operator::DoesNotExist, &[])]);
    let in_a = reqs(vec![r(ZONE, Operator::In, &["A"])]);
    let not_in_a = reqs(vec![r(ZONE, Operator::NotIn, &["A"])]);

    // strict (allow_undefined = false): incoming Exists/In on an undefined
    // key is denied; NotIn / DoesNotExist are allowed.
    assert!(unconstrained.compatible(&exists, false).is_err());
    assert!(unconstrained.compatible(&in_a, false).is_err());
    assert!(unconstrained.compatible(&does_not_exist, false).is_ok());
    assert!(unconstrained.compatible(&not_in_a, false).is_ok());
    // exists row unchanged from loose
    assert!(exists.compatible(&exists, false).is_ok());
    assert!(exists.compatible(&does_not_exist, false).is_err());
}

#[test]
fn error_messages_detect_typos() {
    let unconstrained = reqs(vec![]);
    let case = |bad: &str| -> String {
        let req = reqs(vec![r(bad, Operator::Exists, &[])]);
        unconstrained.compatible(&req, true).unwrap_err()
    };
    assert_eq!(
        case("zone"),
        r#"label "zone" does not have known values (typo of "topology.kubernetes.io/zone"?)"#
    );
    assert_eq!(
        case("region"),
        r#"label "region" does not have known values (typo of "topology.kubernetes.io/region"?)"#
    );
    assert_eq!(
        case("nodepool"),
        r#"label "nodepool" does not have known values (typo of "karpenter.sh/nodepool"?)"#
    );
    assert_eq!(
        case("topology.kubernetesio/zone"),
        r#"label "topology.kubernetesio/zone" does not have known values (typo of "topology.kubernetes.io/zone"?)"#
    );
    assert_eq!(
        case("karpenter/nodepool"),
        r#"label "karpenter/nodepool" does not have known values (typo of "karpenter.sh/nodepool"?)"#
    );
    // unknown label → no hint
    let req = reqs(vec![r("deployment", Operator::Exists, &[])]);
    assert_eq!(
        unconstrained.compatible(&req, false).unwrap_err(),
        r#"label "deployment" does not have known values"#
    );
}

#[test]
fn node_selector_requirements_conversion() {
    let rs = reqs(vec![
        r("inAB", Operator::In, &["A", "B"]),
        r("gt1", Operator::Gt, &["1"]),
        r("lt9", Operator::Lt, &["9"]),
    ]);
    let out = rs.node_selector_requirements();
    assert_eq!(out.len(), 3);
    let find = |k: &str| out.iter().find(|n| n.key == k).unwrap().clone();
    assert_eq!(find("gt1").operator, Operator::Gte);
    assert_eq!(find("gt1").values, vec!["2".to_string()]);
    assert_eq!(find("lt9").operator, Operator::Lte);
    assert_eq!(find("lt9").values, vec!["8".to_string()]);
}

#[test]
fn node_selector_requirements_both_bounds() {
    let rs = reqs(vec![
        r("cpu", Operator::Gte, &["8"]),
        r("cpu", Operator::Lte, &["8"]),
    ]);
    let out = rs.node_selector_requirements();
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|n| n.operator == Operator::Gte && n.values == vec!["8".to_string()]));
    assert!(out.iter().any(|n| n.operator == Operator::Lte && n.values == vec!["8".to_string()]));
}

#[test]
fn string_prints_sorted() {
    let rs = reqs(vec![
        r("exists", Operator::Exists, &[]),
        r("doesNotExist", Operator::DoesNotExist, &[]),
        r("inA", Operator::In, &["A"]),
        r("inB", Operator::In, &["B"]),
        r("inAB", Operator::In, &["A", "B"]),
        r("notInA", Operator::NotIn, &["A"]),
        r("in1", Operator::In, &["1"]),
        r("in9", Operator::In, &["9"]),
        r("in19", Operator::In, &["1", "9"]),
        r("notIn12", Operator::NotIn, &["1", "2"]),
        r("greaterThan1", Operator::Gt, &["1"]),
        r("greaterThan9", Operator::Gt, &["9"]),
        r("lessThan1", Operator::Lt, &["1"]),
        r("lessThan9", Operator::Lt, &["9"]),
    ]);
    assert_eq!(
        rs.to_string(),
        "doesNotExist DoesNotExist, exists Exists, greaterThan1 Exists >=2, greaterThan9 Exists >=10, in1 In [1], in19 In [1 9], in9 In [9], inA In [A], inAB In [A B], inB In [B], lessThan1 Exists <=0, lessThan9 Exists <=8, notIn12 NotIn [1 2], notInA NotIn [A]"
    );
}

#[test]
fn has_min_values() {
    let plain = reqs(vec![r(ZONE, Operator::In, &["A"])]);
    assert!(!plain.has_min_values());
    let mut flex = Requirements::new(vec![]);
    flex.add(Requirement::new_with_flexibility(ZONE, Operator::In, Some(2), &["A".into(), "B".into()]));
    assert!(flex.has_min_values());
}
