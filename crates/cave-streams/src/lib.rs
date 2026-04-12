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
