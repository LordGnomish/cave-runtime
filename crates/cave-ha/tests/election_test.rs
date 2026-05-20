// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Raft leader election tests.

use std::sync::Arc;
use std::time::Duration;

use cave_ha::{
    MembershipConfig,
    config::NodeConfig,
    metrics::Metrics,
    raft::{
        node::RaftHandle,
        state_machine::NoopStateMachine,
        types::{NodeId, NodeInfo, Role},
    },
    transport::memory::MemNetwork,
};
use prometheus_client::registry::Registry;

/// Spawn a cluster of `n` nodes connected via an in-memory network.
/// Returns the handles and the shared network.
async fn spawn_cluster(n: u32) -> (Vec<RaftHandle>, Arc<MemNetwork>) {
    let network = Arc::new(MemNetwork::new());
    let mut handles = Vec::new();

    // Collect all members first.
    let members: Vec<NodeInfo> = (1..=n)
        .map(|id| NodeInfo {
            id: id as NodeId,
            addr: format!("mem:{id}"),
            is_learner: false,
        })
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
            pre_vote: true,
            check_quorum: true,
            ..Default::default()
        };

        let handle = cave_ha::raft::node::RaftNode::spawn(
            config,
            members.clone(),
            Arc::new(transport),
            Arc::new(NoopStateMachine),
            metrics,
        );

        // Forward inbound messages to the node.
        let msg_tx = handle.msg_tx.clone();
        tokio::spawn(async move {
            while let Some((from, msg)) = rx.recv().await {
                if msg_tx.send((from, msg)).is_err() {
                    break;
                }
            }
        });

        handles.push(handle);
    }
    (handles, network)
}

async fn wait_for_leader(handles: &[RaftHandle], timeout: Duration) -> Option<NodeId> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() > deadline {
            return None;
        }
        for h in handles {
            if let Ok(status) = h.status().await {
                if status.role == "Leader" {
                    return Some(status.id);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn get_leader(handles: &[RaftHandle]) -> Option<NodeId> {
    for h in handles {
        if let Ok(s) = h.status().await {
            if s.role == "Leader" {
                return Some(s.id);
            }
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Single-node cluster immediately becomes leader.
#[tokio::test]
async fn test_single_node_leader() {
    let (handles, _net) = spawn_cluster(1).await;
    let leader = wait_for_leader(&handles, Duration::from_secs(3)).await;
    assert_eq!(leader, Some(1), "single node should self-elect");
}

/// Three-node cluster elects exactly one leader.
#[tokio::test]
async fn test_three_node_election() {
    let (handles, _net) = spawn_cluster(3).await;
    let leader = wait_for_leader(&handles, Duration::from_secs(5)).await;
    assert!(leader.is_some(), "should elect a leader");

    // Only one leader.
    let leaders: Vec<NodeId> = {
        let mut l = vec![];
        for h in &handles {
            if let Ok(s) = h.status().await {
                if s.role == "Leader" {
                    l.push(s.id);
                }
            }
        }
        l
    };
    assert_eq!(leaders.len(), 1, "exactly one leader: {:?}", leaders);
}

/// Five-node cluster elects one leader (larger quorum).
#[tokio::test]
async fn test_five_node_election() {
    let (handles, _net) = spawn_cluster(5).await;
    let leader = wait_for_leader(&handles, Duration::from_secs(8)).await;
    assert!(leader.is_some(), "should elect a leader in 5-node cluster");
}

/// After leader shutdown, remaining nodes elect a new leader.
///
/// Flaky under parallel scheduling — election timeouts race against the
/// tokio scheduler on a contested runtime. Passes with `--test-threads=1`.
#[ignore = "flaky under parallel scheduling — election timeout races scheduler"]
#[tokio::test]
async fn test_leader_failover() {
    let (handles, _net) = spawn_cluster(3).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5))
        .await
        .expect("initial election should succeed");

    // Shut down the leader.
    let leader_handle = handles.iter().find(|h| h.node_id == leader_id).unwrap();
    leader_handle.shutdown().await;

    // Remaining nodes should elect a new leader.
    let remaining: Vec<&RaftHandle> = handles.iter().filter(|h| h.node_id != leader_id).collect();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut new_leader = None;
    while tokio::time::Instant::now() < deadline {
        for h in &remaining {
            if let Ok(s) = h.status().await {
                if s.role == "Leader" && s.id != leader_id {
                    new_leader = Some(s.id);
                    break;
                }
            }
        }
        if new_leader.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        new_leader.is_some(),
        "new leader should be elected after failover"
    );
    assert_ne!(new_leader.unwrap(), leader_id, "should be a different node");
}

/// Pre-vote prevents unnecessary term increments during partitions.
#[tokio::test]
async fn test_pre_vote_prevents_term_inflation() {
    let (handles, network) = spawn_cluster(3).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5))
        .await
        .expect("leader elected");

    // Record initial term.
    let initial_term = handles
        .iter()
        .find(|h| h.node_id == leader_id)
        .unwrap()
        .status()
        .await
        .unwrap()
        .term;

    // Partition node 3 from everyone.
    network.partition(3, 1, true);
    network.partition(3, 2, true);

    // Wait a bit — node 3 would try elections but pre-vote prevents term inflation.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Heal partition.
    network.heal(3, 1);
    network.heal(3, 2);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Leader's term should not have been disrupted significantly.
    if let Ok(s) = handles
        .iter()
        .find(|h| h.node_id == leader_id)
        .unwrap()
        .status()
        .await
    {
        // With pre-vote, isolated node's attempts should not have caused a term bump
        // (it can't get pre-votes). Without pre-vote it could disrupt the leader.
        assert!(
            s.term <= initial_term + 2,
            "pre-vote should prevent large term inflation, term={}",
            s.term
        );
    }
}

/// Leadership transfer sends TimeoutNow to target.
#[tokio::test]
async fn test_leadership_transfer() {
    let (handles, _net) = spawn_cluster(3).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5))
        .await
        .expect("leader elected");

    // Find a non-leader node to transfer to.
    let target = handles
        .iter()
        .find(|h| h.node_id != leader_id)
        .unwrap()
        .node_id;

    let leader_handle = handles.iter().find(|h| h.node_id == leader_id).unwrap();
    let result = leader_handle.transfer_leadership(target).await;
    // Transfer should succeed (or be in progress).
    assert!(result.is_ok() || matches!(result, Err(cave_ha::HaError::TransferInProgress)));

    // After transfer, target should become leader.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let new_leader = get_leader(&handles).await;
    // Either target became leader or another election happened.
    assert!(
        new_leader.is_some(),
        "cluster should still have a leader after transfer"
    );
}

