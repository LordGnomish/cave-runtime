// SPDX-License-Identifier: AGPL-3.0-or-later
//! Admin API — topic, consumer group, and broker-level management operations.

use crate::compaction::{CompactionEngine, CompactionStats, RetentionStats};
use crate::consumer::GroupAdmin;
use crate::error::{StreamError, StreamResult};
use crate::models::{
    ConnectorConfig, ConsumerGroup, StorageTierConfig, TopicInfo,
};
use crate::storage::StreamStorage;
use crate::topic::{TopicConfigPatch, TopicManager};

/// Unified admin facade providing all management operations.
pub struct AdminClient<S: StreamStorage + Clone> {
    storage: S,
}

impl<S: StreamStorage + Clone> AdminClient<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    // ─── Topics ───────────────────────────────────────────────────────────────

    pub fn create_topic(
        &self,
        name: &str,
        partitions: u32,
        replication_factor: u16,
        config: Option<crate::models::TopicConfig>,
    ) -> StreamResult<TopicInfo> {
        TopicManager::new(self.storage.clone()).create(
            name,
            partitions,
            replication_factor,
            config,
        )
    }

    pub fn delete_topic(&self, name: &str) -> StreamResult<()> {
        TopicManager::new(self.storage.clone()).delete(name)
    }

    pub fn describe_topic(&self, name: &str) -> StreamResult<TopicInfo> {
        TopicManager::new(self.storage.clone()).describe(name)
    }

    pub fn list_topics(&self) -> StreamResult<Vec<TopicInfo>> {
        TopicManager::new(self.storage.clone()).list()
    }

    pub fn alter_topic_config(
        &self,
        name: &str,
        patch: TopicConfigPatch,
    ) -> StreamResult<TopicInfo> {
        TopicManager::new(self.storage.clone()).alter_config(name, patch)
    }

    pub fn add_partitions(&self, name: &str, new_total: u32) -> StreamResult<TopicInfo> {
        TopicManager::new(self.storage.clone()).add_partitions(name, new_total)
    }

    pub fn topic_watermarks(&self, name: &str) -> StreamResult<Vec<(u32, i64)>> {
        TopicManager::new(self.storage.clone()).watermarks(name)
    }

    // ─── Consumer groups ──────────────────────────────────────────────────────

    pub fn list_groups(&self) -> StreamResult<Vec<ConsumerGroup>> {
        GroupAdmin::new(self.storage.clone()).list()
    }

    pub fn describe_group(&self, group_id: &str) -> StreamResult<ConsumerGroup> {
        GroupAdmin::new(self.storage.clone()).describe(group_id)
    }

    pub fn delete_group(&self, group_id: &str) -> StreamResult<()> {
        GroupAdmin::new(self.storage.clone()).delete(group_id)
    }

    pub fn reset_offsets_earliest(&self, group_id: &str, topic: &str) -> StreamResult<()> {
        GroupAdmin::new(self.storage.clone()).reset_offsets_earliest(group_id, topic)
    }

    pub fn reset_offsets_latest(&self, group_id: &str, topic: &str) -> StreamResult<()> {
        GroupAdmin::new(self.storage.clone()).reset_offsets_latest(group_id, topic)
    }

    // ─── Compaction ───────────────────────────────────────────────────────────

    pub fn run_compaction(&self) -> StreamResult<CompactionStats> {
        CompactionEngine::new(self.storage.clone()).compact_all()
    }

    pub fn enforce_retention(&self) -> StreamResult<RetentionStats> {
        CompactionEngine::new(self.storage.clone()).enforce_retention_all()
    }

    // ─── Tiered storage ───────────────────────────────────────────────────────

    pub fn get_tier_config(&self) -> StreamResult<StorageTierConfig> {
        self.storage.get_tier_config()
    }

    pub fn set_tier_config(&self, cfg: StorageTierConfig) -> StreamResult<()> {
        self.storage.set_tier_config(cfg)
    }

    // ─── Connectors ───────────────────────────────────────────────────────────

    pub fn list_connectors(&self) -> StreamResult<Vec<ConnectorConfig>> {
        self.storage.list_connectors()
    }

    // ─── Broker / cluster info ────────────────────────────────────────────────

    pub fn cluster_info(&self) -> ClusterInfo {
        let topics = self.storage.list_topics().unwrap_or_default();
        let groups = self.storage.list_groups().unwrap_or_default();
        let total_partitions: u32 = topics.iter().map(|t| t.partitions).sum();

        ClusterInfo {
            broker_id: 1,
            broker_host: "localhost".into(),
            kafka_port: 9092,
            api_port: 8080,
            topic_count: topics.len(),
            partition_count: total_partitions as usize,
            group_count: groups.len(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

/// Cluster-level metadata returned by the admin API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClusterInfo {
    pub broker_id: i32,
    pub broker_host: String,
    pub kafka_port: u16,
    pub api_port: u16,
    pub topic_count: usize,
    pub partition_count: usize,
    pub group_count: usize,
    pub version: String,
}
