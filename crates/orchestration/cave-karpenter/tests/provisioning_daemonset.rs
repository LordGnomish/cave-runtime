// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of DaemonSet-overhead reservation from the provisioning scheduler in
// pkg/controllers/provisioning/scheduling/scheduler.go (kubernetes-sigs/
// karpenter v1.12.1, sha ed490e8). Before packing application pods onto a
// simulated node, the scheduler subtracts the resource requests of the
// DaemonSet pods that will run on *every* node from that node's allocatable.
// A node too small to host the DaemonSet overhead is never opened.

use cave_karpenter::batcher::PodSpec;
use cave_karpenter::binpack::{binpack_with_daemonset, daemon_overhead, BinpackResult, InstanceType};

fn inst(name: &str, cpu: u32, mem: u32) -> InstanceType {
    InstanceType {
        name: name.into(),
        cpu_millis: cpu,
        memory_mib: mem,
        zone: "z".into(),
    }
}

// ---- daemon_overhead ---------------------------------------------------------

#[test]
fn daemon_overhead_sums_requests() {
    let ds = vec![
        PodSpec::with_resources("kube-proxy", 300, 200),
        PodSpec::with_resources("cni", 100, 50),
    ];
    assert_eq!(daemon_overhead(&ds), (400, 250));
}

#[test]
fn daemon_overhead_zero_for_no_daemonsets() {
    assert_eq!(daemon_overhead(&[]), (0, 0));
}

// ---- binpack_with_daemonset --------------------------------------------------

#[test]
fn no_daemonset_matches_plain_binpack() {
    let pods = vec![PodSpec::with_resources("app", 500, 256)];
    let res = binpack_with_daemonset(&pods, &[inst("a", 1000, 4096)], &[], &[]);
    match res {
        BinpackResult::Assigned { instances } => assert_eq!(instances.len(), 1),
        _ => panic!("expected assignment"),
    }
}

#[test]
fn daemonset_overhead_shrinks_available_capacity() {
    // Instance "a" has 1000m cpu; an 800m DaemonSet leaves only 200m, so a
    // 500m app pod no longer fits on it and falls through to NoFit.
    let pods = vec![PodSpec::with_resources("app", 500, 256)];
    let ds = vec![PodSpec::with_resources("agent", 800, 256)];
    let res = binpack_with_daemonset(&pods, &[inst("a", 1000, 4096)], &ds, &[]);
    assert!(matches!(res, BinpackResult::NoFit { .. }));
}

#[test]
fn daemonset_overhead_picks_a_larger_node() {
    // With a bigger instance "b" available, the 500m pod fits after the 800m
    // DaemonSet overhead (2000 - 800 = 1200 >= 500).
    let pods = vec![PodSpec::with_resources("app", 500, 256)];
    let ds = vec![PodSpec::with_resources("agent", 800, 256)];
    let res = binpack_with_daemonset(&pods, &[inst("a", 1000, 4096), inst("b", 2000, 4096)], &ds, &[]);
    match res {
        BinpackResult::Assigned { instances } => {
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0].instance.name, "b");
            // remaining reflects post-DaemonSet, post-app capacity: 1200 - 500.
            assert_eq!(instances[0].remaining_cpu_millis, 700);
        }
        _ => panic!("expected assignment on the larger node"),
    }
}

#[test]
fn daemonset_too_large_for_any_node_is_no_fit() {
    let pods = vec![PodSpec::with_resources("app", 100, 64)];
    let ds = vec![PodSpec::with_resources("fat-agent", 3000, 256)];
    let res = binpack_with_daemonset(&pods, &[inst("a", 1000, 4096), inst("b", 2000, 4096)], &ds, &[]);
    assert!(matches!(res, BinpackResult::NoFit { .. }));
}

#[test]
fn empty_pods_with_daemonset_opens_no_nodes() {
    let ds = vec![PodSpec::with_resources("agent", 800, 256)];
    let res = binpack_with_daemonset(&[], &[inst("a", 1000, 4096)], &ds, &[]);
    match res {
        BinpackResult::Assigned { instances } => assert!(instances.is_empty()),
        _ => panic!("no pods => no nodes opened"),
    }
}
