// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// core/src/main/scala/kafka/coordinator/group/ConsumerGroupCoordinator.scala
//
//! [RED] KIP-848 — behavioural specification. Drives the
//! `cave_streams::kafka::kip848` module implementation.

use cave_streams::kafka::kip848::{
    ConsumerGroupCoordinator, ConsumerGroupHeartbeatRequest, ConsumerGroupRecord,
    HeartbeatErrorCode, MemberRecord, MemberSubscription, TargetAssignmentBuilder,
    TargetAssignmentRecord, TopicPartitions, UniformAssignor,
};

fn req(
    group_id: &str,
    member_id: &str,
    member_epoch: i32,
    subscribed: &[&str],
) -> ConsumerGroupHeartbeatRequest {
    ConsumerGroupHeartbeatRequest {
        group_id: group_id.into(),
        member_id: member_id.into(),
        member_epoch,
        instance_id: None,
        rack_id: None,
        rebalance_timeout_ms: 30_000,
        subscribed_topic_names: subscribed.iter().map(|s| (*s).into()).collect(),
        subscribed_topic_regex: None,
        server_assignor: None,
        topic_partitions: vec![],
        protocol_version: 1,
    }
}

#[test]
fn new_member_join_assigns_member_id_and_bumps_group_epoch() {
    let mut coord = ConsumerGroupCoordinator::new();
    let resp = coord
        .heartbeat(req("g1", "", 0, &["t0"]))
        .expect("first heartbeat ok");
    assert!(!resp.member_id.is_empty(), "fresh member_id assigned");
    assert_eq!(resp.member_epoch, 1, "new member starts at epoch 1");
    assert_eq!(resp.error_code, HeartbeatErrorCode::None as i16);
    let g = coord.describe_group("g1").unwrap();
    assert_eq!(g.group_epoch, 1, "first join bumps group_epoch to 1");
}

#[test]
fn second_heartbeat_with_matching_epoch_is_a_noop() {
    let mut coord = ConsumerGroupCoordinator::new();
    let r0 = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    coord.set_topic_partition_count("t", 4);
    let mid = r0.member_id.clone();
    // After the first response, the member confirms with its assigned epoch.
    let r1 = coord
        .heartbeat(req("g", &mid, r0.member_epoch, &["t"]))
        .unwrap();
    assert_eq!(r1.member_epoch, r0.member_epoch);
    assert_eq!(r1.error_code, HeartbeatErrorCode::None as i16);
}

#[test]
fn group_epoch_bumps_when_member_changes_subscription() {
    let mut coord = ConsumerGroupCoordinator::new();
    let r0 = coord.heartbeat(req("g", "", 0, &["t1"])).unwrap();
    let mid = r0.member_id.clone();
    let r1 = coord
        .heartbeat(req("g", &mid, r0.member_epoch, &["t1", "t2"]))
        .unwrap();
    assert!(
        r1.member_epoch > r0.member_epoch,
        "subscription change bumps the member epoch"
    );
    let g = coord.describe_group("g").unwrap();
    assert_eq!(g.group_epoch, 2);
}

#[test]
fn coordinator_assigns_partitions_via_uniform_assignor() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.set_topic_partition_count("t", 6);
    let a = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    let b = coord
        .heartbeat(req("g", "", 0, &["t"]))
        .unwrap();
    // Confirm — second heartbeat returns the assignment.
    let a2 = coord
        .heartbeat(req("g", &a.member_id, a.member_epoch, &["t"]))
        .unwrap();
    let b2 = coord
        .heartbeat(req("g", &b.member_id, b.member_epoch, &["t"]))
        .unwrap();
    let parts_a: usize = a2.assignment.iter().map(|t| t.partitions.len()).sum();
    let parts_b: usize = b2.assignment.iter().map(|t| t.partitions.len()).sum();
    assert_eq!(parts_a + parts_b, 6);
    // Uniform assignor balances within 1.
    assert!((parts_a as i32 - parts_b as i32).abs() <= 1);
}

