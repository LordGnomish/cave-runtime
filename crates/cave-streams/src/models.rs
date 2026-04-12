//! Core domain models for cave-streams.
//! Domain models for cave-streams.
//!
//! Core abstractions:
//!   Stream      — partition-less, auto-scaling event channel (replaces Kafka Topic)
//!   Subscription — consumer binding with retry/DLQ/exactly-once (replaces ConsumerGroup)
//!   Message     — individual event with tiered storage placement
//!   Schema      — built-in Avro/Protobuf/JSON schema registry
//!   Connector   — source/sink bridge to external systems
//!   DeadLetterEntry — exhausted-retry message with replay support

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
// ── Stream ────────────────────────────────────────────────────────────────────
/// A partition-less, auto-scaling event channel.
/// Unlike Kafka topics, streams have no explicit partition count —
/// the platform handles horizontal scaling transparently.
pub struct Stream {
    pub id: Uuid,
    pub namespace: String,
    pub description: Option<String>,
    pub storage_tier: StorageTierHint,
    pub retention: RetentionPolicy,
    /// Optional schema enforced on publish.
    pub schema_id: Option<Uuid>,
    pub max_message_size_bytes: u32,
    pub throughput_limit: Option<ThroughputLimit>,
    pub subscriber_count: u32,
    pub message_count: u64,
    /// Monotonic sequence counter for ordering within this stream.
    pub sequence: u64,
    pub labels: HashMap<String, String>,
    pub updated_at: DateTime<Utc>,
/// Hint to the tiering engine for initial placement.
/// The platform will still tier down automatically based on age/pressure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StorageTierHint {
    /// Keep in memory as long as possible.
    Hot,
    /// Spill to local SSD soon after write.
    Warm,
    /// Send to object storage quickly (archival workloads).
    Cold,
    /// Let the platform decide based on access patterns.
    #[default]
    Auto,
pub struct RetentionPolicy {
    pub max_age_seconds: Option<u64>,
    pub max_bytes: Option<u64>,
    pub max_messages: Option<u64>,
    pub on_full: OnFullPolicy,
impl Default for RetentionPolicy {
            max_age_seconds: Some(86400 * 7), // 7 days
            max_bytes: None,
            max_messages: None,
            on_full: OnFullPolicy::DropOldest,
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
#[serde(rename_all = "snake_case")]
pub enum OnFullPolicy {
    DropOldest,
    Reject,
pub struct ThroughputLimit {
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
pub struct CreateStreamRequest {
    pub name: String,
    pub namespace: Option<String>,
    pub description: Option<String>,
    pub storage_tier: Option<StorageTierHint>,
    pub retention: Option<RetentionPolicy>,
    pub schema_id: Option<Uuid>,
    pub max_message_size_bytes: Option<u32>,
    pub throughput_limit: Option<ThroughputLimit>,
    pub labels: Option<HashMap<String, String>>,
pub struct ThrottleRequest {
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
pub struct StreamStats {
    pub stream_id: Uuid,
    pub name: String,
    pub namespace: String,
    pub message_count: u64,
    pub subscriber_count: u32,
    pub sequence: u64,
    pub throughput_limit: Option<ThroughputLimit>,
    pub storage_tier: StorageTierHint,
    pub retention: RetentionPolicy,
// ── Subscription ──────────────────────────────────────────────────────────────
/// A named consumer binding on a stream with integrated retry and DLQ policy.
/// Unlike Kafka consumer groups, subscriptions own their retry/DLQ lifecycle.
pub struct Subscription {
    pub id: Uuid,
    pub stream_id: Uuid,
    pub name: String,
    pub subscription_type: SubscriptionType,
    pub delivery_policy: DeliveryPolicy,
    pub retry_policy: RetryPolicy,
    /// Name of the stream to write undeliverable messages to.
    pub dead_letter_stream: Option<String>,
    /// JMESPath/CEL expression to filter messages.
    pub filter_expression: Option<String>,
    pub ack_deadline_seconds: u32,
    /// Enables idempotent delivery via deduplication IDs.
    pub exactly_once: bool,
    pub consumer_count: u32,
    /// Sequence of the last acknowledged message.
    pub cursor: u64,
    /// Estimated unacknowledged messages.
    pub lag: u64,
    pub status: SubscriptionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
#[serde(rename_all = "snake_case")]
pub enum SubscriptionType {
    /// Platform pushes to a configured webhook endpoint.
    Push,
    /// Consumer polls on demand (default).
    Pull,
    /// Broadcast — every active consumer receives every message.
    Fanout,
#[serde(rename_all = "snake_case")]
pub enum DeliveryPolicy {
    /// Start from the newest message at subscription creation.
    Latest,
    /// Replay from the very first retained message.
    Earliest,
    /// Resume from a known sequence number.
    BySequence(u64),
    /// Resume from a specific wall-clock timestamp.
    ByTimestamp(DateTime<Utc>),
impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self::Latest
/// Integrated retry policy — no external retry framework needed.
pub struct RetryPolicy {
    pub max_retries: u32,
    pub backoff: BackoffStrategy,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    /// Move to DLQ after all retries are exhausted.
    pub dead_letter_after_retries: bool,
impl Default for RetryPolicy {
    fn default() -> Self {
            max_retries: 3,
            backoff: BackoffStrategy::Exponential,
            initial_delay_ms: 1_000,
            max_delay_ms: 60_000,
            dead_letter_after_retries: true,
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
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    Fixed,
    Linear,
    Exponential,
    /// Exponential + random jitter to avoid thundering herd.
    Jittered,
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Active,
    Paused,
    /// Consumer is too slow; platform is shedding or buffering.
    Backpressure,
    Error,
pub struct CreateSubscriptionRequest {
    pub name: String,
    pub subscription_type: SubscriptionType,
    pub delivery_policy: Option<DeliveryPolicy>,
    pub retry_policy: Option<RetryPolicy>,
    pub dead_letter_stream: Option<String>,
    pub filter_expression: Option<String>,
    pub ack_deadline_seconds: Option<u32>,
    pub exactly_once: Option<bool>,
// ── Message ───────────────────────────────────────────────────────────────────
/// An individual event stored in a stream.
pub struct Message {
    pub id: Uuid,
    pub stream_id: Uuid,
    /// Monotonically increasing within the stream; used for cursor-based pull.
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub key: Option<String>,
    pub payload: serde_json::Value,
    pub headers: HashMap<String, String>,
    /// Schema used to validate/encode this message, if any.
    pub schema_id: Option<Uuid>,
    /// Current storage tier this message resides in.
    pub storage_tier: StorageTierHint,
    /// Number of times this message has been delivered (for retry tracking).
    pub delivery_count: u32,
pub struct PublishRequest {
    pub key: Option<String>,
    pub payload: serde_json::Value,
    pub headers: Option<HashMap<String, String>>,
    pub schema_id: Option<Uuid>,
    /// Supply a stable UUID to enable exactly-once semantics.
    /// Duplicate publishes with the same ID are silently dropped.
    pub deduplication_id: Option<Uuid>,
pub struct PublishResponse {
    pub message_id: Uuid,
    pub stream_id: Uuid,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub storage_tier: StorageTierHint,
    /// True if this message was a duplicate and was not stored again.
    pub deduplicated: bool,
pub struct PullRequest {
    pub subscription_id: Uuid,
    pub max_messages: Option<u32>,
    pub ack_deadline_seconds: Option<u32>,
pub struct AckRequest {
    pub message_ids: Vec<Uuid>,
pub struct AckResponse {
    pub acked: Vec<Uuid>,
    pub not_found: Vec<Uuid>,
    pub cursor_advanced_to: u64,
// ── Schema Registry ───────────────────────────────────────────────────────────
/// Built-in schema registry — not a separate service.
/// Supports Avro, Protobuf, JSON Schema, and raw bytes.
    pub id: Uuid,
    /// Namespace-qualified name, e.g. `orders.v2`.
    pub format: SchemaFormat,
    pub compatibility: CompatibilityMode,
    /// FNV-1a fingerprint for content-based deduplication.
    pub fingerprint: String,
    /// Number of streams currently using this schema.
    pub stream_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
pub enum SchemaFormat {
    Raw,
    None,
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
pub struct RegisterSchemaRequest {
    pub subject: String,
    pub format: SchemaFormat,
    pub definition: String,
    pub compatibility: Option<CompatibilityMode>,
pub struct ValidateSchemaRequest {
    pub subject: String,
    pub definition: String,
pub struct ValidateSchemaResponse {
    pub valid: bool,
    pub compatible: bool,
    pub errors: Vec<String>,
// ── Connector ─────────────────────────────────────────────────────────────────
/// Bridge to external systems. Supports lightweight DSL transforms.
pub struct Connector {
    pub id: Uuid,
    /// Human-readable class name, e.g. `postgres-cdc`, `s3-sink`, `http-source`.
    pub stream_id: Uuid,
    pub config: serde_json::Value,
    /// Optional transformation DSL applied before write/after read.
    pub transform_dsl: Option<String>,
    pub error_msg: Option<String>,
    pub messages_processed: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
#[serde(rename_all = "snake_case")]
    Bidirectional,
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
    Provisioning,
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
pub struct CreateConnectorRequest {
    pub name: String,
    pub connector_class: String,
    pub direction: ConnectorDirection,
    pub stream_id: Uuid,
    pub config: serde_json::Value,
    pub transform_dsl: Option<String>,
}
pub struct PatchConnectorRequest {
    pub status: Option<ConnectorStatus>,
    pub config: Option<serde_json::Value>,
    pub transform_dsl: Option<String>,
}
// ── Dead Letter Queue ─────────────────────────────────────────────────────────
/// A message that has exhausted its retry policy.
/// Can be replayed individually or in bulk.
pub struct DeadLetterEntry {
    pub id: Uuid,
    pub original_message_id: Uuid,
    pub stream_id: Uuid,
    pub subscription_id: Uuid,
    pub payload: serde_json::Value,
    pub headers: HashMap<String, String>,
    pub failure_reason: String,
    pub retry_count: u32,
    pub last_retry_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub retry_policy: RetryPolicy,
    pub status: DlqStatus,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DlqStatus {
    /// Awaiting retry.
    Pending,
    /// Retry in progress.
    Retrying,
    /// All retries exhausted.
    Exhausted,
    /// Manually discarded.
    Discarded,
    /// Successfully replayed.
    Resolved,
}
// ── Tiered Storage ────────────────────────────────────────────────────────────
/// Platform-wide tiered storage configuration.
/// hot → warm → cold promotion happens automatically.
    pub hot: HotStorageConfig,
    pub warm: WarmStorageConfig,
    pub cold: ColdStorageConfig,
    pub auto_tier_enabled: bool,
    /// Spill hot→warm when hot memory is this % full.
    pub hot_to_warm_threshold_percent: u8,
    /// Move warm→cold after this many seconds.
    pub warm_to_cold_age_seconds: u64,
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
            hot: HotStorageConfig {
                max_memory_bytes: 512 * 1024 * 1024,
                target_retention_seconds: 300,
            warm: WarmStorageConfig {
                path: "/var/lib/cave-streams/warm".to_string(),
                compression: WarmCompression::Lz4,
            cold: ColdStorageConfig {
                provider: ObjectStorageProvider::S3Compatible,
                endpoint: "http://minio:9000".to_string(),
                bucket: "cave-streams".to_string(),
                prefix: "events/".to_string(),
                region: "us-east-1".to_string(),
                compression: ColdCompression::Zstd,
            auto_tier_enabled: true,
            hot_to_warm_threshold_percent: 80,
            warm_to_cold_age_seconds: 3600,
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
pub struct HotStorageConfig {
    /// Maximum bytes to keep in RAM across all streams.
    pub max_memory_bytes: u64,
    /// Target how long messages stay in hot tier before spill.
    pub target_retention_seconds: u64,
pub struct WarmStorageConfig {
    pub path: String,
    pub compression: WarmCompression,
#[serde(rename_all = "snake_case")]
pub enum WarmCompression {
    None,
    Lz4,
    Snappy,
pub struct ColdStorageConfig {
    pub provider: ObjectStorageProvider,
    pub prefix: String,
    pub compression: ColdCompression,
#[serde(rename_all = "snake_case")]
pub enum ObjectStorageProvider {
    S3Compatible,
    Gcs,
    AzureBlob,
    Local,
#[serde(rename_all = "snake_case")]
pub enum ColdCompression {
    None,
    Zstd,
    Gzip,
pub struct StorageTierStats {
    pub hot_message_count: u64,
    pub warm_message_count: u64,
    pub cold_message_count: u64,
    pub hot_estimated_bytes: u64,
    pub warm_estimated_bytes: u64,
    pub cold_estimated_bytes: u64,
    pub auto_tier_enabled: bool,
    pub config: StorageTierConfig,
// ── Metrics ───────────────────────────────────────────────────────────────────
pub struct StreamMetrics {
    pub stream_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub messages_in_per_sec: f64,
    pub messages_out_per_sec: f64,
    pub bytes_in_per_sec: f64,
    pub bytes_out_per_sec: f64,
    pub subscriber_count: u32,
    pub dlq_count: u64,
    pub hot_bytes: u64,
    pub warm_bytes: u64,
    pub cold_bytes: u64,
    pub publish_latency_ms_p50: f64,
    pub publish_latency_ms_p99: f64,
    pub end_to_end_latency_ms_p99: f64,
    pub exactly_once_dedup_count: u64,
pub struct PlatformMetrics {
    pub timestamp: DateTime<Utc>,
    pub total_streams: u64,
    pub total_subscriptions: u64,
    pub total_messages: u64,
    pub total_connectors: u64,
    pub total_dlq_entries: u64,
    pub active_schemas: u64,
    pub dedup_cache_size: u64,
    pub per_stream: Vec<StreamMetrics>,
// ── Backpressure ──────────────────────────────────────────────────────────────
pub struct BackpressureStatus {
    pub stream_id: Uuid,
    pub stream_name: String,
    pub throttle_active: bool,
    pub current_limit: Option<ThroughputLimit>,
    pub slow_subscriptions: Vec<SlowSubscription>,
    pub recommended_action: BackpressureAction,
pub struct SlowSubscription {
    pub subscription_id: Uuid,
    pub lag: u64,
    pub status: SubscriptionStatus,
#[serde(rename_all = "snake_case")]
pub enum BackpressureAction {
    None,
    ThrottlePublishers,
    ScaleConsumers,
    MoveToColdTier,
    AlertOnly,
}
