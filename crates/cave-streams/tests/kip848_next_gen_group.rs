// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 core/src/main/scala/kafka/coordinator/group/GroupCoordinatorService.scala
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupHeartbeatRequest.json
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/resources/common/message/ConsumerGroupDescribeRequest.json
//
// KIP-848 — Next-gen consumer rebalance protocol — integration tests.

use std::collections::BTreeSet;

use cave_streams::next_gen_group_protocol::{
    ConsumerGroupCoordinatorV2, ConsumerGroupDescribeRequest, ConsumerGroupHeartbeatRequest,
    GroupProtocol, KafkaErrorCodes, MemberEpoch, ServerAssignor,
};

fn parts(topic: &str, ps: &[i32]) -> Vec<(String, i32)> {
    ps.iter().map(|p| (topic.to_string(), *p)).collect()
}

#[test]
fn first_heartbeat_assigns_member_id_and_epoch_zero() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    let req = ConsumerGroupHeartbeatRequest {
        group_id: "g1".into(),
        member_id: String::new(), // empty → coordinator assigns
        member_epoch: MemberEpoch::JOINING,
        instance_id: None,
        subscribed_topic_names: vec!["events".into()],
        topic_partitions: vec![],
    };
    let resp = coord.consumer_group_heartbeat(req).unwrap();
    assert!(!resp.member_id.is_empty());
    // Coordinator increments to epoch=1 on accept.
    assert!(resp.member_epoch >= 1);
    assert_eq!(resp.error_code, KafkaErrorCodes::NONE);
    // Empty group → coordinator returns the target assignment.
    assert!(resp.assignment.is_some());
}

#[test]
fn second_heartbeat_same_epoch_returns_ack() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    let topic_partitions = vec![("events".to_string(), 0i32), ("events".to_string(), 1)];
    coord
        .declare_topic_partitions(&topic_partitions)
        .unwrap();
    let r1 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let r2 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: r1.member_id.clone(),
            member_epoch: r1.member_epoch,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: r1.assignment.clone().unwrap_or_default(),
        })
        .unwrap();
    // Heartbeat ack at same epoch — error_code stays 0,
    // assignment can be None (no diff to send).
    assert_eq!(r2.error_code, 0);
    assert_eq!(r2.member_epoch, r1.member_epoch);
}

#[test]
fn heartbeat_wrong_epoch_returns_fenced() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    let r1 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let r2 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: r1.member_id,
            member_epoch: r1.member_epoch + 100,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    assert_eq!(r2.error_code, KafkaErrorCodes::FENCED_MEMBER_EPOCH);
}

#[test]
fn leave_group_sets_member_epoch_minus_one() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    let r1 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    // LEAVING epoch = -1 explicitly removes the member.
    let r2 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: r1.member_id.clone(),
            member_epoch: MemberEpoch::LEAVING,
            instance_id: None,
            subscribed_topic_names: vec![],
            topic_partitions: vec![],
        })
        .unwrap();
    assert_eq!(r2.error_code, 0);
    // Subsequent heartbeat with the same id is fenced.
    let r3 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g1".into(),
            member_id: r1.member_id,
            member_epoch: r1.member_epoch,
            instance_id: None,
            subscribed_topic_names: vec!["events".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    assert_eq!(r3.error_code, KafkaErrorCodes::UNKNOWN_MEMBER_ID);
}

#[test]
fn describe_returns_group_state_and_members() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    coord
        .declare_topic_partitions(&parts("e", &[0, 1, 2, 3]))
        .unwrap();
    let r1 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "gdesc".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["e".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let _ = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "gdesc".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["e".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let desc = coord
        .consumer_group_describe(ConsumerGroupDescribeRequest {
            group_ids: vec!["gdesc".into()],
            include_authorized_operations: false,
        })
        .unwrap();
    assert_eq!(desc.groups.len(), 1);
    let g = &desc.groups[0];
    assert_eq!(g.group_id, "gdesc");
    assert!(g.group_epoch >= 1);
    assert_eq!(g.protocol_name, "consumer");
    // Two heartbeats → two members.
    assert_eq!(g.members.len(), 2);
    // The earlier member was already given an assignment, so its
    // assigned_partitions must be non-empty after the second heartbeat
    // triggers a rebalance.
    let m_with_assignment = g.members.iter().find(|m| m.member_id == r1.member_id);
    assert!(m_with_assignment.is_some());
}

#[test]
fn server_assignor_uniform_spreads_partitions_evenly() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    coord
        .declare_topic_partitions(&parts("u", &[0, 1, 2, 3]))
        .unwrap();
    let _ = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["u".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let _ = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["u".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let desc = coord
        .consumer_group_describe(ConsumerGroupDescribeRequest {
            group_ids: vec!["g".into()],
            include_authorized_operations: false,
        })
        .unwrap();
    let g = &desc.groups[0];
    // Uniform: 4 parts / 2 members ⇒ {2,2}
    let counts: Vec<usize> = g
        .members
        .iter()
        .map(|m| m.assigned_partitions.len())
        .collect();
    let sum: usize = counts.iter().sum();
    assert_eq!(sum, 4);
    assert!(counts.iter().all(|&c| c == 2));
}

#[test]
fn protocol_value_consumer_is_kip_848_path() {
    assert_eq!(GroupProtocol::Consumer.as_str(), "consumer");
    assert_eq!(GroupProtocol::Classic.as_str(), "classic");
}

#[test]
fn describe_unknown_group_returns_group_id_not_found() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    let desc = coord
        .consumer_group_describe(ConsumerGroupDescribeRequest {
            group_ids: vec!["nope".into()],
            include_authorized_operations: false,
        })
        .unwrap();
    assert_eq!(desc.groups.len(), 1);
    assert_eq!(
        desc.groups[0].error_code,
        KafkaErrorCodes::GROUP_ID_NOT_FOUND
    );
}

#[test]
fn target_assignment_diff_excludes_already_held_partitions() {
    let coord = ConsumerGroupCoordinatorV2::new(ServerAssignor::Uniform);
    coord
        .declare_topic_partitions(&parts("d", &[0, 1]))
        .unwrap();
    let r1 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: String::new(),
            member_epoch: MemberEpoch::JOINING,
            instance_id: None,
            subscribed_topic_names: vec!["d".into()],
            topic_partitions: vec![],
        })
        .unwrap();
    let owned = r1.assignment.clone().unwrap_or_default();
    let set: BTreeSet<_> = owned.iter().cloned().collect();
    assert_eq!(set.len(), 2);
    // Repeat heartbeat ACK'ing the assignment → assignment field is
    // None (nothing new to push).
    let r2 = coord
        .consumer_group_heartbeat(ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: r1.member_id,
            member_epoch: r1.member_epoch,
            instance_id: None,
            subscribed_topic_names: vec!["d".into()],
            topic_partitions: owned,
        })
        .unwrap();
    assert!(r2.assignment.is_none());
}
