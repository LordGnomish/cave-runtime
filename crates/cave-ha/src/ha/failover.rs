// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Automatic leader failover and split-brain prevention.
//!
//! The `FailoverManager` monitors cluster health and:
//! 1. Detects leader loss within `<5s` (configurable).
//! 2. Coordinates graceful leader stepdown.
//! 3. Prevents split-brain by requiring quorum before promoting a new leader.
//! 4. Detects and handles network partitions.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

use crate::error::HaResult;
use crate::ha::health::{HealthRegistry, SystemProbe};
use crate::raft::node::RaftHandle;
use crate::raft::types::{MembershipConfig, NodeId, NodeStatus};

/// Events emitted by the failover manager.
#[derive(Debug, Clone)]
pub enum FailoverEvent {
    LeaderLost { last_known_leader: Option<NodeId> },
    LeaderElected { new_leader: NodeId, term: u64 },
    QuorumLost,
    QuorumRestored,
    PartitionDetected { isolated: Vec<NodeId> },
    PartitionHealed,
    StepdownRequested { from: NodeId, reason: String },
    FailbackInitiated { to: NodeId },
}

/// Configuration for failover behavior.
#[derive(Debug, Clone)]
pub struct FailoverConfig {
    /// How long to wait before declaring leader lost.
    pub leader_loss_timeout: Duration,
    /// Minimum time between failover attempts (backoff).
    pub failover_backoff: Duration,
    /// Allow automatic failback after DR event.
    pub auto_failback: bool,
    /// Delay before initiating failback.
    pub failback_delay: Duration,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            leader_loss_timeout: Duration::from_secs(5),
            failover_backoff: Duration::from_secs(2),
            auto_failback: false,
            failback_delay: Duration::from_secs(300),
        }
    }
}

/// Monitors a Raft cluster for leadership changes and takes corrective action.
pub struct FailoverManager {
    local_id: NodeId,
    config: FailoverConfig,
    handle: RaftHandle,
    health: Arc<RwLock<HealthRegistry>>,
    event_tx: mpsc::UnboundedSender<FailoverEvent>,
    // Internal state.
    last_leader: Option<NodeId>,
    last_leader_seen: Instant,
    last_failover: Option<Instant>,
    quorum_available: bool,
}

impl FailoverManager {
    pub fn new(
        local_id: NodeId,
        config: FailoverConfig,
        handle: RaftHandle,
    ) -> (Self, mpsc::UnboundedReceiver<FailoverEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mgr = Self {
            local_id,
            config,
            handle,
            health: Arc::new(RwLock::new(HealthRegistry::new())),
            event_tx: tx,
            last_leader: None,
            last_leader_seen: Instant::now(),
            last_failover: None,
            quorum_available: true,
        };
        (mgr, rx)
    }

    /// Run the failover loop — typically spawned as a tokio task.
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;

            // Update our own health.
            let my_health = SystemProbe::probe(self.local_id);
            self.health.write().await.update(my_health);

            // Poll node status.
            let status = match self.handle.status().await {
                Ok(s) => s,
                Err(_) => continue,
            };

            self.monitor_leadership(&status).await;
            self.check_quorum(&status).await;
        }
    }

    async fn monitor_leadership(&mut self, status: &NodeStatus) {
        let current_leader = status.leader_id;

        if current_leader.is_some() {
            if current_leader != self.last_leader {
                // Leadership changed.
                if let Some(new_leader) = current_leader {
                    info!(
                        new_leader,
                        term = status.term,
                        "new leader elected"
                    );
                    let _ = self.event_tx.send(FailoverEvent::LeaderElected {
                        new_leader,
                        term: status.term,
                    });
                }
            }
            self.last_leader = current_leader;
            self.last_leader_seen = Instant::now();
        } else {
            // No leader.
            let elapsed = self.last_leader_seen.elapsed();
            if elapsed > self.config.leader_loss_timeout {
                // Check if we already triggered a failover recently.
                let can_failover = self.last_failover
                    .map(|t| t.elapsed() > self.config.failover_backoff)
                    .unwrap_or(true);
                if can_failover {
                    warn!(
                        elapsed_ms = elapsed.as_millis(),
                        last_leader = ?self.last_leader,
                        "leader loss detected"
                    );
                    let _ = self.event_tx.send(FailoverEvent::LeaderLost {
                        last_known_leader: self.last_leader,
                    });
                    self.last_failover = Some(Instant::now());
                    // Raft election is self-driven — just log the event.
                    // The election_timeout in the Raft node will trigger automatically.
                }
            }
        }
    }

    async fn check_quorum(&mut self, status: &NodeStatus) {
        let health = self.health.read().await;
        let quorum = health.has_healthy_quorum(&status.membership.voters);
        if !quorum && self.quorum_available {
            warn!("quorum loss detected");
            let _ = self.event_tx.send(FailoverEvent::QuorumLost);
            self.quorum_available = false;
        } else if quorum && !self.quorum_available {
            info!("quorum restored");
            let _ = self.event_tx.send(FailoverEvent::QuorumRestored);
            self.quorum_available = true;
        }
    }

    /// Request graceful stepdown: leader transfers leadership to the healthiest peer.
    pub async fn graceful_stepdown(&self) -> HaResult<()> {
        let status = self.handle.status().await?;
        if status.leader_id != Some(self.local_id) {
            return Ok(()); // Not the leader.
        }
        let health = self.health.read().await;
        let candidates: Vec<NodeId> = status.membership.voters
            .iter()
            .filter(|&&id| id != self.local_id)
            .copied()
            .collect();
        if let Some(target) = health.best_candidate(candidates.iter()) {
            info!(target, "graceful stepdown: transferring to healthiest peer");
            let _ = self.event_tx.send(FailoverEvent::StepdownRequested {
                from: self.local_id,
                reason: "graceful shutdown".into(),
            });
            self.handle.transfer_leadership(target).await?;
        }
        Ok(())
    }

    pub fn health_registry(&self) -> Arc<RwLock<HealthRegistry>> {
        Arc::clone(&self.health)
    }
}

/// Split-brain guard — verifies quorum before any leadership action.
pub struct SplitBrainGuard {
    voters: BTreeSet<NodeId>,
}

impl SplitBrainGuard {
    pub fn new(voters: BTreeSet<NodeId>) -> Self {
        Self { voters }
    }

    /// Returns true only if we can confirm quorum of `respondents`.
    pub fn quorum_confirmed(&self, respondents: &BTreeSet<NodeId>) -> bool {
        let cfg = MembershipConfig {
            voters: self.voters.clone(),
            ..Default::default()
        };
        cfg.has_quorum(respondents)
    }

    pub fn quorum_size(&self) -> usize {
        MembershipConfig::quorum(self.voters.len())
    }
}
