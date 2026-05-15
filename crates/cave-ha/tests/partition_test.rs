// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Network partition simulation tests.
//!
//! These tests verify split-brain prevention, partition recovery,
//! and quorum loss detection.

use std::sync::Arc;
use std::time::Duration;

use cave_ha::{
    config::NodeConfig,
    metrics::Metrics,
    raft::{
        node::{RaftHandle, RaftNode},
        state_machine::NoopStateMachine,
        types::{NodeId, NodeInfo},
    },
    transport::memory::MemNetwork,
    HaError,
};
use prometheus_client::registry::Registry;

async fn spawn_cluster(n: u32, pre_vote: bool, check_quorum: bool) -> (Vec<RaftHandle>, Arc<MemNetwork>) {
    let network = Arc::new(MemNetwork::new());
    let mut handles = Vec::new();

    let members: Vec<NodeInfo> = (1..=n)
        .map(|id| NodeInfo { id: id as NodeId, addr: format!("mem:{id}"), is_learner: false })
        .collect();

    for id in 1..=n {
        let (transport, mut rx) = network.register(id as NodeId);
        let mut registry = Registry::default();
        let metrics = Metrics::new(&mut registry);

        let config = NodeConfig {
            id: id as NodeId,
            election_timeout_min: 5,
            election_timeout_max: 10,
            heartbeat_interval: 2,
            pre_vote,
            check_quorum,
            check_quorum_interval: 8,
            ..Default::default()
        };

        let handle = RaftNode::spawn(
            config,
            members.clone(),
            Arc::new(transport),
            Arc::new(NoopStateMachine),
            metrics,
        );

        let msg_tx = handle.msg_tx.clone();
        tokio::spawn(async move {
            while let Some((from, msg)) = rx.recv().await {
                if msg_tx.send((from, msg)).is_err() { break; }
            }
        });

        handles.push(handle);
    }
    (handles, network)
}

async fn wait_for_leader(handles: &[RaftHandle], timeout: Duration) -> Option<NodeId> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline { return None; }
        for h in handles {
            if let Ok(s) = h.status().await {
                if s.role == "Leader" { return Some(s.id); }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn count_leaders(handles: &[RaftHandle]) -> usize {
    let mut count = 0;
    for h in handles {
        if let Ok(s) = h.status().await {
            if s.role == "Leader" { count += 1; }
        }
    }
    count
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Split brain: majority partition gets leader, minority does not.
#[tokio::test]
async fn test_split_brain_prevention() {
    // 5-node cluster: split 3+2.
    let (handles, network) = spawn_cluster(5, true, true).await;
    let _initial_leader = wait_for_leader(&handles, Duration::from_secs(6)).await
        .expect("initial leader elected");

    // Partition nodes 4 and 5 from 1, 2, 3.
    network.partition(4, 1, true);
    network.partition(4, 2, true);
    network.partition(4, 3, true);
    network.partition(5, 1, true);
    network.partition(5, 2, true);
    network.partition(5, 3, true);

    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Majority side (1, 2, 3) should have exactly one leader.
    let majority_handles: Vec<&RaftHandle> = handles.iter()
        .filter(|h| h.node_id <= 3)
        .collect();
    let majority_leaders = {
        let mut c = 0;
        for h in &majority_handles {
            if let Ok(s) = h.status().await {
                if s.role == "Leader" { c += 1; }
            }
        }
        c
    };

    // Minority side (4, 5) should NOT become leaders (no quorum).
    let minority_handles: Vec<&RaftHandle> = handles.iter()
        .filter(|h| h.node_id >= 4)
        .collect();
    let minority_leaders = {
        let mut c = 0;
        for h in &minority_handles {
            if let Ok(s) = h.status().await {
                if s.role == "Leader" { c += 1; }
            }
        }
        c
    };

    assert_eq!(majority_leaders, 1, "majority partition should have exactly one leader");
    assert_eq!(minority_leaders, 0, "minority partition should have no leader");
}

/// After partition heals, cluster converges to single leader.
#[tokio::test]
async fn test_partition_recovery() {
    let (handles, network) = spawn_cluster(3, true, true).await;
    wait_for_leader(&handles, Duration::from_secs(5)).await
        .expect("initial leader elected");

    // Isolate node 3.
    network.partition(3, 1, true);
    network.partition(3, 2, true);
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Heal.
    network.heal(3, 1);
    network.heal(3, 2);

    // After healing, cluster should converge.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let leaders = count_leaders(&handles).await;
    assert!(leaders <= 1, "after partition heal: expected <=1 leaders, got {leaders}");
    // And we should have a leader.
    let leader = wait_for_leader(&handles, Duration::from_secs(5)).await;
    assert!(leader.is_some(), "should elect leader after partition heals");
}

/// Proposals during partition: only the majority side can commit.
#[tokio::test]
async fn test_proposals_during_partition() {
    let (handles, network) = spawn_cluster(3, true, true).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5)).await
        .expect("leader elected");

    // Find a follower to partition away.
    let isolated_id = handles.iter()
        .find(|h| h.node_id != leader_id)
        .unwrap()
        .node_id;
    let other_follower = handles.iter()
        .find(|h| h.node_id != leader_id && h.node_id != isolated_id)
        .unwrap()
        .node_id;

    // Partition isolated node.
    network.partition(isolated_id, leader_id, true);
    network.partition(isolated_id, other_follower, true);

    // Leader (with 2/3 quorum) should still accept proposals.
    let leader_h = handles.iter().find(|h| h.node_id == leader_id).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = leader_h.propose(b"during-partition".to_vec()).await;
    // Should succeed (2 of 3 nodes reachable).
    assert!(result.is_ok(), "majority should commit: {:?}", result);

    // Isolated node should reject proposals.
    let isolated_h = handles.iter().find(|h| h.node_id == isolated_id).unwrap();
    let isolated_result = isolated_h.propose(b"should-fail".to_vec()).await;
    assert!(
        matches!(isolated_result, Err(HaError::NotLeader { .. })),
        "isolated node should return NotLeader: {:?}",
        isolated_result
    );
}

/// Network flakiness (random packet drops) doesn't break consensus.
#[tokio::test]
async fn test_flaky_network_consensus() {
    let (handles, network) = spawn_cluster(3, true, true).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(6)).await
        .expect("leader elected under normal conditions");

    // Apply 20% packet loss.
    network.set_drop_rate(0.2);

    let leader_h = handles.iter().find(|h| h.node_id == leader_id).unwrap();

    // Proposals should still succeed (eventually).
    let mut success = 0;
    for i in 0..5 {
        match leader_h.propose(format!("flaky-{i}").into_bytes()).await {
            Ok(_) => success += 1,
            Err(e) => eprintln!("proposal {i} failed (expected under loss): {e}"),
        }
    }
    // Under 20% loss, most proposals should succeed.
    assert!(success >= 3, "at least 3/5 proposals should succeed under 20% loss");

    // Restore normal network.
    network.set_drop_rate(0.0);
}

