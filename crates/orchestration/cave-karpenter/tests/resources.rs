// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/utils/resources from kubernetes-sigs/karpenter v1.12.1 (sha
// ed490e8). Covers Quantity SI/binary parsing + arithmetic, the ResourceList
// helpers (Merge/Subtract/MaxResources/Fits), and the sidecar-aware
// Ceiling/PodRequests algorithm whose expected values are lifted verbatim from
// pkg/utils/resources/suite_test.go.

use cave_karpenter::resources::{
    ceiling, fits, is_zero, max_resources, merge, subtract, Container, Pod, Quantity, ResourceList,
};

fn rl(pairs: &[(&str, &str)]) -> ResourceList {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), Quantity::parse(v)))
        .collect()
}
fn ctr(cpu: &str, mem: &str, sidecar: bool) -> Container {
    Container {
        requests: rl(&[("cpu", cpu), ("memory", mem)]),
        limits: rl(&[("cpu", cpu), ("memory", mem)]),
        restart_policy_always: sidecar,
    }
}
fn assert_q(list: &ResourceList, key: &str, expected: &str) {
    assert_eq!(
        list.get(key).cloned().unwrap_or_default().cmp_to(&Quantity::parse(expected)),
        std::cmp::Ordering::Equal,
        "resource {key}: got {:?}, want {expected}",
        list.get(key)
    );
}

#[test]
fn quantity_parses_si_and_binary() {
    assert_eq!(Quantity::parse("2").cmp_to(&Quantity::parse("2000m")), std::cmp::Ordering::Equal);
    assert_eq!(Quantity::parse("1Gi").cmp_to(&Quantity::parse("1024Mi")), std::cmp::Ordering::Equal);
    assert_eq!(Quantity::parse("1k").cmp_to(&Quantity::parse("1000")), std::cmp::Ordering::Equal);
    assert_eq!(Quantity::parse("100m").cmp_to(&Quantity::parse("0.1")), std::cmp::Ordering::Equal);
    assert!(Quantity::parse("2Gi").cmp_to(&Quantity::parse("1Gi")) == std::cmp::Ordering::Greater);
    assert!(is_zero(&Quantity::parse("0")));
}

#[test]
fn quantity_add_and_sub() {
    let a = Quantity::parse("1Gi");
    let b = Quantity::parse("2Gi");
    assert_eq!(a.add(&b).cmp_to(&Quantity::parse("3Gi")), std::cmp::Ordering::Equal);
    assert_eq!(b.sub(&a).cmp_to(&Quantity::parse("1Gi")), std::cmp::Ordering::Equal);
}

#[test]
fn merge_sums_resource_lists() {
    let m = merge(&[rl(&[("cpu", "2"), ("memory", "1Gi")]), rl(&[("cpu", "3"), ("memory", "2Gi")])]);
    assert_q(&m, "cpu", "5");
    assert_q(&m, "memory", "3Gi");
}

#[test]
fn subtract_resource_lists() {
    let s = subtract(&rl(&[("cpu", "10"), ("memory", "8Gi")]), &rl(&[("cpu", "3"), ("memory", "2Gi")]));
    assert_q(&s, "cpu", "7");
    assert_q(&s, "memory", "6Gi");
}

#[test]
fn max_resources_takes_componentwise_max() {
    let m = max_resources(&[rl(&[("cpu", "2"), ("memory", "4Gi")]), rl(&[("cpu", "5"), ("memory", "1Gi")])]);
    assert_q(&m, "cpu", "5");
    assert_q(&m, "memory", "4Gi");
}

#[test]
fn fits_checks_candidate_within_total() {
    assert!(fits(&rl(&[("cpu", "2")]), &rl(&[("cpu", "4")])));
    assert!(!fits(&rl(&[("cpu", "5")]), &rl(&[("cpu", "4")])));
    assert!(fits(&rl(&[("cpu", "4")]), &rl(&[("cpu", "4")])));
}

// ---- Ceiling / PodRequests: expected values verbatim from suite_test.go ----

#[test]
fn ceiling_containers_plus_sidecars() {
    let pod = Pod {
        containers: vec![ctr("2", "1Gi", false)],
        init_containers: vec![ctr("1", "2Gi", true)], // sidecar
        overhead: ResourceList::new(),
    };
    let c = ceiling(&pod);
    assert_q(&c.requests, "cpu", "3");
    assert_q(&c.requests, "memory", "3Gi");
    assert_q(&c.limits, "cpu", "3");
    assert_q(&c.limits, "memory", "3Gi");
}

#[test]
fn ceiling_containers_sidecars_init_and_overhead() {
    let pod = Pod {
        containers: vec![ctr("2", "1Gi", false)],
        init_containers: vec![
            ctr("4", "2Gi", false), // non-restartable init
            ctr("3", "3Gi", true),  // sidecar
        ],
        overhead: rl(&[("cpu", "5"), ("memory", "1Gi")]),
    };
    let c = ceiling(&pod);
    assert_q(&c.requests, "cpu", "10");
    assert_q(&c.requests, "memory", "5Gi");
}

#[test]
fn ceiling_init_after_sidecar_exceeding() {
    let pod = Pod {
        containers: vec![ctr("2", "1Gi", false)],
        init_containers: vec![
            ctr("4", "2Gi", true),   // sidecar
            ctr("10", "2Gi", false), // non-restartable init exceeds
        ],
        overhead: ResourceList::new(),
    };
    let c = ceiling(&pod);
    assert_q(&c.requests, "cpu", "14");
    assert_q(&c.requests, "memory", "4Gi");
}

#[test]
fn ceiling_init_after_sidecar_not_exceeding() {
    let pod = Pod {
        containers: vec![ctr("2", "2Gi", false)],
        init_containers: vec![
            ctr("4", "2Gi", true),  // sidecar
            ctr("1", "1Gi", false), // non-restartable init below
        ],
        overhead: ResourceList::new(),
    };
    let c = ceiling(&pod);
    assert_q(&c.requests, "cpu", "6");
    assert_q(&c.requests, "memory", "4Gi");
}
