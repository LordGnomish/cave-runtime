// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory Kafka broker — topic/partition log, offset tracking.

use crate::acl::AclStore;
use crate::compression::Codec;
use crate::consumer_group::GroupCoordinator;
use crate::error::{StreamsError, StreamsResult};
use crate::quota::QuotaManager;
use crate::transactions::TransactionCoordinator;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

// ── Configuration ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    /// Broker ID (default: 1)
    pub broker_id: i32,
    /// Hostname advertised to clients
    pub host: String,
    /// Port (default: 9092)
    pub port: u16,
    /// Default number of partitions for auto-created topics
    pub default_num_partitions: i32,
    /// Default replication factor
    pub default_replication_factor: i16,
    /// Default log retention duration (ms), -1 = unlimited
    pub log_retention_ms: i64,
    /// Default log retention bytes, -1 = unlimited
    pub log_retention_bytes: i64,
    /// Default log segment bytes
    pub log_segment_bytes: i64,
    /// Enable log compaction by default
    pub log_compaction: bool,
    /// Default message max bytes
    pub message_max_bytes: usize,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            broker_id: 1,
            host: "localhost".into(),
            port: 9092,
            default_num_partitions: 1,
            default_replication_factor: 1,
            log_retention_ms: 7 * 24 * 3600 * 1000, // 7 days
            log_retention_bytes: -1,
            log_segment_bytes: 1_073_741_824, // 1 GiB
            log_compaction: false,
            message_max_bytes: 1_048_576, // 1 MiB
        }
    }
}

// ── Topic config ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicConfig {
    pub cleanup_policy: CleanupPolicy,
    pub retention_ms: i64,
    pub retention_bytes: i64,
    pub segment_bytes: i64,
    pub max_message_bytes: usize,
    pub compression_type: Codec,
    pub min_in_sync_replicas: i32,
    pub extra: HashMap<String, String>,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            cleanup_policy: CleanupPolicy::Delete,
            retention_ms: 7 * 24 * 3600 * 1000,
            retention_bytes: -1,
            segment_bytes: 1_073_741_824,
            max_message_bytes: 1_048_576,
            compression_type: Codec::None,
            min_in_sync_replicas: 1,
            extra: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CleanupPolicy {
    Delete,
    Compact,
    CompactDelete,
}

// ── Record batch ──────────────────────────────────────────────────────────────

/// A record batch stored in the partition log.
#[derive(Debug, Clone)]
pub struct RecordBatch {
    pub base_offset: i64,
    pub last_offset_delta: i32,
    pub first_timestamp: i64,
    pub max_timestamp: i64,
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub base_sequence: i32,
    pub is_transactional: bool,
    pub codec: Codec,
    pub data: Bytes,
}

impl RecordBatch {
    pub fn record_count(&self) -> i32 {
        self.last_offset_delta + 1
    }

    pub fn last_offset(&self) -> i64 {
        self.base_offset + self.last_offset_delta as i64
    }
}

// ── Partition log ─────────────────────────────────────────────────────────────

pub struct PartitionLog {
    pub partition_index: i32,
    pub leader_epoch: i32,
    batches: VecDeque<RecordBatch>,
    log_start_offset: i64,
    log_end_offset: i64, // next offset to be assigned
    config: TopicConfig,
}

impl PartitionLog {
    pub fn new(partition_index: i32, config: TopicConfig) -> Self {
        Self {
            partition_index,
            leader_epoch: 0,
            batches: VecDeque::new(),
            log_start_offset: 0,
            log_end_offset: 0,
            config,
        }
    }

    /// Append raw record bytes, returning the base offset assigned.
    pub fn append(
        &mut self,
        data: Bytes,
        producer_id: i64,
        producer_epoch: i16,
        base_sequence: i32,
        is_transactional: bool,
        codec: Codec,
    ) -> StreamsResult<i64> {
        let base_offset = self.log_end_offset;
        let batch = RecordBatch {
            base_offset,
            last_offset_delta: 0, // single batch simplified
            first_timestamp: Utc::now().timestamp_millis(),
            max_timestamp: Utc::now().timestamp_millis(),
            producer_id,
            producer_epoch,
            base_sequence,
            is_transactional,
            codec,
            data,
        };
        self.log_end_offset += 1;
        self.batches.push_back(batch);
        self.maybe_enforce_retention();
        Ok(base_offset)
    }

