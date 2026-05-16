// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/java/org/apache/kafka/clients/consumer/internals/CooperativeStickyAssignor.java
//
// KIP-415 — Cooperative Incremental Rebalance: integration tests
// exercising the IncrementalAssignor contract end-to-end.

use std::collections::{BTreeSet, HashMap};

use cave_streams::cooperative_assignor::{
    cooperative_sticky_plan, CooperativeStickyAssignor, IncrementalAssignor,
};
use cave_streams::incremental_rebalance::{is_already_balanced, Tp};

fn tp(topic: &str, p: i32) -> Tp {
    (topic.to_string(), p)
}

#[test]
fn integration_cooperative_balanced_assignment_is_stable() {
    let assignor = CooperativeStickyAssignor::new();
    let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
    prev.insert("c1".into(), [tp("orders", 0), tp("orders", 1)].into_iter().collect());
    prev.insert("c2".into(), [tp("orders", 2), tp("orders", 3)].into_iter().collect());
    let members = vec!["c1".to_string(), "c2".to_string()];
    let parts: BTreeSet<Tp> = (0..4).map(|p| tp("orders", p)).collect();
    let plan = assignor.cooperative_plan(&prev, &members, &parts);
    assert!(is_already_balanced(&plan));
}

#[test]
fn integration_rolling_bounce_revokes_only_minimum() {
    // 3 members, 6 partitions. Bring one offline (rolling-bounce
    // step): the surviving two should split 3-3. The single
    // member that leaves releases its 2 partitions; survivors
    // each pick up 1.
    let assignor = CooperativeStickyAssignor::new();
    let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
    prev.insert("c1".into(), [tp("t", 0), tp("t", 1)].into_iter().collect());
    prev.insert("c2".into(), [tp("t", 2), tp("t", 3)].into_iter().collect());
    prev.insert("c3".into(), [tp("t", 4), tp("t", 5)].into_iter().collect());
    let members = vec!["c1".to_string(), "c2".to_string()];
    let parts: BTreeSet<Tp> = (0..6).map(|p| tp("t", p)).collect();
    let plan = assignor.cooperative_plan(&prev, &members, &parts);
    // c1 and c2 retain their existing 2 partitions (sticky).
    let c1 = plan.phase1.iter().find(|m| m.member_id == "c1").unwrap();
    let c2 = plan.phase1.iter().find(|m| m.member_id == "c2").unwrap();
    assert_eq!(c1.retain.len(), 2);
    assert_eq!(c2.retain.len(), 2);
    // c3 must release all its partitions.
    let c3 = plan.phase1.iter().find(|m| m.member_id == "c3").unwrap();
    assert!(c3.retain.is_empty());
    assert_eq!(c3.to_release.len(), 2);
}

#[test]
fn integration_member_join_steals_balanced_share_via_freefn() {
    let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
    prev.insert("c1".into(), (0..6).map(|p| tp("t", p)).collect());
    let members = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
    let parts: BTreeSet<Tp> = (0..6).map(|p| tp("t", p)).collect();
    let plan = cooperative_sticky_plan(&prev, &members, &parts);
    // c1 must give up 4 partitions; new members each get 2.
    let c1 = plan.phase1.iter().find(|m| m.member_id == "c1").unwrap();
    assert_eq!(c1.to_release.len(), 4);
    assert_eq!(c1.retain.len(), 2);
    let c2_p2 = plan.phase2.iter().find(|m| m.member_id == "c2").unwrap();
    let c3_p2 = plan.phase2.iter().find(|m| m.member_id == "c3").unwrap();
    assert_eq!(c2_p2.assigned.len(), 2);
    assert_eq!(c3_p2.assigned.len(), 2);
}

#[test]
fn integration_assignor_name_matches_kafka_string() {
    assert_eq!(CooperativeStickyAssignor::new().name(), "cooperative-sticky");
}
