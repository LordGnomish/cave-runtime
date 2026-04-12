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

// ── Stream ────────────────────────────────────────────────────────────────────

/// A partition-less, auto-scaling event channel.
/// Unlike Kafka topics, streams have no explicit partition count —
/// the platform handles horizontal scaling transparently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    pub id: Uuid,
    pub name: String,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_age_seconds: Option<u64>,
    pub max_bytes: Option<u64>,
    pub max_messages: Option<u64>,
    pub on_full: OnFullPolicy,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_seconds: Some(86400 * 7), // 7 days
            max_bytes: None,
            max_messages: None,
            on_full: OnFullPolicy::DropOldest,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFullPolicy {
    DropOldest,
    Reject,
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputLimit {
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleRequest {
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

// ── Subscription ──────────────────────────────────────────────────────────────

/// A named consumer binding on a stream with integrated retry and DLQ policy.
/// Unlike Kafka consumer groups, subscriptions own their retry/DLQ lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionType {
    /// Platform pushes to a configured webhook endpoint.
    Push,
    /// Consumer polls on demand (default).
    Pull,
    /// Broadcast — every active consumer receives every message.
    Fanout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self::Latest
    }
}

/// Integrated retry policy — no external retry framework needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub backoff: BackoffStrategy,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    /// Move to DLQ after all retries are exhausted.
    pub dead_letter_after_retries: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: BackoffStrategy::Exponential,
            initial_delay_ms: 1_000,
            max_delay_ms: 60_000,
            dead_letter_after_retries: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    Fixed,
    Linear,
    Exponential,
    /// Exponential + random jitter to avoid thundering herd.
    Jittered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Active,
    Paused,
    /// Consumer is too slow; platform is shedding or buffering.
    Backpressure,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub name: String,
    pub subscription_type: SubscriptionType,
    pub delivery_policy: Option<DeliveryPolicy>,
    pub retry_policy: Option<RetryPolicy>,
    pub dead_letter_stream: Option<String>,
    pub filter_expression: Option<String>,
    pub ack_deadline_seconds: Option<u32>,
    pub exactly_once: Option<bool>,
}

// ── Message ───────────────────────────────────────────────────────────────────