    /// Fetch records starting at `fetch_offset`, up to `max_bytes`.
    pub fn fetch(&self, fetch_offset: i64, max_bytes: i32) -> StreamsResult<Vec<&RecordBatch>> {
        if fetch_offset > self.log_end_offset {
            return Err(StreamsError::OffsetOutOfRange {
                topic: String::new(),
                partition: self.partition_index,
                offset: fetch_offset,
            });
        }
        let mut result = Vec::new();
        let mut accumulated = 0i32;
        for batch in &self.batches {
            if batch.base_offset < fetch_offset {
                continue;
            }
            let batch_size = batch.data.len() as i32;
            if accumulated > 0 && accumulated + batch_size > max_bytes {
                break;
            }
            result.push(batch);
            accumulated += batch_size;
        }
        Ok(result)
    }

    pub fn log_start_offset(&self) -> i64 {
        self.log_start_offset
    }

    pub fn log_end_offset(&self) -> i64 {
        self.log_end_offset
    }

    /// Delete records before `before_offset`.
    pub fn delete_records(&mut self, before_offset: i64) {
        while let Some(front) = self.batches.front() {
            if front.base_offset < before_offset {
                self.batches.pop_front();
            } else {
                break;
            }
        }
        self.log_start_offset = self.log_start_offset.max(before_offset);
    }

    fn maybe_enforce_retention(&mut self) {
        // Size-based retention
        if self.config.retention_bytes > 0 {
            let total_bytes: usize = self.batches.iter().map(|b| b.data.len()).sum();
            let limit = self.config.retention_bytes as usize;
            while total_bytes > limit {
                if let Some(front) = self.batches.pop_front() {
                    self.log_start_offset = front.base_offset + front.record_count() as i64;
                } else {
                    break;
                }
            }
        }
        // Time-based retention
        if self.config.retention_ms > 0 {
            let cutoff = Utc::now().timestamp_millis() - self.config.retention_ms;
            while let Some(front) = self.batches.front() {
                if front.max_timestamp < cutoff {
                    self.log_start_offset =
                        front.base_offset + front.record_count() as i64;
                    self.batches.pop_front();
                } else {
                    break;
                }
            }
        }
    }
}

// ── Topic ─────────────────────────────────────────────────────────────────────

pub struct Topic {
    pub name: String,
    pub partitions: Vec<PartitionLog>,
    pub config: TopicConfig,
    pub created_at: DateTime<Utc>,
    pub is_internal: bool,
}

impl Topic {
    pub fn new(name: String, num_partitions: i32, config: TopicConfig) -> Self {
        let partitions = (0..num_partitions)
            .map(|i| PartitionLog::new(i, config.clone()))
            .collect();
        Self {
            name,
            partitions,
            config,
            created_at: Utc::now(),
            is_internal: false,
        }
    }

    pub fn partition(&self, index: i32) -> StreamsResult<&PartitionLog> {
        self.partitions.get(index as usize).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: self.name.clone(),
                partition: index,
            }
        })
    }

    pub fn partition_mut(&mut self, index: i32) -> StreamsResult<&mut PartitionLog> {
        let name = self.name.clone();
        self.partitions.get_mut(index as usize).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: name,
                partition: index,
            }
        })
    }

    /// Add new partitions to the topic.
    pub fn add_partitions(&mut self, new_count: i32) -> StreamsResult<()> {
        let current = self.partitions.len() as i32;
        if new_count <= current {
            return Err(StreamsError::InvalidTopicName(format!(
                "new partition count {new_count} must be > current {current}"
            )));
        }
        for i in current..new_count {
            self.partitions.push(PartitionLog::new(i, self.config.clone()));
        }
        Ok(())
    }
}

// ── Broker ────────────────────────────────────────────────────────────────────