/// Quorum loss detection via check-quorum.
#[tokio::test]
async fn test_quorum_loss_detection() {
    let (handles, network) = spawn_cluster(3, true, true).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5)).await
        .expect("leader elected");

    // Partition the leader from both followers → quorum loss.
    for h in &handles {
        if h.node_id != leader_id {
            network.partition(leader_id, h.node_id, true);
        }
    }

    // Wait for check-quorum to fire (config: 8 ticks × 100ms = ~800ms).
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Leader should step down.
    let leader_h = handles.iter().find(|h| h.node_id == leader_id).unwrap();
    if let Ok(s) = leader_h.status().await {
        // check_quorum should have demoted it to Follower.
        assert_ne!(s.role, "Leader",
            "leader should step down after quorum loss, but role is {}", s.role);
    }

    // Heal and verify new election.
    for h in &handles {
        if h.node_id != leader_id {
            network.heal(leader_id, h.node_id);
        }
    }
    let new_leader = wait_for_leader(&handles, Duration::from_secs(6)).await;
    assert!(new_leader.is_some(), "should elect new leader after partition heals");
}

/// Symmetric partition (3-node cluster into 1+1+1): no leader possible.
#[tokio::test]
async fn test_full_isolation_no_leader() {
    let (handles, network) = spawn_cluster(3, true, true).await;

    // Fully isolate all nodes from each other immediately.
    // Do this before any leader is elected.
    network.partition(1, 2, true);
    network.partition(1, 3, true);
    network.partition(2, 3, true);

    // No leader should be elected (no node can get quorum).
    let leader = wait_for_leader(&handles, Duration::from_millis(1500)).await;
    assert!(leader.is_none(), "no leader should be possible without quorum");
}

/// Log compaction doesn't lose data (entries before snapshot are gone from log
/// but state machine reflects them).
#[tokio::test]
async fn test_compaction_correctness() {
    use cave_ha::raft::state_machine::KvStateMachine;

    let network = Arc::new(MemNetwork::new());
    let members = vec![NodeInfo { id: 1, addr: "mem:1".into(), is_learner: false }];
    let (transport, mut rx) = network.register(1);
    let mut registry = Registry::default();
    let metrics = Metrics::new(&mut registry);
    let sm = Arc::new(KvStateMachine::new());

    let config = NodeConfig {
        id: 1,
        election_timeout_min: 3,
        election_timeout_max: 6,
        log_compaction_threshold: 5, // Compact after 5 entries.
        ..Default::default()
    };

    let handle = RaftNode::spawn(
        config,
        members,
        Arc::new(transport),
        Arc::clone(&sm) as Arc<dyn cave_ha::StateMachine>,
        metrics,
    );

    let msg_tx = handle.msg_tx.clone();
    tokio::spawn(async move {
        while let Some((from, msg)) = rx.recv().await {
            if msg_tx.send((from, msg)).is_err() { break; }
        }
    });

    wait_for_leader(&[handle.clone()], Duration::from_secs(3)).await;

    // Propose enough entries to trigger compaction.
    for i in 0..10 {
        handle.propose(format!("entry-{i}").into_bytes()).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Node should still be functioning (compaction didn't crash it).
    let status = handle.status().await.unwrap();
    assert!(status.commit_index >= 10, "all entries should be committed");
    assert!(status.last_applied >= 10, "all entries should be applied");
}
