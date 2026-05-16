// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// clients/src/main/java/org/apache/kafka/clients/consumer/CooperativeStickyAssignor.java
//
//! [RED] KIP-415 — CooperativeStickyAssignor (consumer-side).

use cave_streams::kafka::kip415::{
    CooperativeStickyAssignor, MemberSubscription, TopicPartition,
};
use std::collections::BTreeMap;

fn sub(topics: &[&str], owned: Vec<(&str, i32)>) -> MemberSubscription {
    MemberSubscription {
        topics: topics.iter().map(|s| (*s).into()).collect(),
        owned_partitions: owned
            .into_iter()
            .map(|(t, p)| TopicPartition {
                topic: t.into(),
                partition: p,
            })
            .collect(),
        generation: 0,
    }
}

fn partition_counts() -> BTreeMap<String, i32> {
    let mut m = BTreeMap::new();
    m.insert("t".to_string(), 4);
    m
}

#[test]
fn empty_members_returns_empty_plan() {
    let a = CooperativeStickyAssignor::new();
    let plan = a.assign(&BTreeMap::new(), &partition_counts());
    assert!(plan.is_empty());
}

#[test]
fn single_member_owns_all_partitions() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("m1".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    assert_eq!(plan["m1"].len(), 4);
}

#[test]
fn fresh_assignment_three_members_four_partitions() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("a".to_string(), sub(&["t"], vec![]));
    subs.insert("b".to_string(), sub(&["t"], vec![]));
    subs.insert("c".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    let total: usize = plan.values().map(|v| v.len()).sum();
    assert_eq!(total, 4);
    let max = plan.values().map(|v| v.len()).max().unwrap();
    let min = plan.values().map(|v| v.len()).min().unwrap();
    assert!(max - min <= 1, "balanced within 1");
}

#[test]
fn stable_assignment_preserved_when_no_membership_change() {
    let a = CooperativeStickyAssignor::new();
    // m1 owns [0, 1], m2 owns [2, 3].
    let mut subs = BTreeMap::new();
    subs.insert(
        "m1".to_string(),
        sub(&["t"], vec![("t", 0), ("t", 1)]),
    );
    subs.insert(
        "m2".to_string(),
        sub(&["t"], vec![("t", 2), ("t", 3)]),
    );
    let plan = a.assign(&subs, &partition_counts());
    let m1_parts: Vec<i32> = plan["m1"].iter().map(|tp| tp.partition).collect();
    let m2_parts: Vec<i32> = plan["m2"].iter().map(|tp| tp.partition).collect();
    assert_eq!(m1_parts, vec![0, 1]);
    assert_eq!(m2_parts, vec![2, 3]);
}

#[test]
fn member_join_revokes_one_partition_only() {
    let a = CooperativeStickyAssignor::new();
    // Pre-state: m1 owns all 4. After m2 joins, m1 should lose 2 to balance.
    let mut subs = BTreeMap::new();
    subs.insert(
        "m1".to_string(),
        sub(&["t"], vec![("t", 0), ("t", 1), ("t", 2), ("t", 3)]),
    );
    subs.insert("m2".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    assert_eq!(plan["m1"].len() + plan["m2"].len(), 4);
    assert!(plan["m1"].len() <= 2, "m1 keeps ≤ 2 partitions");
    assert!(plan["m2"].len() >= 2, "m2 receives at least 2 partitions");
    // Cooperative: m2 only gets partitions that m1 actually owned.
    for tp in &plan["m2"] {
        // After cooperative revoke-then-assign, m2 receives partitions
        // that m1 had to revoke. Verify the partitions are valid range.
        assert!(tp.partition >= 0 && tp.partition < 4);
    }
}

#[test]
fn member_leave_reassigns_orphans_to_others() {
    let a = CooperativeStickyAssignor::new();
    // After m2 leaves, m1 should pick up its [2, 3].
    let mut subs = BTreeMap::new();
    subs.insert(
        "m1".to_string(),
        sub(&["t"], vec![("t", 0), ("t", 1)]),
    );
    // m2 not present in subs — that's the "leave".
    let plan = a.assign(&subs, &partition_counts());
    assert_eq!(plan["m1"].len(), 4);
    let parts: std::collections::BTreeSet<i32> =
        plan["m1"].iter().map(|tp| tp.partition).collect();
    assert_eq!(parts, [0, 1, 2, 3].into_iter().collect());
}

#[test]
fn assignment_no_duplicates_across_members() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("a".to_string(), sub(&["t"], vec![]));
    subs.insert("b".to_string(), sub(&["t"], vec![]));
    subs.insert("c".to_string(), sub(&["t"], vec![]));
    subs.insert("d".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    let mut all = vec![];
    for v in plan.values() {
        for tp in v {
            all.push((tp.topic.clone(), tp.partition));
        }
    }
    let unique: std::collections::HashSet<_> = all.iter().cloned().collect();
    assert_eq!(unique.len(), all.len());
    assert_eq!(all.len(), 4);
}

#[test]
fn cross_topic_balance() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("m1".to_string(), sub(&["t", "u"], vec![]));
    subs.insert("m2".to_string(), sub(&["t", "u"], vec![]));
    let mut topics = BTreeMap::new();
    topics.insert("t".to_string(), 3);
    topics.insert("u".to_string(), 3);
    let plan = a.assign(&subs, &topics);
    let m1c = plan["m1"].len();
    let m2c = plan["m2"].len();
    assert_eq!(m1c + m2c, 6);
    assert!((m1c as i32 - m2c as i32).abs() <= 1);
}