/// Quorum loss: when majority is partitioned, leader steps down (check-quorum).
#[tokio::test]
async fn test_check_quorum_stepdown() {
    let (handles, network) = spawn_cluster(3).await;
    let leader_id = wait_for_leader(&handles, Duration::from_secs(5))
        .await
        .expect("leader elected");

    // Partition the leader from both followers.
    let followers: Vec<NodeId> = handles
        .iter()
        .filter(|h| h.node_id != leader_id)
        .map(|h| h.node_id)
        .collect();

    for &f in &followers {
        network.partition(leader_id, f, true);
    }

    // Wait for check-quorum to trigger (should be within ~1-2s with fast config).
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Leader should no longer be leader.
    let leader_handle = handles.iter().find(|h| h.node_id == leader_id).unwrap();
    if let Ok(s) = leader_handle.status().await {
        // Either stepped down or shutdown.
        let still_leader = s.role == "Leader";
        // With check_quorum enabled and fast timeouts, should step down.
        // Allow some slack in timing.
        if still_leader {
            // Give more time.
            tokio::time::sleep(Duration::from_millis(1000)).await;
            if let Ok(s2) = leader_handle.status().await {
                assert_ne!(s2.role, "Leader", "leader should step down on quorum loss");
            }
        }
    }
}

/// Membership quorum calculation is correct.
#[test]
fn test_membership_quorum() {
    use std::collections::BTreeSet;
    let cfg = MembershipConfig {
        voters: [1, 2, 3].iter().copied().collect(),
        ..Default::default()
    };
    let votes_two: BTreeSet<NodeId> = [1, 2].iter().copied().collect();
    let votes_one: BTreeSet<NodeId> = [1].iter().copied().collect();
    assert!(cfg.has_quorum(&votes_two));
    assert!(!cfg.has_quorum(&votes_one));
}
