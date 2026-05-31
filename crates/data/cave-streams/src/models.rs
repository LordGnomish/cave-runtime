// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core domain models for cave-streams.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Record / Message ────────────────────────────────────────────────────────

/// A single event record in a partition log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Routing / compaction key (raw bytes, may be empty).
    pub key: Option<Vec<u8>>,
    /// Message payload (raw bytes).
    pub value: Option<Vec<u8>>,
    /// Arbitrary key-value metadata.
    pub headers: Vec<Header>,
    /// Unix epoch in milliseconds.
    pub timestamp_ms: i64,
    /// Absolute offset within the partition (assigned on append).
    pub offset: i64,
    /// Partition this record belongs to.
    pub partition: u32,
    /// Topic name.
    pub topic: String,
    // ── Exactly-once fields ──
    pub producer_id: Option<i64>,
    pub producer_epoch: Option<i16>,
    pub sequence: Option<i32>,
    /// True when record is a transactional control marker.
    pub is_control: bool,
}

impl Record {
    pub fn new(
        topic: impl Into<String>,
        partition: u32,
        key: Option<Vec<u8>>,
        value: Option<Vec<u8>>,
    ) -> Self {
        Self {
            key,
            value,
            headers: Vec::new(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            offset: -1,
            partition,
            topic: topic.into(),
            producer_id: None,
            producer_epoch: None,
            sequence: None,
            is_control: false,
        }
    }
}

/// A single header entry on a record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub key: String,
    pub value: Vec<u8>,
}

// ─── Topic ───────────────────────────────────────────────────────────────────

/// Topic metadata returned by list/describe operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicInfo {
    pub name: String,
    pub partitions: u32,
    pub replication_factor: u16,
    pub config: TopicConfig,
    pub created_at: DateTime<Utc>,
}

/// Mutable topic configuration knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicConfig {
    /// Retention by age (milliseconds). `None` = unlimited.
    pub retention_ms: Option<i64>,
    /// Retention by size (bytes). `None` = unlimited.
    pub retention_bytes: Option<i64>,
    /// Log cleanup policy.
    pub cleanup_policy: CleanupPolicy,
    /// Minimum in-sync replicas required for an ack.
    pub min_insync_replicas: u16,
    /// Producer-side compression codec.
    pub compression_type: CompressionType,
    /// Maximum record batch size in bytes.
    pub max_message_bytes: usize,
    /// Log segment file size.
    pub segment_bytes: usize,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            retention_ms: Some(7 * 24 * 60 * 60 * 1_000), // 7 days
            retention_bytes: None,
            cleanup_policy: CleanupPolicy::Delete,
            min_insync_replicas: 1,
            compression_type: CompressionType::None,
            max_message_bytes: 1_048_576, // 1 MiB
            segment_bytes: 1_073_741_824, // 1 GiB
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CleanupPolicy {
    Delete,
    Compact,
    DeleteAndCompact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    Gzip,
    Snappy,
    Lz4,
    Zstd,
}

// ─── Partition log (internal) ─────────────────────────────────────────────────

/// Per-partition append-only log kept in memory (or swapped to tiered storage).
#[derive(Debug, Clone)]
pub struct PartitionLog {
    pub topic: String,
    pub partition: u32,
    pub records: Vec<Record>,
    /// Offset that the log conceptually starts at (advances after compaction/retention).
    pub log_start_offset: i64,
    /// Offset of the next record to be appended.
    pub high_watermark: i64,
    /// Highest offset that has been compacted.
    pub last_compacted_offset: i64,
}

impl PartitionLog {
    pub fn new(topic: impl Into<String>, partition: u32) -> Self {
        Self {
            topic: topic.into(),
            partition,
            records: Vec::new(),
            log_start_offset: 0,
            high_watermark: 0,
            last_compacted_offset: -1,
        }
    }