#[test]
fn member_leaves_with_negative_epoch_drops_assignment() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.set_topic_partition_count("t", 4);
    let r = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    let mid = r.member_id.clone();
    let _ = coord.heartbeat(req("g", &mid, r.member_epoch, &["t"])); // confirm
    let leave = coord.heartbeat(req("g", &mid, -1, &["t"])).unwrap();
    assert_eq!(leave.error_code, HeartbeatErrorCode::None as i16);
    let g = coord.describe_group("g").unwrap();
    assert!(g.members.iter().all(|m| m.member_id != mid));
}

#[test]
fn unknown_member_id_returns_unknown_member_id_error() {
    let mut coord = ConsumerGroupCoordinator::new();
    let r = coord.heartbeat(req("g", "phantom", 1, &["t"])).unwrap();
    assert_eq!(r.error_code, HeartbeatErrorCode::UnknownMemberId as i16);
}

#[test]
fn fenced_member_returns_fenced_member_epoch_error() {
    let mut coord = ConsumerGroupCoordinator::new();
    let r0 = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    let mid = r0.member_id.clone();
    let stale = coord.heartbeat(req("g", &mid, 999, &["t"])).unwrap();
    assert_eq!(stale.error_code, HeartbeatErrorCode::FencedMemberEpoch as i16);
}

#[test]
fn legacy_protocol_version_zero_is_rejected_with_unsupported_version() {
    let mut coord = ConsumerGroupCoordinator::new();
    let mut r = req("g", "", 0, &["t"]);
    r.protocol_version = 0;
    let resp = coord.heartbeat(r).unwrap();
    assert_eq!(
        resp.error_code,
        HeartbeatErrorCode::UnsupportedVersion as i16
    );
}

#[test]
fn three_members_six_partitions_assigns_two_each() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.set_topic_partition_count("t", 6);
    let mut ids = vec![];
    let mut epochs = vec![];
    for _ in 0..3 {
        let r = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
        ids.push(r.member_id.clone());
        epochs.push(r.member_epoch);
    }
    let mut counts = vec![];
    for i in 0..3 {
        let r = coord
            .heartbeat(req("g", &ids[i], epochs[i], &["t"]))
            .unwrap();
        counts.push(r.assignment.iter().map(|t| t.partitions.len()).sum::<usize>());
    }
    assert_eq!(counts.iter().sum::<usize>(), 6);
    for c in &counts {
        assert_eq!(*c, 2, "uniform assignor gives 2 each for 6/3");
    }
}

#[test]
fn instance_id_static_membership_reuses_member_id_on_reconnect() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.set_topic_partition_count("t", 4);
    let mut r = req("g", "", 0, &["t"]);
    r.instance_id = Some("inst-a".into());
    let r1 = coord.heartbeat(r.clone()).unwrap();
    // Same instance reconnects with empty member_id — coordinator must
    // recognise it and return the prior member_id rather than minting a new one.
    let r2 = coord.heartbeat(r.clone()).unwrap();
    assert_eq!(r1.member_id, r2.member_id);
}

#[test]
fn target_assignment_record_serde_round_trip() {
    let rec = TargetAssignmentRecord {
        group_id: "g".into(),
        member_id: "m1".into(),
        group_epoch: 7,
        assigned: vec![TopicPartitions {
            topic: "t".into(),
            partitions: vec![0, 1, 2],
        }],
    };
    let bytes = rec.encode();
    let back = TargetAssignmentRecord::decode(&bytes).expect("decode ok");
    assert_eq!(back, rec);
}

#[test]
fn member_record_serde_round_trip() {
    let rec = MemberRecord {
        group_id: "g".into(),
        member_id: "m".into(),
        instance_id: Some("inst".into()),
        member_epoch: 4,
        subscription: MemberSubscription {
            topic_names: vec!["a".into(), "b".into()],
            topic_regex: None,
        },
        rack_id: Some("rack-1".into()),
    };
    let bytes = rec.encode();
    let back = MemberRecord::decode(&bytes).expect("decode ok");
    assert_eq!(back, rec);
}

#[test]
fn consumer_group_record_serde_round_trip() {
    let rec = ConsumerGroupRecord {
        group_id: "g".into(),
        group_epoch: 3,
        topic_partition_metadata: vec![("t".into(), 4), ("u".into(), 2)],
    };
    let bytes = rec.encode();
    let back = ConsumerGroupRecord::decode(&bytes).expect("decode ok");
    assert_eq!(back, rec);
}

