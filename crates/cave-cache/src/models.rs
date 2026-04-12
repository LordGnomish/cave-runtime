//! Data models for cave-cache.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single cached entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub value: serde_json::Value,
    /// TTL in seconds, if set.
    pub ttl: Option<u64>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Incremented on every read (used by LFU eviction).
    pub access_count: u64,
    /// Updated on every read (used by LRU eviction).
    pub last_accessed: DateTime<Utc>,
}

/// Aggregate cache statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheStats {
    pub total_keys: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub pubsub_channels: usize,
    pub pubsub_messages_total: u64,
}

/// Eviction strategy applied when memory pressure is high.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvictionPolicy {
    /// Evict least-recently-used keys first.
    #[default]
    Lru,
    /// Evict least-frequently-used keys first.
    Lfu,
    /// Evict keys whose TTL expires soonest.
    Ttl,
}

/// Snapshot of a pub/sub channel, including recent buffered messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubSubChannel {
    pub name: String,
    pub message_count: u64,
    pub recent_messages: Vec<PubSubMessage>,
}

/// A message published to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubSubMessage {
    pub message_id: Uuid,
    pub channel: String,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

/// Logical cluster of cache nodes (metadata only — actual sharding is external).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheCluster {
    pub cluster_id: Uuid,
    pub nodes: Vec<CacheNode>,
    pub replication_factor: u8,
    pub eviction_policy: EvictionPolicy,
}

/// A single node in a cache cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheNode {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub is_primary: bool,
    /// Hash-slot range start (0–16383, Redis-compatible).
    pub slot_start: u16,
    /// Hash-slot range end (inclusive).
    pub slot_end: u16,
}
