// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of OrderByWeight from pkg/utils/nodepool/nodepool.go in
// kubernetes-sigs/karpenter v1.12.1 (sha ed490e8):
//
//   func OrderByWeight(nodePools []*v1.NodePool) {
//       sort.Slice(nodePools, func(a, b int) bool {
//           weightA := lo.FromPtr(nodePools[a].Spec.Weight)
//           weightB := lo.FromPtr(nodePools[b].Spec.Weight)
//           if weightA == weightB {
//               return nodePools[a].Name < nodePools[b].Name
//           }
//           return weightA > weightB
//       })
//   }
//
// The provisioner evaluates NodePools highest-weight-first; this wires that
// ordering into schedule_first_match so the highest-weight *matching* pool
// wins instead of input order.

use cave_karpenter::models::NodePool;
use cave_karpenter::nodepool_utils::{effective_weight, order_by_weight, ordered_by_weight};
use cave_karpenter::{schedule_first_match, ScheduleOutcome};

fn np(name: &str, weight: Option<i32>) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.into();
    p.weight = weight;
    p
}

// ---- effective_weight (lo.FromPtr) -------------------------------------------

#[test]
fn effective_weight_treats_none_as_zero() {
    assert_eq!(effective_weight(&np("a", None)), 0);
    assert_eq!(effective_weight(&np("a", Some(42))), 42);
    assert_eq!(effective_weight(&np("a", Some(-7))), -7);
}

// ---- OrderByWeight -----------------------------------------------------------

#[test]
fn order_by_weight_descending() {
    let mut pools = vec![np("a", Some(10)), np("b", Some(100)), np("c", Some(50))];
    order_by_weight(&mut pools);
    let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["b", "c", "a"]);
}

#[test]
fn order_by_weight_none_sinks_to_zero() {
    let mut pools = vec![np("a", None), np("b", Some(1)), np("c", Some(-5))];
    order_by_weight(&mut pools);
    let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();
    // 1 > 0 (None) > -5
    assert_eq!(names, vec!["b", "a", "c"]);
}

#[test]
fn order_by_weight_ties_break_by_name_ascending() {
    let mut pools = vec![np("zeta", Some(5)), np("alpha", Some(5)), np("mid", Some(5))];
    order_by_weight(&mut pools);
    let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "mid", "zeta"]);
}

#[test]
fn ordered_by_weight_does_not_mutate_input() {
    let pools = vec![np("a", Some(1)), np("b", Some(9))];
    let sorted = ordered_by_weight(&pools);
    assert_eq!(pools[0].name, "a", "input slice must be untouched");
    assert_eq!(sorted[0].name, "b");
    assert_eq!(sorted[1].name, "a");
}

// ---- scheduler integration: weight priority ----------------------------------

#[test]
fn scheduler_prefers_higher_weight_pool() {
    // Both pools are permissive (no requirements) so both match; the
    // higher-weight pool must win regardless of input order.
    let lo = np("low", Some(1));
    let hi = np("high", Some(100));
    match schedule_first_match(&[lo, hi], &[]) {
        ScheduleOutcome::Provisioned { pool, .. } => assert_eq!(pool, "high"),
        ScheduleOutcome::NoMatch { .. } => panic!("expected a match"),
    }
}

#[test]
fn scheduler_weight_tie_breaks_by_name() {
    let z = np("zzz", Some(5));
    let a = np("aaa", Some(5));
    match schedule_first_match(&[z, a], &[]) {
        ScheduleOutcome::Provisioned { pool, .. } => assert_eq!(pool, "aaa"),
        ScheduleOutcome::NoMatch { .. } => panic!("expected a match"),
    }
}
