<<<<<<< HEAD
//! # CAVE Streams
//!
//! Cloud-native event streaming platform — a production-grade Kafka replacement
//! without JVM or ZooKeeper dependencies.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                     cave-streams                         │
//! │                                                         │
//! │  ┌──────────┐  ┌──────────┐  ┌────────────────────┐    │
//! │  │ Producer │  │ Consumer │  │  Schema Registry   │    │
//! │  └──────────┘  └──────────┘  └────────────────────┘    │
//! │  ┌──────────┐  ┌──────────┐  ┌────────────────────┐    │
//! │  │  Topics  │  │ Connect  │  │   Streams API      │    │
//! │  └──────────┘  └──────────┘  └────────────────────┘    │
//! │  ┌──────────────────────────────────────────────────┐   │
//! │  │       Kafka Wire Protocol (TCP :9092)            │   │
//! │  └──────────────────────────────────────────────────┘   │
//! │  ┌──────────────────────────────────────────────────┐   │
//! │  │         REST Proxy + Admin API (HTTP :8080)      │   │
//! │  └──────────────────────────────────────────────────┘   │
//! │  ┌──────────────────────────────────────────────────┐   │
//! │  │   StreamStorage trait  (Memory / PostgreSQL)     │   │
//! │  └──────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Features
//!
//! - **Topics & Partitions** — Kafka-compatible topic/partition model.
//! - **Producer API** — key-hash, round-robin, and manual partitioning.
//! - **Consumer API** — consumer groups with eager and cooperative-sticky rebalancing.
//! - **Exactly-Once** — idempotent producers (PID + sequence) and transactional API.
//! - **Log Compaction** — keep latest value per key per partition.
//! - **Schema Registry** — Avro, JSON Schema, Protobuf; BACKWARD/FORWARD/FULL compatibility.
//! - **Kafka Protocol** — binary wire protocol so existing Kafka clients connect directly.
//! - **Connect API** — source/sink connector framework.
//! - **Streams API** — stateless (map/filter/flatMap) and stateful (aggregate/count/join) transforms.
//! - **Tiered Storage** — hot/warm/cold archival with S3-compatible object storage.
//! - **REST Proxy** — produce/consume via HTTP for clients without a Kafka SDK.
//! - **Admin API** — topic, group, compaction, and tier management over HTTP.
//! - **cave-db integration** — [`StreamStorage`] trait implemented by both in-memory and PostgreSQL backends.

pub mod admin;
pub mod compaction;
pub mod connect;
pub mod consumer;
pub mod error;
pub mod kafka_protocol;
pub mod models;
pub mod producer;
pub mod routes;
pub mod schema_registry;
pub mod storage;
pub mod streams_api;
pub mod topic;

#[cfg(test)]
mod tests;

// ─── Re-exports ───────────────────────────────────────────────────────────────

pub use error::{StreamError, StreamResult};
pub use models::{
    CompatibilityMode, ConnectorConfig, ConnectorDirection, ConnectorStatus,
    ConsumerGroup, GroupMember, GroupState,
    Header, PartitionLog, PartitionerStrategy, ProducerRecord, ProducerState,
    RebalanceProtocol, RecordMetadata, Record,
    Schema, SchemaType,
    StorageTierConfig, StreamPipelineConfig, StreamOperation,
    TopicConfig, TopicInfo, TopicPartition, Transaction, TransactionState,
};
pub use storage::{MemoryStorage, PostgresStorage, StreamStorage};
pub use producer::{Producer, ProducerRecordBuilder};
pub use consumer::{Consumer, GroupAdmin};
pub use topic::{TopicManager, TopicConfigPatch};
pub use compaction::CompactionEngine;
pub use schema_registry::SchemaRegistry;
pub use kafka_protocol::KafkaServer;
pub use connect::{ConnectorRegistry, SourceConnector, SinkConnector};
pub use streams_api::{PipelineRegistry, StreamPipelineBuilder};
pub use admin::AdminClient;
pub use routes::{StreamsState, router};

pub const MODULE_NAME: &str = "streams";
pub const KAFKA_DEFAULT_PORT: u16 = 9092;
=======
//! Cave Streams — cloud-native event streaming platform.
//!
//! Replaces: Apache Kafka, Confluent Platform, Schema Registry, Kafka Connect,
//!           Kafka Streams, NATS, Pulsar
//!
//! Design principles:
//!   - No JVM, no ZooKeeper — pure Rust, Kubernetes-native
//!   - Object storage (S3/MinIO) as primary durable tier
//!   - Partition-less topic model with automatic scaling
//!   - Built-in schema registry, DLQ, retry policies, exactly-once semantics
//!   - gRPC + HTTP API, NATS-like simplicity, Kafka-compatible migration path
//!   - Tiered storage: hot (memory) → warm (local SSD) → cold (S3/MinIO)
//!
//! Upstream tracking: see cave-upstream for monitored features.

pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use std::sync::{Arc, Mutex};

pub use store::StreamsStore;

/// Module state — all mutable platform state behind a single Mutex.
pub struct StreamsState {
    pub store: Arc<Mutex<StreamsStore>>,
}

impl Default for StreamsState {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(StreamsStore::default())),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<StreamsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "streams";
>>>>>>> claude/youthful-babbage
