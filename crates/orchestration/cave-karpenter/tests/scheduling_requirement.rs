// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/requirement_test.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). Behavioral parity for the complement-based
// Requirement set-algebra: constructors, Has, Operator, Len, Intersection,
// HasIntersection, inclusive integer bounds (Gt/Lt canonicalized to Gte/Lte),
// Any, String, and NodeSelectorRequirement conversion.

use cave_karpenter::scheduling::{NodeSelectorRequirement, Operator, Requirement};

fn req(op: Operator, vals: &[&str]) -> Requirement {
    Requirement::new("key", op, &vals.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

// ---- shared fixtures mirroring the upstream Describe block ----
fn exists() -> Requirement {
    req(Operator::Exists, &[])
}
fn does_not_exist() -> Requirement {
    req(Operator::DoesNotExist, &[])
}
fn in_a() -> Requirement {
    req(Operator::In, &["A"])
}
fn in_b() -> Requirement {
    req(Operator::In, &["B"])
}
fn in_ab() -> Requirement {
    req(Operator::In, &["A", "B"])
}
fn not_in_a() -> Requirement {
    req(Operator::NotIn, &["A"])
}
fn in1() -> Requirement {
    req(Operator::In, &["1"])
}
fn in9() -> Requirement {
    req(Operator::In, &["9"])
}
fn in19() -> Requirement {
    req(Operator::In, &["1", "9"])
}
fn not_in12() -> Requirement {
    req(Operator::NotIn, &["1", "2"])
}
fn gt1() -> Requirement {
    req(Operator::Gt, &["1"])
}
fn gt9() -> Requirement {
    req(Operator::Gt, &["9"])
}
fn lt1() -> Requirement {
    req(Operator::Lt, &["1"])
}
fn lt9() -> Requirement {
    req(Operator::Lt, &["9"])
}

#[test]
fn has_truth_table() {
    // (requirement, value, expected) rows lifted verbatim from upstream.
    let cases: &[(Requirement, &str, bool)] = &[
        (exists(), "A", true),
        (does_not_exist(), "A", false),
        (in_a(), "A", true),
        (in_b(), "A", false),
        (in_ab(), "A", true),
        (not_in_a(), "A", false),
        (not_in12(), "A", true),
        (gt1(), "A", false),
        (not_in_a(), "B", true),
        (in1(), "1", true),
        (not_in_a(), "1", true),
        (in19(), "1", true),
        (not_in12(), "1", false),
        (lt9(), "1", true),
        (gt1(), "2", true),
        (lt9(), "2", true),
        (gt1(), "9", true),
        (gt9(), "9", false),
        (in9(), "9", true),
        (lt1(), "9", false),
    ];
    for (r, v, expected) in cases {
        assert_eq!(r.has(v), *expected, "Has({:?}, {})", r, v);
    }
}

#[test]
fn operator_canonicalization() {
    assert_eq!(exists().operator(), Operator::Exists);
    assert_eq!(does_not_exist().operator(), Operator::DoesNotExist);
    assert_eq!(in_a().operator(), Operator::In);
    assert_eq!(not_in_a().operator(), Operator::NotIn);
    // Gt/Lt are stored as bounds and surface as Exists.
    assert_eq!(gt1().operator(), Operator::Exists);
    assert_eq!(lt9().operator(), Operator::Exists);
}

#[test]
fn len_matches_upstream() {
    assert_eq!(exists().len(), i64::MAX);
    assert_eq!(does_not_exist().len(), 0);
    assert_eq!(in_a().len(), 1);
    assert_eq!(in_ab().len(), 2);
    assert_eq!(not_in_a().len(), i64::MAX - 1);
    assert_eq!(not_in12().len(), i64::MAX - 2);
    assert_eq!(gt1().len(), i64::MAX);
}

#[test]
fn intersection_truth_table() {
    // existing.Intersection(new) == expected (compared via canonical form).
    let same = |a: &Requirement, b: &Requirement| a.canonical() == b.canonical();
    assert!(same(&exists().intersection(&exists()), &exists()));
    assert!(same(&exists().intersection(&in_a()), &in_a()));
    assert!(same(&exists().intersection(&not_in_a()), &not_in_a()));
    assert!(same(&does_not_exist().intersection(&exists()), &does_not_exist()));
    assert!(same(&does_not_exist().intersection(&in_a()), &does_not_exist()));
    assert!(same(&in_a().intersection(&exists()), &in_a()));
    assert!(same(&in_a().intersection(&in_a()), &in_a()));
    assert!(same(&in_a().intersection(&in_b()), &does_not_exist()));
    assert!(same(&in_a().intersection(&in_ab()), &in_a()));
    assert!(same(&in_a().intersection(&not_in_a()), &does_not_exist()));
    assert!(same(&in_a().intersection(&not_in12()), &in_a()));
    assert!(same(&in_b().intersection(&not_in_a()), &in_b()));
}

#[test]
fn has_intersection_matches_intersection() {
    assert!(exists().has_intersection(&in_a()));
    assert!(!in_a().has_intersection(&in_b()));
    assert!(in_a().has_intersection(&in_ab()));
    assert!(!in_a().has_intersection(&not_in_a()));
    assert!(in_a().has_intersection(&not_in12()));
    assert!(gt1().has_intersection(&lt9()));
    assert!(!gt9().has_intersection(&lt1()));
}

#[test]
fn inclusive_bounds() {
    let gte2 = Requirement::new("key", Operator::Gte, &["2".into()]);
    assert!(gte2.has("2"));
    assert!(gte2.has("3"));
    assert!(!gte2.has("1"));
    assert!(!gte2.has("0"));
    let lte8 = Requirement::new("key", Operator::Lte, &["8".into()]);
    assert!(lte8.has("8"));
    assert!(lte8.has("0"));
    assert!(!lte8.has("9"));
    // Gte 2 ≡ Gt 1, Lte 8 ≡ Lt 9.
    assert_eq!(gte2.has("2"), gt1().has("2"));
    assert_eq!(gte2.has("1"), gt1().has("1"));
    // Intersection of bounds.
    let both = gte2.intersection(&lte8);
    assert!(both.has("2") && both.has("5") && both.has("8"));
    assert!(!both.has("1") && !both.has("9"));
}

#[test]
fn bound_collapse_keeps_most_restrictive() {
    // Gt 5 (→Gte 6) AND Gte 5 → Gte 6.
    let gt5 = Requirement::new("key", Operator::Gt, &["5".into()]);
    let gte5 = Requirement::new("key", Operator::Gte, &["5".into()]);
    let i = gt5.intersection(&gte5);
    assert!(i.has("6") && !i.has("5"));
    let nsr = i.node_selector_requirement();
    assert_eq!(nsr.operator, Operator::Gte);
    assert_eq!(nsr.values, vec!["6".to_string()]);
    // Lt 5 (→Lte 4) AND Lte 5 → Lte 4.
    let lt5 = Requirement::new("key", Operator::Lt, &["5".into()]);
    let lte5 = Requirement::new("key", Operator::Lte, &["5".into()]);
    let j = lt5.intersection(&lte5);
    assert!(j.has("4") && !j.has("5"));
    assert_eq!(j.node_selector_requirement().values, vec!["4".to_string()]);
}

#[test]
fn gt_maxint_matches_nothing() {
    let gt_max = Requirement::new("key", Operator::Gt, &[i64::MAX.to_string()]);
    assert_eq!(gt_max.operator(), Operator::DoesNotExist);
    assert_eq!(gt_max.len(), 0);
}

#[test]
fn any_returns_representative_value() {
    assert!(!exists().any().is_empty());
    assert!(does_not_exist().any().is_empty());
    assert_eq!(in_a().any(), "A");
    assert_eq!(in9().any(), "9");
    assert!(in_ab().any() == "A" || in_ab().any() == "B");
    let na = not_in_a().any();
    assert!(!na.is_empty() && na != "A");
    assert!(gt1().any().parse::<i64>().unwrap() >= 1);
    let g9 = gt9().any().parse::<i64>().unwrap();
    assert!(g9 >= 9 && g9 < i64::MAX);
    assert_eq!(lt1().any(), "0");
}

#[test]
fn string_format() {
    assert_eq!(exists().to_string(), "key Exists");
    assert_eq!(does_not_exist().to_string(), "key DoesNotExist");
    assert_eq!(in_a().to_string(), "key In [A]");
    assert_eq!(in_ab().to_string(), "key In [A B]");
    assert_eq!(not_in_a().to_string(), "key NotIn [A]");
    assert_eq!(not_in12().to_string(), "key NotIn [1 2]");
    assert_eq!(gt1().to_string(), "key Exists >=2");
    assert_eq!(gt9().to_string(), "key Exists >=10");
    assert_eq!(lt1().to_string(), "key Exists <=0");
    assert_eq!(lt9().to_string(), "key Exists <=8");
    assert_eq!(gt1().intersection(&lt9()).to_string(), "key Exists >=2 <=8");
    assert_eq!(gt9().intersection(&lt1()).to_string(), "key DoesNotExist");
}

#[test]
fn node_selector_requirement_conversion() {
    let c = |r: Requirement| -> NodeSelectorRequirement { r.node_selector_requirement() };
    assert_eq!(c(exists()).operator, Operator::Exists);
    assert_eq!(c(does_not_exist()).operator, Operator::DoesNotExist);
    assert_eq!(c(in_a()).operator, Operator::In);
    assert_eq!(c(in_ab()).values, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(c(not_in_a()).operator, Operator::NotIn);
    let g = c(gt1());
    assert_eq!(g.operator, Operator::Gte);
    assert_eq!(g.values, vec!["2".to_string()]);
    let l = c(lt9());
    assert_eq!(l.operator, Operator::Lte);
    assert_eq!(l.values, vec!["8".to_string()]);
}

#[test]
fn min_values_canonicalized_via_max() {
    let a = Requirement::new_with_flexibility("key", Operator::In, Some(1), &["A".into(), "B".into()]);
    let b = Requirement::new_with_flexibility("key", Operator::In, Some(2), &["A".into(), "B".into()]);
    let i = a.intersection(&b);
    assert_eq!(i.min_values, Some(2));
}