/// Central broker managing all state.
pub struct Broker {
    pub config: BrokerConfig,
    /// topic_name → Topic
    topics: DashMap<String, Topic>,
    pub groups: Arc<GroupCoordinator>,
    pub transactions: Arc<TransactionCoordinator>,
    pub acls: Arc<AclStore>,
    pub quotas: Arc<QuotaManager>,
    next_producer_id: AtomicI64,
    /// Committed consumer offsets: (group, topic, partition) → offset
    committed_offsets: DashMap<(String, String, i32), i64>,
    /// Partition reassignments in progress: topic → partition → target replicas
    pending_reassignments: DashMap<String, HashMap<i32, Vec<i32>>>,
    #[allow(dead_code)]
    generation: AtomicI32,
    /// Optional KRaft handler. `None` for non-controller brokers
    /// and for installs that haven't switched to KRaft mode yet
    /// (the broker still runs against in-memory metadata).
    /// `Some(...)` enables Vote / BeginQuorumEpoch /
    /// EndQuorumEpoch / DescribeQuorum dispatch in
    /// `server::dispatch_request`.
    kraft: std::sync::OnceLock<Arc<crate::kraft::KraftHandler>>,
}

impl Broker {
    pub fn new(config: BrokerConfig) -> Self {
        Self {
            config,
            topics: DashMap::new(),
            groups: Arc::new(GroupCoordinator::new()),
            transactions: Arc::new(TransactionCoordinator::new()),
            acls: Arc::new(AclStore::new()),
            quotas: Arc::new(QuotaManager::new()),
            next_producer_id: AtomicI64::new(1),
            committed_offsets: DashMap::new(),
            pending_reassignments: DashMap::new(),
            generation: AtomicI32::new(0),
            kraft: std::sync::OnceLock::new(),
        }
    }

    /// Install a KRaft handler. Idempotent — first call wins;
    /// subsequent calls are no-ops (cluster init only mints one
    /// handler).
    pub fn set_kraft_handler(&self, handler: Arc<crate::kraft::KraftHandler>) {
        let _ = self.kraft.set(handler);
    }

    /// Borrow the KRaft handler if installed.
    pub fn kraft_handler(&self) -> Option<Arc<crate::kraft::KraftHandler>> {
        self.kraft.get().cloned()
    }

    // ── Topic management ──────────────────────────────────────────────────────

    pub fn create_topic(
        &self,
        name: String,
        num_partitions: i32,
        _replication_factor: i16,
        configs: Vec<(String, Option<String>)>,
    ) -> StreamsResult<()> {
        self.validate_topic_name(&name)?;
        if self.topics.contains_key(&name) {
            return Err(StreamsError::TopicAlreadyExists(name));
        }
        let mut topic_config = TopicConfig::default();
        for (k, v) in configs {
            self.apply_config(&mut topic_config, &k, v.as_deref());
        }
        let np = if num_partitions < 0 {
            self.config.default_num_partitions
        } else {
            num_partitions
        };
        self.topics.insert(name.clone(), Topic::new(name, np, topic_config));
        Ok(())
    }

    pub fn delete_topic(&self, name: &str) -> StreamsResult<()> {
        self.topics
            .remove(name)
            .ok_or_else(|| StreamsError::UnknownTopicOrPartition {
                topic: name.into(),
                partition: -1,
            })?;
        Ok(())
    }

    pub fn list_topics(&self) -> Vec<String> {
        self.topics.iter().map(|e| e.key().clone()).collect()
    }

    pub fn topic_exists(&self, name: &str) -> bool {
        self.topics.contains_key(name)
    }

    pub fn topic_partition_count(&self, name: &str) -> StreamsResult<i32> {
        self.topics
            .get(name)
            .map(|t| t.partitions.len() as i32)
            .ok_or_else(|| StreamsError::UnknownTopicOrPartition {
                topic: name.into(),
                partition: -1,
            })
    }

