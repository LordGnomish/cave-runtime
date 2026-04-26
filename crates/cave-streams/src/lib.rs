//! CAVE Streams — Kafka-compatible streaming platform.
//!
//! Compatible with: Apache Kafka + Confluent Schema Registry + Kafka Connect
//!
//! Features:
//! - Full Kafka wire protocol (API keys 0-67)
//! - Topic management (create, delete, describe, alter configs)
//! - Partition management (rebalance, reassignment)
//! - Consumer groups (range, roundrobin, sticky, cooperative-sticky)
//! - Exactly-once semantics (idempotent producer, transactions)
//! - Schema Registry (Avro, Protobuf, JSON Schema)
//! - Kafka Connect API (connectors, tasks, transforms)
//! - Message compression (gzip, snappy, lz4, zstd)
//! - Log compaction and retention policies
//! - ACLs (per topic, group, cluster)
//! - Quotas (produce/fetch byte rate, request rate)
//! - MirrorMaker pattern (cross-cluster replication)

pub mod acl;
pub mod broker;
pub mod compression;
pub mod connect;
pub mod consumer_group;
pub mod error;
pub mod mirror;
pub mod protocol;
pub mod quota;
pub mod routes;
pub mod schema_registry;
pub mod server;
pub mod transactions;

use axum::Router;
use std::sync::Arc;

pub use broker::Broker;
pub use error::{StreamsError, StreamsResult};

pub const MODULE_NAME: &str = "streams";
/// Default Kafka wire protocol port
pub const KAFKA_PORT: u16 = 9092;
/// Default Schema Registry port
pub const SCHEMA_REGISTRY_PORT: u16 = 8081;

/// Shared application state for the streams module.
pub struct StreamsState {
    pub broker: Arc<Broker>,
}

impl Default for StreamsState {
    fn default() -> Self {
        Self {
            broker: Arc::new(Broker::new(broker::BrokerConfig::default())),
        }
    }
}

/// Build Axum management/REST router (Schema Registry + Connect + admin endpoints).
pub fn router(state: Arc<StreamsState>) -> Router {
    routes::create_router(state)
}