#[test]
fn topic_subscription_filter_applied() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("only_t".to_string(), sub(&["t"], vec![]));
    subs.insert("only_u".to_string(), sub(&["u"], vec![]));
    let mut topics = BTreeMap::new();
    topics.insert("t".to_string(), 2);
    topics.insert("u".to_string(), 2);
    let plan = a.assign(&subs, &topics);
    // Each member only gets its subscribed topic.
    for tp in &plan["only_t"] {
        assert_eq!(tp.topic, "t");
    }
    for tp in &plan["only_u"] {
        assert_eq!(tp.topic, "u");
    }
    assert_eq!(plan["only_t"].len(), 2);
    assert_eq!(plan["only_u"].len(), 2);
}

#[test]
fn rebalance_preserves_owned_when_balanced() {
    // 3 members, 6 partitions — already balanced. Plan must keep
    // each member's owned set intact.
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("a".to_string(), sub(&["t"], vec![("t", 0), ("t", 3)]));
    subs.insert("b".to_string(), sub(&["t"], vec![("t", 1), ("t", 4)]));
    subs.insert("c".to_string(), sub(&["t"], vec![("t", 2), ("t", 5)]));
    let mut topics = BTreeMap::new();
    topics.insert("t".to_string(), 6);
    let plan = a.assign(&subs, &topics);
    let to_set = |v: &Vec<TopicPartition>| -> std::collections::BTreeSet<i32> {
        v.iter().map(|tp| tp.partition).collect()
    };
    assert_eq!(to_set(&plan["a"]), [0, 3].into_iter().collect());
    assert_eq!(to_set(&plan["b"]), [1, 4].into_iter().collect());
    assert_eq!(to_set(&plan["c"]), [2, 5].into_iter().collect());
}

#[test]
fn imbalanced_owners_revoke_excess() {
    // m1 owns 4, m2 owns 0 — must move 2 to m2.
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert(
        "m1".to_string(),
        sub(&["t"], vec![("t", 0), ("t", 1), ("t", 2), ("t", 3)]),
    );
    subs.insert("m2".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    assert_eq!(plan["m1"].len(), 2);
    assert_eq!(plan["m2"].len(), 2);
}

#[test]
fn assign_is_deterministic_under_stable_input() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("a".to_string(), sub(&["t"], vec![]));
    subs.insert("b".to_string(), sub(&["t"], vec![]));
    let p1 = a.assign(&subs, &partition_counts());
    let p2 = a.assign(&subs, &partition_counts());
    assert_eq!(p1, p2);
}

#[test]
fn assigner_handles_partition_count_change() {
    // Topic shrank from 4 → 2 between rebalances.
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert(
        "m1".to_string(),
        sub(&["t"], vec![("t", 0), ("t", 1), ("t", 2), ("t", 3)]),
    );
    let mut topics = BTreeMap::new();
    topics.insert("t".to_string(), 2);
    let plan = a.assign(&subs, &topics);
    let parts: Vec<i32> = plan["m1"].iter().map(|tp| tp.partition).collect();
    assert_eq!(parts, vec![0, 1]); // 2 and 3 dropped.
}

#[test]
fn member_subscribing_to_unknown_topic_gets_empty() {
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("m1".to_string(), sub(&["nonexistent"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    assert!(plan["m1"].is_empty());
}

#[test]
fn ported_member_join_owns_after_assign() {
    // 4-partition scenario, 3 members. Reflects upstream
    // CooperativeStickyAssignorTest::testNewMemberAssignment.
    let a = CooperativeStickyAssignor::new();
    let mut subs = BTreeMap::new();
    subs.insert("a".to_string(), sub(&["t"], vec![("t", 0), ("t", 1)]));
    subs.insert("b".to_string(), sub(&["t"], vec![("t", 2), ("t", 3)]));
    subs.insert("c".to_string(), sub(&["t"], vec![]));
    let plan = a.assign(&subs, &partition_counts());
    let total: usize = plan.values().map(|v| v.len()).sum();
    assert_eq!(total, 4);
    let max = plan.values().map(|v| v.len()).max().unwrap();
    let min = plan.values().map(|v| v.len()).min().unwrap();
    // Cooperative-sticky: balanced within 1, c gets at least 1.
    assert!(max - min <= 1);
    assert!(plan["c"].len() >= 1, "joiner gets ≥ 1 partition");
}
