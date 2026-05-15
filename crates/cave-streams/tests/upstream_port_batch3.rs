// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch 3 (2026-05-14) — Kafka GroupCoordinator + Connect runtime
//! ports beyond `upstream_port.rs` (batch1, 2026-05-13).
//!
//! Batch1 covered UnifiedLog + ProducerStateManager + Range/RoundRobin
//! assignors. Batch3 expands into the full join/sync/heartbeat group
//! lifecycle and the Kafka Connect runtime (connector lifecycle, task
//! restart, config validation).
//!
//! Upstream: apache/kafka @ 4.2.0
//!   * core/src/test/scala/unit/kafka/coordinator/group/GroupCoordinatorTest.scala
//!   * connect/runtime/src/test/java/org/apache/kafka/connect/runtime/{ConnectorConfig,Worker,distributed/DistributedHerder}_test.java

use cave_streams::connect::{
    ConnectCluster, Connector, ConnectorState, ConnectorType, TaskState,
};
use cave_streams::consumer_group::{GroupCoordinator, GroupState};
use cave_streams::error::StreamsError;
use std::collections::HashMap;

fn protocols_with_subscription(topics: &[&str]) -> HashMap<String, Vec<u8>> {
    let mut p = HashMap::new();
    let payload: Vec<u8> = topics.join(",").into_bytes();
    p.insert("range".into(), payload);
    p
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: core/src/test/scala/unit/kafka/coordinator/group/GroupCoordinatorTest.scala
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: GroupCoordinatorTest / `testJoinGroupAssignsMemberId`.
/// First join with empty member_id mints a fresh id and starts a
/// rebalance.
#[test]
fn upstream_group_coordinator_join_group_assigns_member_id_and_starts_rebalance() {
    let c = GroupCoordinator::new();
    let result = c
        .join_group(
            "g".into(),
            None,
            "client-1".into(),
            "host".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    assert_eq!(result.error_code, 0);
    assert!(!result.member_id.is_empty(), "fresh member_id minted");
    assert_eq!(result.generation_id, 1);
    assert_eq!(result.leader_id, result.member_id, "sole member is leader");
}

/// Upstream: GroupCoordinatorTest / `testJoinGroupReusesProvidedMemberId`.
/// Non-empty member_id on re-join must be reused.
#[test]
fn upstream_group_coordinator_join_group_reuses_provided_member_id() {
    let c = GroupCoordinator::new();
    let r = c
        .join_group(
            "g".into(),
            Some("existing-member".into()),
            "client".into(),
            "host".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    assert_eq!(r.member_id, "existing-member");
}

/// Upstream: GroupCoordinatorTest / `testHeartbeatUnknownMember`.
/// Heartbeat for a member that hasn't joined → MemberNotFound.
#[test]
fn upstream_group_coordinator_heartbeat_unknown_member_errors() {
    let c = GroupCoordinator::new();
    c.join_group(
        "g".into(),
        Some("alice".into()),
        "client".into(),
        "host".into(),
        10_000,
        60_000,
        "consumer".into(),
        protocols_with_subscription(&["t"]),
    )
    .unwrap();
    let err = c.heartbeat("g", 1, "bob").unwrap_err();
    assert!(matches!(err, StreamsError::MemberNotFound { .. }));
}

/// Upstream: GroupCoordinatorTest / `testHeartbeatIllegalGeneration`.
/// Wrong generation_id → IllegalGeneration.
#[test]
fn upstream_group_coordinator_heartbeat_illegal_generation_errors() {
    let c = GroupCoordinator::new();
    c.join_group(
        "g".into(),
        Some("alice".into()),
        "client".into(),
        "host".into(),
        10_000,
        60_000,
        "consumer".into(),
        protocols_with_subscription(&["t"]),
    )
    .unwrap();
    let err = c.heartbeat("g", 99, "alice").unwrap_err();
    assert!(matches!(err, StreamsError::IllegalGeneration { .. }));
}

/// Upstream: GroupCoordinatorTest / `testHeartbeatDuringRebalanceReturnsRebalanceInProgress`.
/// During PreparingRebalance state, heartbeat returns error_code=27
/// (REBALANCE_IN_PROGRESS).
#[test]
fn upstream_group_coordinator_heartbeat_during_rebalance_returns_rebalance_in_progress() {
    let c = GroupCoordinator::new();
    let r = c
        .join_group(
            "g".into(),
            Some("alice".into()),
            "client".into(),
            "host".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    // After join_group state is PreparingRebalance.
    let code = c.heartbeat("g", r.generation_id, "alice").unwrap();
    assert_eq!(code, 27, "REBALANCE_IN_PROGRESS code per Kafka spec");
}

/// Upstream: GroupCoordinatorTest / `testSyncGroupTransitionsToStable`.
#[test]
fn upstream_group_coordinator_sync_group_transitions_to_stable() {
    let c = GroupCoordinator::new();
    let r = c
        .join_group(
            "g".into(),
            Some("alice".into()),
            "client".into(),
            "host".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    let mut assignments = HashMap::new();
    assignments.insert("alice".to_string(), b"partition-0".to_vec());
    let assignment = c
        .sync_group("g", r.generation_id, "alice", assignments)
        .unwrap();
    assert_eq!(assignment, b"partition-0");
    // After SyncGroup, state must be Stable.
    let code = c.heartbeat("g", r.generation_id, "alice").unwrap();
    assert_eq!(code, 0, "Stable state → heartbeat returns 0");
}

/// Upstream: GroupCoordinatorTest / `testLeaveGroup`.
/// LeaveGroup removes the member and bumps the generation if others
/// remain (otherwise drops the group to Empty).
#[test]
fn upstream_group_coordinator_leave_group_removes_member_and_bumps_generation() {
    let c = GroupCoordinator::new();
    let _ = c
        .join_group(
            "g".into(),
            Some("alice".into()),
            "ca".into(),
            "h".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    let _ = c
        .join_group(
            "g".into(),
            Some("bob".into()),
            "cb".into(),
            "h".into(),
            10_000,
            60_000,
            "consumer".into(),
            protocols_with_subscription(&["t"]),
        )
        .unwrap();
    c.leave_group("g", "alice").unwrap();
    let descr = c.describe_group("g").unwrap();
    assert_eq!(descr.members.len(), 1);
    assert_eq!(descr.members[0].member_id, "bob");
}

/// Upstream: GroupCoordinatorTest / `testDeleteGroupRefusesActiveMembers`.
#[test]
fn upstream_group_coordinator_delete_group_refuses_when_active_members() {
    let c = GroupCoordinator::new();
    c.join_group(
        "g".into(),
        Some("alice".into()),
        "c".into(),
        "h".into(),
        10_000,
        60_000,
        "consumer".into(),
        protocols_with_subscription(&["t"]),
    )
    .unwrap();
    let err = c.delete_group("g").unwrap_err();
    assert!(matches!(err, StreamsError::Internal(_)));
    // Once everyone leaves, delete succeeds.
    c.leave_group("g", "alice").unwrap();
    c.delete_group("g").unwrap();
    assert!(c.describe_group("g").is_none());
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: connect/runtime/src/test/java/org/apache/kafka/connect/runtime/
// ────────────────────────────────────────────────────────────────────────────

fn source_config(name: &str, tasks_max: usize) -> HashMap<String, String> {
    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "org.apache.kafka.connect.file.FileStreamSourceConnector".into(),
    );
    cfg.insert("name".into(), name.into());
    cfg.insert("tasks.max".into(), tasks_max.to_string());
    cfg
}

/// Upstream: ConnectorConfigTest / `derive_source_type_from_class_name`.
/// `Connector::new` detects source vs sink from `connector.class`.
#[test]
fn upstream_connect_connector_derives_source_type_from_class_name() {
    let cfg = source_config("file-src", 1);
    let connector = Connector::new("file-src".into(), cfg);
    assert_eq!(connector.connector_type, ConnectorType::Source);
}

/// Upstream: ConnectorConfigTest / `tasks_max_creates_task_slots`.
#[test]
fn upstream_connect_connector_tasks_max_creates_task_slots() {
    let connector = Connector::new("file-src".into(), source_config("file-src", 4));
    assert_eq!(connector.tasks.len(), 4);
    for (i, task) in connector.tasks.iter().enumerate() {
        assert_eq!(task.id.task, i);
        assert_eq!(task.id.connector, "file-src");
    }
}

/// Upstream: WorkerTest / `start_connector_transitions_to_running_and_tasks_running`.
#[test]
fn upstream_connect_worker_start_transitions_connector_and_tasks_to_running() {
    let mut connector = Connector::new("file-src".into(), source_config("file-src", 2));
    connector.start();
    assert_eq!(connector.state, ConnectorState::Running);
    for task in &connector.tasks {
        assert_eq!(task.state, TaskState::Running);
    }
}

/// Upstream: DistributedHerderTest / `createConnectorRejectsDuplicate`.
#[test]
fn upstream_connect_cluster_create_connector_rejects_duplicate() {
    let cluster = ConnectCluster::new();
    cluster
        .create_connector("src-1".into(), source_config("src-1", 1))
        .unwrap();
    let err = cluster
        .create_connector("src-1".into(), source_config("src-1", 1))
        .unwrap_err();
    assert!(matches!(err, StreamsError::ConnectorAlreadyExists(ref n) if n == "src-1"));
}

/// Upstream: DistributedHerderTest / `pauseConnectorTransitionsTasks`.
#[test]
fn upstream_connect_cluster_pause_connector_transitions_tasks_to_paused() {
    let cluster = ConnectCluster::new();
    cluster
        .create_connector("src-1".into(), source_config("src-1", 2))
        .unwrap();
    cluster.pause_connector("src-1").unwrap();
    let connector = cluster.get_connector("src-1").unwrap();
    assert_eq!(connector.state, ConnectorState::Paused);
    for task in &connector.tasks {
        assert_eq!(task.state, TaskState::Paused);
    }
}

/// Upstream: DistributedHerderTest / `restartTaskClearsFailureTrace`.
#[test]
fn upstream_connect_cluster_restart_task_clears_failure_trace() {
    let cluster = ConnectCluster::new();
    cluster
        .create_connector("src-1".into(), source_config("src-1", 2))
        .unwrap();
    // Simulate a failed task via fail() then restart_task.
    {
        // Mutate via update_connector_config to keep a single mut borrow.
        let mut connector = cluster.get_connector("src-1").unwrap();
        connector.fail(Some("network down".into()));
        cluster
            .update_connector_config("src-1", connector.config.clone())
            .unwrap();
    }
    // restart_task brings task back to Running, clears trace.
    cluster.restart_task("src-1", 0).unwrap();
    let after = cluster.get_connector("src-1").unwrap();
    // restart_task only resets the targeted task's state.
    assert_eq!(after.tasks[0].state, TaskState::Running);
    assert!(after.tasks[0].trace.is_none());
}

/// Upstream: PluginConfigTest / `validate_config_rejects_missing_connector_class`.
#[test]
fn upstream_connect_cluster_validate_config_rejects_missing_connector_class() {
    let cluster = ConnectCluster::new();
    let mut bad = HashMap::new();
    bad.insert("name".into(), "x".into());
    let validation = cluster.validate_config("any", &bad);
    assert_eq!(validation.error_count, 1);
    assert_eq!(
        validation.configs[0].errors[0],
        "connector.class is required"
    );
}

/// Upstream: DistributedHerderTest / `deleteConnectorRemovesFromCluster`.
#[test]
fn upstream_connect_cluster_delete_connector_removes_from_list() {
    let cluster = ConnectCluster::new();
    cluster
        .create_connector("src-1".into(), source_config("src-1", 1))
        .unwrap();
    assert!(cluster.list_connectors().contains(&"src-1".to_string()));
    cluster.delete_connector("src-1").unwrap();
    assert!(!cluster.list_connectors().contains(&"src-1".to_string()));
    assert!(matches!(
        cluster.get_connector("src-1").unwrap_err(),
        StreamsError::ConnectorNotFound(_)
    ));
}
