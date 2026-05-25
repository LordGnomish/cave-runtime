// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MirrorMaker pattern — cross-cluster topic replication.

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Mirror cluster config ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    pub alias: String,
    /// Bootstrap servers (comma-separated)
    pub bootstrap_servers: String,
    /// Security protocol
    pub security_protocol: String,
    pub extra: HashMap<String, String>,
}

// ── Replication policy ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationPolicy {
    /// Prepend source alias: `source.topic-name`
    DefaultReplicationPolicy,
    /// Keep exact topic name (identity)
    IdentityReplicationPolicy,
}

// ── Mirror flow ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MirrorFlowState {
    Started,
    Running,
    Paused,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorFlow {
    pub id: String,
    /// Source cluster alias
    pub source_cluster: String,
    /// Target cluster alias
    pub target_cluster: String,
    /// Topic patterns to replicate (regex)
    pub topics: Vec<String>,
    /// Topics to exclude (regex)
    pub topics_exclude: Vec<String>,
    /// Replication factor on target
    pub replication_factor: i16,
    /// Replication policy
    pub replication_policy: ReplicationPolicy,
    /// Whether to sync topic configurations
    pub sync_topic_configs: bool,
    /// Whether to sync consumer group offsets
    pub sync_group_offsets: bool,
    /// Offset sync interval (ms)
    pub offset_sync_interval_ms: i64,
    pub state: MirrorFlowState,
    /// Lag per topic partition (topic -> partition -> lag in messages)
    pub replication_lag: HashMap<String, HashMap<i32, i64>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MirrorFlow {
    pub fn new(source_cluster: String, target_cluster: String, topics: Vec<String>) -> Self {
        Self {
            id: format!("{source_cluster}->{target_cluster}"),
            source_cluster,
            target_cluster,
            topics,
            topics_exclude: vec![],
            replication_factor: 1,
            replication_policy: ReplicationPolicy::DefaultReplicationPolicy,
            sync_topic_configs: true,
            sync_group_offsets: false,
            offset_sync_interval_ms: 60_000,
            state: MirrorFlowState::Stopped,
            replication_lag: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Get the mirrored topic name on the target cluster.
    pub fn mirror_topic(&self, source_topic: &str) -> String {
        match self.replication_policy {
            ReplicationPolicy::IdentityReplicationPolicy => source_topic.to_string(),
            ReplicationPolicy::DefaultReplicationPolicy => {
                format!("{}.{source_topic}", self.source_cluster)
            }
        }
    }

    /// Check if a topic matches any of the include patterns.
    pub fn matches_topic(&self, topic: &str) -> bool {
        // Simple glob-like: ".*" matches all, exact match, or prefix.*
        let matches_include = self.topics.iter().any(|pattern| {
            if pattern == ".*" || pattern == "*" {
                return true;
            }
            if let Some(prefix) = pattern.strip_suffix(".*") {
                return topic.starts_with(prefix);
            }
            topic == pattern
        });
        let excluded = self.topics_exclude.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix(".*") {
                return topic.starts_with(prefix);
            }
            topic == pattern
        });
        matches_include && !excluded
    }

    pub fn start(&mut self) {
        self.state = MirrorFlowState::Running;
        self.updated_at = Utc::now();
    }

    pub fn pause(&mut self) {
        if self.state == MirrorFlowState::Running {
            self.state = MirrorFlowState::Paused;
            self.updated_at = Utc::now();
        }
    }

    pub fn resume(&mut self) {
        if self.state == MirrorFlowState::Paused {
            self.state = MirrorFlowState::Running;
            self.updated_at = Utc::now();
        }
    }

    pub fn stop(&mut self) {
        self.state = MirrorFlowState::Stopped;
        self.updated_at = Utc::now();
    }

    pub fn update_lag(&mut self, topic: &str, partition: i32, lag: i64) {
        self.replication_lag
            .entry(topic.to_string())
            .or_default()
            .insert(partition, lag);
    }

    pub fn total_lag(&self) -> i64 {
        self.replication_lag.values().flat_map(|p| p.values()).sum()
    }
}

// ── MirrorMaker manager ───────────────────────────────────────────────────────

pub struct MirrorMaker {
    clusters: DashMap<String, ClusterConfig>,
    flows: DashMap<String, MirrorFlow>,
}

impl MirrorMaker {
    pub fn new() -> Self {
        Self {
            clusters: DashMap::new(),
            flows: DashMap::new(),
        }
    }

    // ── Cluster management ────────────────────────────────────────────────────

    pub fn register_cluster(&self, config: ClusterConfig) {
        self.clusters.insert(config.alias.clone(), config);
    }

    pub fn list_clusters(&self) -> Vec<ClusterConfig> {
        self.clusters.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get_cluster(&self, alias: &str) -> StreamsResult<ClusterConfig> {
        self.clusters
            .get(alias)
            .map(|c| c.clone())
            .ok_or_else(|| StreamsError::Internal(format!("cluster not found: {alias}")))
    }

    // ── Flow management ───────────────────────────────────────────────────────

    pub fn create_flow(
        &self,
        source_cluster: String,
        target_cluster: String,
        topics: Vec<String>,
    ) -> StreamsResult<MirrorFlow> {
        // Validate clusters exist
        if !self.clusters.contains_key(&source_cluster) {
            return Err(StreamsError::Internal(format!(
                "source cluster not registered: {source_cluster}"
            )));
        }
        if !self.clusters.contains_key(&target_cluster) {
            return Err(StreamsError::Internal(format!(
                "target cluster not registered: {target_cluster}"
            )));
        }
        let flow = MirrorFlow::new(source_cluster, target_cluster, topics);
        let id = flow.id.clone();
        self.flows.insert(id, flow.clone());
        Ok(flow)
    }

    pub fn get_flow(&self, id: &str) -> StreamsResult<MirrorFlow> {
        self.flows
            .get(id)
            .map(|f| f.clone())
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {id}")))
    }

    pub fn list_flows(&self) -> Vec<MirrorFlow> {
        self.flows.iter().map(|e| e.value().clone()).collect()
    }

    pub fn start_flow(&self, id: &str) -> StreamsResult<()> {
        self.flows
            .get_mut(id)
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {id}")))?
            .start();
        Ok(())
    }

    pub fn pause_flow(&self, id: &str) -> StreamsResult<()> {
        self.flows
            .get_mut(id)
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {id}")))?
            .pause();
        Ok(())
    }

    pub fn stop_flow(&self, id: &str) -> StreamsResult<()> {
        self.flows
            .get_mut(id)
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {id}")))?
            .stop();
        Ok(())
    }

    pub fn delete_flow(&self, id: &str) -> StreamsResult<()> {
        self.flows
            .remove(id)
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {id}")))?;
        Ok(())
    }

    pub fn update_lag(
        &self,
        flow_id: &str,
        topic: &str,
        partition: i32,
        lag: i64,
    ) -> StreamsResult<()> {
        self.flows
            .get_mut(flow_id)
            .ok_or_else(|| StreamsError::Internal(format!("mirror flow not found: {flow_id}")))?
            .update_lag(topic, partition, lag);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm() -> MirrorMaker {
        let m = MirrorMaker::new();
        m.register_cluster(ClusterConfig {
            alias: "dc1".into(),
            bootstrap_servers: "dc1-kafka:9092".into(),
            security_protocol: "PLAINTEXT".into(),
            extra: HashMap::new(),
        });
        m.register_cluster(ClusterConfig {
            alias: "dc2".into(),
            bootstrap_servers: "dc2-kafka:9092".into(),
            security_protocol: "PLAINTEXT".into(),
            extra: HashMap::new(),
        });
        m
    }

    #[test]
    fn create_and_start_flow() {
        let m = mm();
        let flow = m
            .create_flow("dc1".into(), "dc2".into(), vec!["orders.*".into()])
            .unwrap();
        let id = flow.id;
        m.start_flow(&id).unwrap();
        assert_eq!(m.get_flow(&id).unwrap().state, MirrorFlowState::Running);
    }

    #[test]
    fn topic_matching() {
        let flow = MirrorFlow::new("src".into(), "dst".into(), vec!["orders.*".into()]);
        assert!(flow.matches_topic("orders.created"));
        assert!(flow.matches_topic("orders.updated"));
        assert!(!flow.matches_topic("payments.created"));
    }

    #[test]
    fn mirror_topic_naming() {
        let flow = MirrorFlow::new("primary".into(), "backup".into(), vec![".*".into()]);
        assert_eq!(flow.mirror_topic("orders"), "primary.orders");

        let mut identity_flow = flow;
        identity_flow.replication_policy = ReplicationPolicy::IdentityReplicationPolicy;
        assert_eq!(identity_flow.mirror_topic("orders"), "orders");
    }

    #[test]
    fn lag_tracking() {
        let m = mm();
        let flow = m
            .create_flow("dc1".into(), "dc2".into(), vec![".*".into()])
            .unwrap();
        m.update_lag(&flow.id, "orders", 0, 100).unwrap();
        m.update_lag(&flow.id, "orders", 1, 200).unwrap();
        assert_eq!(m.get_flow(&flow.id).unwrap().total_lag(), 300);
    }
}