    pub fn add_partitions(&self, topic: &str, new_count: i32) -> StreamsResult<()> {
        let mut t = self.topics.get_mut(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition: -1,
            }
        })?;
        t.add_partitions(new_count)
    }

    // ── Produce ───────────────────────────────────────────────────────────────

    pub fn produce(
        &self,
        topic: &str,
        partition: i32,
        data: Bytes,
        producer_id: i64,
        producer_epoch: i16,
        base_sequence: i32,
        is_transactional: bool,
        codec: Codec,
    ) -> StreamsResult<i64> {
        let mut t = self.topics.get_mut(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        let log = t.partition_mut(partition)?;
        log.append(data, producer_id, producer_epoch, base_sequence, is_transactional, codec)
    }

    // ── Fetch ─────────────────────────────────────────────────────────────────

    pub fn fetch(
        &self,
        topic: &str,
        partition: i32,
        offset: i64,
        max_bytes: i32,
    ) -> StreamsResult<Vec<RecordBatch>> {
        let t = self.topics.get(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        let log = t.partition(partition)?;
        Ok(log.fetch(offset, max_bytes)?.iter().map(|b| (*b).clone()).collect())
    }

    // ── Offset management ─────────────────────────────────────────────────────

    pub fn log_end_offset(&self, topic: &str, partition: i32) -> StreamsResult<i64> {
        let t = self.topics.get(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        Ok(t.partition(partition)?.log_end_offset())
    }

    pub fn log_start_offset(&self, topic: &str, partition: i32) -> StreamsResult<i64> {
        let t = self.topics.get(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        Ok(t.partition(partition)?.log_start_offset())
    }

    pub fn commit_offset(&self, group: &str, topic: &str, partition: i32, offset: i64) {
        self.committed_offsets
            .insert((group.into(), topic.into(), partition), offset);
    }

    pub fn fetch_offset(&self, group: &str, topic: &str, partition: i32) -> i64 {
        self.committed_offsets
            .get(&(group.into(), topic.into(), partition))
            .map(|v| *v)
            .unwrap_or(-1)
    }

    pub fn delete_records(&self, topic: &str, partition: i32, before_offset: i64) -> StreamsResult<i64> {
        let mut t = self.topics.get_mut(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        let log = t.partition_mut(partition)?;
        log.delete_records(before_offset);
        Ok(log.log_start_offset())
    }

    // ── Producer ID allocation ────────────────────────────────────────────────

    pub fn allocate_producer_id(&self) -> i64 {
        self.next_producer_id.fetch_add(1, Ordering::SeqCst)
    }

    // ── Partition reassignment ────────────────────────────────────────────────

    pub fn start_reassignment(
        &self,
        topic: &str,
        assignments: HashMap<i32, Vec<i32>>,
    ) -> StreamsResult<()> {
        if !self.topics.contains_key(topic) {
            return Err(StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition: -1,
            });
        }
        self.pending_reassignments.insert(topic.into(), assignments);
        Ok(())
    }

    pub fn list_reassignments(&self) -> HashMap<String, HashMap<i32, Vec<i32>>> {
        self.pending_reassignments
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    pub fn cancel_reassignment(&self, topic: &str) {
        self.pending_reassignments.remove(topic);
    }

    // ── Metadata ─────────────────────────────────────────────────────────────

    pub fn broker_id(&self) -> i32 {
        self.config.broker_id
    }

    pub fn cluster_id(&self) -> &str {
        "cave-streams-cluster"
    }

    pub fn controller_id(&self) -> i32 {
        self.config.broker_id
    }

    // ── Topic config helpers ──────────────────────────────────────────────────

    fn validate_topic_name(&self, name: &str) -> StreamsResult<()> {
        if name.is_empty() {
            return Err(StreamsError::InvalidTopicName("empty name".into()));
        }
        if name.len() > 249 {
            return Err(StreamsError::InvalidTopicName("name too long".into()));
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
            return Err(StreamsError::InvalidTopicName(format!(
                "invalid characters in '{name}'"
            )));
        }
        Ok(())
    }

    fn apply_config(&self, cfg: &mut TopicConfig, key: &str, value: Option<&str>) {
        match key {
            "retention.ms" => {
                if let Some(v) = value.and_then(|s| s.parse().ok()) {
                    cfg.retention_ms = v;
                }
            }
            "retention.bytes" => {
                if let Some(v) = value.and_then(|s| s.parse().ok()) {
                    cfg.retention_bytes = v;
                }
            }
            "cleanup.policy" => {
                cfg.cleanup_policy = match value {
                    Some("compact") => CleanupPolicy::Compact,
                    Some("compact,delete") | Some("delete,compact") => CleanupPolicy::CompactDelete,
                    _ => CleanupPolicy::Delete,
                };
            }
            "max.message.bytes" => {
                if let Some(v) = value.and_then(|s| s.parse().ok()) {
                    cfg.max_message_bytes = v;
                }
            }
            "min.insync.replicas" => {
                if let Some(v) = value.and_then(|s| s.parse().ok()) {
                    cfg.min_in_sync_replicas = v;
                }
            }
            "compression.type" => {
                cfg.compression_type = Codec::from_name(value.unwrap_or("none"));
            }
            k => {
                cfg.extra.insert(k.into(), value.unwrap_or("").into());
            }
        }
    }

    pub fn get_topic_configs(&self, topic: &str) -> StreamsResult<HashMap<String, String>> {
        let t = self.topics.get(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition: -1,
            }
        })?;
        let mut configs = HashMap::new();
        configs.insert("cleanup.policy".into(), format!("{:?}", t.config.cleanup_policy).to_lowercase());
        configs.insert("retention.ms".into(), t.config.retention_ms.to_string());
        configs.insert("retention.bytes".into(), t.config.retention_bytes.to_string());
        configs.insert("max.message.bytes".into(), t.config.max_message_bytes.to_string());
        for (k, v) in &t.config.extra {
            configs.insert(k.clone(), v.clone());
        }
        Ok(configs)
    }

    pub fn alter_topic_configs(
        &self,
        topic: &str,
        configs: Vec<(String, Option<String>)>,
    ) -> StreamsResult<()> {
        let mut t = self.topics.get_mut(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition: -1,
            }
        })?;
        let mut cfg = t.config.clone();
        for (k, v) in configs {
            self.apply_config(&mut cfg, &k, v.as_deref());
        }
        t.config = cfg;
        Ok(())
    }

    // ── Describe producers ────────────────────────────────────────────────────
    pub fn describe_producers(&self, topic: &str, partition: i32) -> StreamsResult<Vec<i64>> {
        // Return active producer IDs that have written to this partition
        let t = self.topics.get(topic).ok_or_else(|| {
            StreamsError::UnknownTopicOrPartition {
                topic: topic.into(),
                partition,
            }
        })?;
        let log = t.partition(partition)?;
        let pids: std::collections::HashSet<i64> = log
            .batches
            .iter()
            .filter(|b| b.producer_id > 0)
            .map(|b| b.producer_id)
            .collect();
        Ok(pids.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broker() -> Broker {
        Broker::new(BrokerConfig::default())
    }

    #[test]
    fn create_and_delete_topic() {
        let b = broker();
        b.create_topic("test-topic".into(), 3, 1, vec![]).unwrap();
        assert!(b.topic_exists("test-topic"));
        assert_eq!(b.topic_partition_count("test-topic").unwrap(), 3);
        b.delete_topic("test-topic").unwrap();
        assert!(!b.topic_exists("test-topic"));
    }

    #[test]
    fn create_topic_duplicate_fails() {
        let b = broker();
        b.create_topic("dup".into(), 1, 1, vec![]).unwrap();
        assert!(matches!(
            b.create_topic("dup".into(), 1, 1, vec![]),
            Err(StreamsError::TopicAlreadyExists(_))
        ));
    }

    #[test]
    fn produce_and_fetch() {
        let b = broker();
        b.create_topic("events".into(), 1, 1, vec![]).unwrap();
        let offset = b
            .produce("events", 0, Bytes::from("hello"), -1, 0, 0, false, Codec::None)
            .unwrap();
        assert_eq!(offset, 0);
        let batches = b.fetch("events", 0, 0, 1024 * 1024).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].data, Bytes::from("hello"));
    }

    #[test]
    fn offset_commit_and_fetch() {
        let b = broker();
        b.commit_offset("my-group", "topic", 0, 42);
        assert_eq!(b.fetch_offset("my-group", "topic", 0), 42);
        assert_eq!(b.fetch_offset("my-group", "topic", 1), -1);
    }

    #[test]
    fn producer_id_allocation_monotonic() {
        let b = broker();
        let id1 = b.allocate_producer_id();
        let id2 = b.allocate_producer_id();
        assert!(id2 > id1);
    }

    #[test]
    fn add_partitions() {
        let b = broker();
        b.create_topic("grow-me".into(), 2, 1, vec![]).unwrap();
        b.add_partitions("grow-me", 5).unwrap();
        assert_eq!(b.topic_partition_count("grow-me").unwrap(), 5);
    }

    #[test]
    fn topic_name_validation() {
        let b = broker();
        assert!(b.create_topic("".into(), 1, 1, vec![]).is_err());
        assert!(b.create_topic("bad name!".into(), 1, 1, vec![]).is_err());
        assert!(b.create_topic("good-name_1.0".into(), 1, 1, vec![]).is_ok());
    }
}