/// An individual event stored in a stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub key: Option<String>,
    pub payload: serde_json::Value,
    pub headers: Option<HashMap<String, String>>,
    pub schema_id: Option<Uuid>,
    /// Supply a stable UUID to enable exactly-once semantics.
    /// Duplicate publishes with the same ID are silently dropped.
    pub deduplication_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResponse {
    pub message_id: Uuid,
    pub stream_id: Uuid,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub storage_tier: StorageTierHint,
    /// True if this message was a duplicate and was not stored again.
    pub deduplicated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub subscription_id: Uuid,
    pub max_messages: Option<u32>,
    pub ack_deadline_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckRequest {
    pub message_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckResponse {
    pub acked: Vec<Uuid>,
    pub not_found: Vec<Uuid>,
    pub cursor_advanced_to: u64,
}

// ── Schema Registry ───────────────────────────────────────────────────────────

/// Built-in schema registry — not a separate service.
/// Supports Avro, Protobuf, JSON Schema, and raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub id: Uuid,
    /// Namespace-qualified name, e.g. `orders.v2`.
    pub subject: String,
    pub format: SchemaFormat,
    pub version: u32,
    pub definition: String,
    pub compatibility: CompatibilityMode,
    /// FNV-1a fingerprint for content-based deduplication.
    pub fingerprint: String,
    /// Number of streams currently using this schema.
    pub stream_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SchemaFormat {
    Avro,
    Protobuf,
    JsonSchema,
    Raw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatibilityMode {
    None,
    Backward,
    BackwardTransitive,
    Forward,
    ForwardTransitive,
    Full,
    FullTransitive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSchemaRequest {
    pub subject: String,
    pub format: SchemaFormat,
    pub definition: String,
    pub compatibility: Option<CompatibilityMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateSchemaRequest {
    pub subject: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateSchemaResponse {
    pub valid: bool,
    pub compatible: bool,
    pub errors: Vec<String>,
}

// ── Connector ─────────────────────────────────────────────────────────────────

/// Bridge to external systems. Supports lightweight DSL transforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connector {
    pub id: Uuid,
    pub name: String,
    /// Human-readable class name, e.g. `postgres-cdc`, `s3-sink`, `http-source`.
    pub connector_class: String,
    pub direction: ConnectorDirection,
    pub stream_id: Uuid,
    pub status: ConnectorStatus,
    pub config: serde_json::Value,
    /// Optional transformation DSL applied before write/after read.
    pub transform_dsl: Option<String>,
    pub error_msg: Option<String>,
    pub messages_processed: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorDirection {
    Source,
    Sink,
    Bidirectional,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    Provisioning,
    Running,
    Paused,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateConnectorRequest {
    pub name: String,
    pub connector_class: String,
    pub direction: ConnectorDirection,
    pub stream_id: Uuid,
    pub config: serde_json::Value,
    pub transform_dsl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchConnectorRequest {
    pub status: Option<ConnectorStatus>,
    pub config: Option<serde_json::Value>,
    pub transform_dsl: Option<String>,
}

// ── Dead Letter Queue ─────────────────────────────────────────────────────────

/// A message that has exhausted its retry policy.
/// Can be replayed individually or in bulk.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageTierConfig {
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
            hot: HotStorageConfig {
                max_memory_bytes: 512 * 1024 * 1024,
                target_retention_seconds: 300,
            },
            warm: WarmStorageConfig {
                path: "/var/lib/cave-streams/warm".to_string(),
                max_bytes: 10 * 1024 * 1024 * 1024,
                compression: WarmCompression::Lz4,
            },
            cold: ColdStorageConfig {
                provider: ObjectStorageProvider::S3Compatible,
                endpoint: "http://minio:9000".to_string(),
                bucket: "cave-streams".to_string(),
                prefix: "events/".to_string(),
                region: "us-east-1".to_string(),
                compression: ColdCompression::Zstd,
            },
            auto_tier_enabled: true,
            hot_to_warm_threshold_percent: 80,
            warm_to_cold_age_seconds: 3600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotStorageConfig {
    /// Maximum bytes to keep in RAM across all streams.
    pub max_memory_bytes: u64,
    /// Target how long messages stay in hot tier before spill.
    pub target_retention_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmStorageConfig {
    pub path: String,
    pub max_bytes: u64,
    pub compression: WarmCompression,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarmCompression {
    None,
    Lz4,
    Snappy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColdStorageConfig {
    pub provider: ObjectStorageProvider,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub compression: ColdCompression,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStorageProvider {
    S3Compatible,
    Gcs,
    AzureBlob,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdCompression {
    None,
    Zstd,
    Gzip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageTierStats {
    pub hot_message_count: u64,
    pub warm_message_count: u64,
    pub cold_message_count: u64,
    pub hot_estimated_bytes: u64,
    pub warm_estimated_bytes: u64,
    pub cold_estimated_bytes: u64,
    pub auto_tier_enabled: bool,
    pub config: StorageTierConfig,
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

// ── Backpressure ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackpressureStatus {
    pub stream_id: Uuid,
    pub stream_name: String,
    pub throttle_active: bool,
    pub current_limit: Option<ThroughputLimit>,
    pub slow_subscriptions: Vec<SlowSubscription>,
    pub recommended_action: BackpressureAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowSubscription {
    pub subscription_id: Uuid,
    pub name: String,
    pub lag: u64,
    pub status: SubscriptionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackpressureAction {
    None,
    ThrottlePublishers,
    ScaleConsumers,
    MoveToColdTier,
    AlertOnly,
}
