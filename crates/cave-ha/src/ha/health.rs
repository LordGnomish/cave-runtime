//! Health checking and leader election priority.
//!
//! Each node tracks the health of its peers and can advertise a priority
//! score that influences which node becomes leader after a failover.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::raft::types::NodeId;

/// Health score for a node (higher = healthier = more likely to lead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_id: NodeId,
    /// Composite score 0–100.
    pub score: u8,
    pub disk_ok: bool,
    pub memory_ok: bool,
    pub cpu_ok: bool,
    /// Replication lag behind current leader (entries).
    pub replication_lag: u64,
    /// Round-trip latency to leader (ms).
    pub leader_rtt_ms: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl NodeHealth {
    pub fn new(id: NodeId) -> Self {
        Self {
            node_id: id,
            score: 100,
            disk_ok: true,
            memory_ok: true,
            cpu_ok: true,
            replication_lag: 0,
            leader_rtt_ms: 0,
            last_updated: chrono::Utc::now(),
        }
    }

    /// Recompute composite score.
    pub fn compute_score(&mut self) {
        let mut score = 100u8;
        if !self.disk_ok { score = score.saturating_sub(50); }
        if !self.memory_ok { score = score.saturating_sub(20); }
        if !self.cpu_ok { score = score.saturating_sub(10); }
        if self.replication_lag > 1000 { score = score.saturating_sub(10); }
        if self.leader_rtt_ms > 100 { score = score.saturating_sub(5); }
        self.score = score;
    }

    pub fn is_healthy(&self) -> bool {
        self.score >= 50
    }
}

/// Cluster-wide health registry.
pub struct HealthRegistry {
    nodes: HashMap<NodeId, NodeHealth>,
    /// How long before a health record is considered stale.
    stale_after: Duration,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            stale_after: Duration::from_secs(30),
        }
    }

    pub fn update(&mut self, health: NodeHealth) {
        self.nodes.insert(health.node_id, health);
    }

    pub fn get(&self, id: NodeId) -> Option<&NodeHealth> {
        self.nodes.get(&id)
    }

    /// Select the healthiest node from a candidate set.
    /// Returns the node with the highest score; ties broken by node ID.
    pub fn best_candidate<'a>(&self, candidates: impl Iterator<Item = &'a NodeId>) -> Option<NodeId> {
        candidates
            .filter_map(|id| self.nodes.get(id))
            .filter(|h| h.is_healthy())
            .max_by_key(|h| (h.score, std::cmp::Reverse(h.node_id)))
            .map(|h| h.node_id)

    }

    /// Returns true if we have quorum of healthy nodes.
    pub fn has_healthy_quorum(&self, voters: &std::collections::BTreeSet<NodeId>) -> bool {
        let quorum = voters.len() / 2 + 1;
        let healthy = voters.iter()
            .filter(|id| self.nodes.get(id).map(|h| h.is_healthy()).unwrap_or(false))
            .count();
        healthy >= quorum
    }

    /// Remove stale records.
    pub fn prune_stale(&mut self) {
        let threshold = chrono::Utc::now() - chrono::Duration::from_std(self.stale_after).unwrap();
        self.nodes.retain(|_, h| h.last_updated > threshold);
    }
}

impl Default for HealthRegistry {
    fn default() -> Self { Self::new() }
}

/// System health probe — checks local disk, memory, CPU.
pub struct SystemProbe;

impl SystemProbe {
    pub fn probe(node_id: NodeId) -> NodeHealth {
        // In production, read /proc/meminfo, /proc/stat, check mount points.
        // Here we return a healthy baseline; actual probing can be injected.
        let mut health = NodeHealth::new(node_id);

        // Check disk space (simplified: always ok for now).
        health.disk_ok = Self::check_disk();
        health.memory_ok = Self::check_memory();
        health.cpu_ok = Self::check_cpu();
        health.compute_score();
        health
    }

    fn check_disk() -> bool {
        // Real impl: statvfs("/data") and check usage < threshold.
        true
    }

    fn check_memory() -> bool {
        // Real impl: parse /proc/meminfo for available > threshold.
        true
    }

    fn check_cpu() -> bool {
        // Real impl: check /proc/loadavg.
        true
    }
}