#[test]
fn uniform_assignor_no_members_returns_empty() {
    let assign = UniformAssignor.assign(&[], &[("t".into(), 4)]);
    assert!(assign.is_empty());
}

#[test]
fn uniform_assignor_round_robins_remainder() {
    let members = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let topics = vec![("t".into(), 7)];
    let plan = UniformAssignor.assign(&members, &topics);
    let mut sums: Vec<usize> = members
        .iter()
        .map(|m| plan.get(m).map(|tp| tp.iter().map(|t| t.partitions.len()).sum()).unwrap_or(0))
        .collect();
    sums.sort();
    // 7 / 3 ≈ 2 r 1 → [2, 2, 3].
    assert_eq!(sums, vec![2, 2, 3]);
}

#[test]
fn target_assignment_builder_records_per_member_assignment() {
    let mut b = TargetAssignmentBuilder::new("g", 5);
    b.add("m1", vec![("t".into(), vec![0, 1])]);
    b.add("m2", vec![("t".into(), vec![2, 3])]);
    let recs = b.build();
    assert_eq!(recs.len(), 2);
    let m1 = recs.iter().find(|r| r.member_id == "m1").unwrap();
    assert_eq!(m1.group_epoch, 5);
    assert_eq!(m1.assigned[0].partitions, vec![0, 1]);
}

#[test]
fn heartbeat_wire_serde_round_trip() {
    use cave_streams::kafka::kip848::ConsumerGroupHeartbeatResponse;
    let resp = ConsumerGroupHeartbeatResponse {
        error_code: 0,
        member_id: "m".into(),
        member_epoch: 4,
        heartbeat_interval_ms: 5000,
        assignment: vec![TopicPartitions {
            topic: "t".into(),
            partitions: vec![0, 1, 2],
        }],
    };
    let bytes = resp.encode();
    let back = ConsumerGroupHeartbeatResponse::decode(&bytes).expect("decode ok");
    assert_eq!(back, resp);
}

#[test]
fn coordinator_describe_unknown_group_returns_none() {
    let coord = ConsumerGroupCoordinator::new();
    assert!(coord.describe_group("nope").is_none());
}

#[test]
fn coordinator_lists_groups() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.heartbeat(req("a", "", 0, &["t"])).unwrap();
    coord.heartbeat(req("b", "", 0, &["t"])).unwrap();
    let mut names: Vec<String> = coord.list_groups().into_iter().collect();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn coordinator_persistence_emits_records_on_join() {
    let mut coord = ConsumerGroupCoordinator::new();
    coord.set_topic_partition_count("t", 2);
    let _r = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    let log = coord.drain_persistence_log();
    // Expect at least: ConsumerGroupRecord + MemberRecord + TargetAssignmentRecord.
    let kinds: Vec<&'static str> = log.iter().map(|e| e.kind()).collect();
    assert!(kinds.contains(&"consumer_group"));
    assert!(kinds.contains(&"member"));
    assert!(kinds.contains(&"target_assignment"));
}

#[test]
fn member_leave_emits_tombstone_records() {
    let mut coord = ConsumerGroupCoordinator::new();
    let r = coord.heartbeat(req("g", "", 0, &["t"])).unwrap();
    let mid = r.member_id.clone();
    coord.drain_persistence_log();
    coord.heartbeat(req("g", &mid, -1, &["t"])).unwrap();
    let log = coord.drain_persistence_log();
    // Tombstone = entry with `is_tombstone() == true`.
    assert!(log.iter().any(|e| e.is_tombstone()));
}

#[test]
fn rebalance_timeout_validation() {
    let mut coord = ConsumerGroupCoordinator::new();
    let mut r = req("g", "", 0, &["t"]);
    r.rebalance_timeout_ms = -5;
    let resp = coord.heartbeat(r).unwrap();
    assert_eq!(
        resp.error_code,
        HeartbeatErrorCode::InvalidRequest as i16
    );
}

#[test]
fn empty_group_id_returns_invalid_group_id() {
    let mut coord = ConsumerGroupCoordinator::new();
    let resp = coord.heartbeat(req("", "", 0, &["t"])).unwrap();
    assert_eq!(
        resp.error_code,
        HeartbeatErrorCode::InvalidGroupId as i16
    );
}