    /// Append a record, assign its offset, return that offset.
    pub fn append(&mut self, mut record: Record) -> i64 {
        let offset = self.high_watermark;
        record.offset = offset;
        record.partition = self.partition;
        self.records.push(record);
        self.high_watermark += 1;
        offset
    }

    /// Fetch at most `max_count` records starting at `offset`.
    pub fn fetch(&self, offset: i64, max_count: usize) -> Vec<Record> {
        if offset < self.log_start_offset || offset >= self.high_watermark {
            return Vec::new();
        }
        // Records may have been compacted away; find the first record >= offset.
        let pos = self
            .records
            .partition_point(|r| r.offset < offset);
        self.records[pos..].iter().take(max_count).cloned().collect()
    }
}

// ─── Consumer groups ─────────────────────────────────────────────────────────

/// State of the whole consumer group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerGroup {
    pub group_id: String,
    pub members: HashMap<String, GroupMember>,
    /// Monotonically increasing rebalance counter.
    pub generation: i32,
    /// Member ID elected as assignment leader (runs partition assignor).
    pub leader_id: Option<String>,
    pub protocol: RebalanceProtocol,
    pub state: GroupState,
}

impl ConsumerGroup {
    pub fn new(group_id: impl Into<String>) -> Self {
        Self {
            group_id: group_id.into(),
            members: HashMap::new(),
            generation: 0,
            leader_id: None,
            protocol: RebalanceProtocol::Eager,
            state: GroupState::Empty,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub member_id: String,
    pub client_id: String,
    /// Topics the member wants to consume.
    pub subscriptions: Vec<String>,
    /// Partitions currently assigned to this member.
    pub assignments: Vec<TopicPartition>,
    /// Millisecond timestamp of the last heartbeat.
    pub last_heartbeat_ms: i64,
    pub session_timeout_ms: i32,
    pub rebalance_timeout_ms: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GroupState {
    Empty,
    PreparingRebalance,
    CompletingRebalance,
    Stable,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RebalanceProtocol {
    Eager,
    CooperativeSticky,
}

/// Identifies a specific (topic, partition) pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TopicPartition {
    pub topic: String,
    pub partition: u32,
}

impl TopicPartition {
    pub fn new(topic: impl Into<String>, partition: u32) -> Self {
        Self {
            topic: topic.into(),
            partition,
        }
    }
}

// ─── Idempotent producer / transactions ──────────────────────────────────────

/// Per-producer idempotency state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducerState {
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub transactional_id: Option<String>,
    /// Last accepted sequence per (topic, partition).
    pub last_sequence: HashMap<TopicPartition, i32>,
}

/// In-flight transactional produce operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub transactional_id: String,
    pub producer_id: i64,
    pub producer_epoch: i16,
    /// Buffered records, not yet visible to consumers.
    pub pending: Vec<(TopicPartition, Vec<Record>)>,
    pub state: TransactionState,
    pub timeout_ms: i32,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransactionState {
    Empty,
    Ongoing,
    PrepareCommit,
    PrepareAbort,
    CompleteCommit,
    CompleteAbort,
    Dead,
}

// ─── Schema registry ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub id: u32,
    pub subject: String,
    pub version: u32,
    pub schema_type: SchemaType,
    /// Raw schema definition (JSON for Avro/JSON Schema, IDL text for Protobuf).
    pub definition: String,
    /// FNV-1a fingerprint for deduplication.
    pub fingerprint: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SchemaType {
    Avro,
    JsonSchema,
    Protobuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatibilityMode {
    #[default]
    Backward,
    BackwardTransitive,
    Forward,
    ForwardTransitive,
    Full,
    FullTransitive,
    None,
}

// ─── Connectors ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    pub name: String,
    pub connector_class: String,
    pub config: HashMap<String, String>,
    pub topics: Vec<String>,
    pub direction: ConnectorDirection,
    pub status: ConnectorStatus,
    pub tasks_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectorDirection {
    Source,
    Sink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectorStatus {
    Running,
    Paused,
    Failed,
    Stopped,
}

// ─── Tiered storage ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageTierConfig {
    pub hot: HotTierConfig,
    pub warm: WarmTierConfig,
    pub cold: ColdTierConfig,
    pub enabled: bool,
}

impl Default for StorageTierConfig {
    fn default() -> Self {
        Self {
            hot: HotTierConfig {
                max_bytes: 512 * 1024 * 1024,
                max_age_ms: 3_600_000,
            },
            warm: WarmTierConfig {
                max_bytes: 10 * 1024 * 1024 * 1024,
                max_age_ms: 7 * 24 * 3_600_000,
                compression: CompressionType::Lz4,
            },
            cold: ColdTierConfig {
                endpoint: "http://localhost:9000".into(),
                bucket: "cave-streams".into(),
                access_key: String::new(),
                secret_key: String::new(),
                region: "us-east-1".into(),
                max_age_ms: None,
                compression: CompressionType::Zstd,
            },
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotTierConfig {
    /// Maximum bytes to keep in memory.
    pub max_bytes: u64,
    /// Maximum age in milliseconds before promoting to warm tier.
    pub max_age_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmTierConfig {
    pub max_bytes: u64,
    pub max_age_ms: i64,
    pub compression: CompressionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColdTierConfig {
    /// S3-compatible endpoint URL.
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    /// `None` means retain indefinitely in cold storage.
    pub max_age_ms: Option<i64>,
    pub compression: CompressionType,
}

// ─── Streams API pipeline ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamPipelineConfig {
    pub id: Uuid,
    pub name: String,
    pub source_topic: String,
    pub sink_topic: Option<String>,
    pub operations: Vec<StreamOperation>,
    pub state: PipelineState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamOperation {
    /// Stateless value transformation (expression language).
    Map { expression: String },
    /// Stateless predicate filter.
    Filter { predicate: String },
    /// One-to-many value expansion.
    FlatMap { expression: String },
    /// Key re-extraction before stateful operations.
    GroupBy { key_expression: String },
    /// Windowed aggregation.
    Aggregate {
        aggregation: AggregationType,
        window_ms: Option<i64>,
    },
    /// Rolling count per key.
    Count { window_ms: Option<i64> },
    /// Custom reducer (fold) per key.
    Reduce { expression: String, window_ms: Option<i64> },
    /// Windowed stream-stream join.
    Join {
        right_topic: String,
        window_ms: i64,
        join_key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AggregationType {
    Sum,
    Average,
    Min,
    Max,
    First,
    Last,
    Collect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PipelineState {
    Created,
    Running,
    Paused,
    Stopped,
    Failed,
}

// ─── Partitioner strategies ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PartitionerStrategy {
    /// Hash the record key modulo partition count.
    KeyHash,
    /// Sticky round-robin (null-key records).
    RoundRobin,
    /// Explicit partition override.
    Manual(u32),
    /// Kafka `DefaultPartitioner`-compatible key hashing
    /// (`toPositive(murmur2(key)) % numPartitions`); a null/empty key falls
    /// back to sticky round-robin, matching Apache Kafka 4.2.0.
    Murmur2,
}

// ─── Offset commit / fetch ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffsetCommitRequest {
    pub group_id: String,
    pub generation: i32,
    pub member_id: String,
    pub offsets: Vec<PartitionOffset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionOffset {
    pub topic: String,
    pub partition: u32,
    /// The next offset to be fetched (committed offset + 1 is the convention).
    pub offset: i64,
    pub metadata: Option<String>,
}

// ─── Producer input record ───────────────────────────────────────────────────

/// What a caller hands to the producer to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducerRecord {
    pub topic: String,
    pub key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
    pub headers: Vec<Header>,
    /// `None` = use current wall clock.
    pub timestamp_ms: Option<i64>,
    pub partitioner: PartitionerStrategy,
}

/// What the producer returns after a successful send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordMetadata {
    pub topic: String,
    pub partition: u32,
    pub offset: i64,
    pub timestamp_ms: i64,
}
