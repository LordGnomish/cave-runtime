// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Streams — Kafka + Pulsar streaming platform.
//!
//! Per ADR-RUNTIME-STREAMING-CONSOLIDATION-001 we run a single Rust broker
//! that speaks two wire protocols on separate ports: the Kafka binary
//! protocol (Apache Kafka 4.2.0) on 9092 and the Pulsar binary protocol
//! (Apache Pulsar 4.2.0) on 6650.  Addressing is Pulsar-canonical
//! (`persistent://tenant/ns/topic`); raw Kafka topic names are translated
//! to that form via [`pulsar_topic::TopicName::from_kafka_topic`].
//!
//! Features:
//! - Kafka wire protocol (API keys 0-67) — see [`protocol`] + [`kafka_wire`]
//! - Pulsar wire protocol (CONNECT/PRODUCER/SEND/SUBSCRIBE/FLOW/MESSAGE/ACK)
//!   — see [`pulsar_wire`]
//! - Multi-tenant addressing — see [`tenant`] + [`pulsar_topic`]
//! - Segment-based log + Bookkeeper-style replication ensemble — see
//!   [`segment_log`]
//! - Schema Registry (Avro, JSON Schema, Protobuf)
//! - Kafka Connect API (connectors, tasks, transforms)
//! - Consumer groups (range, roundrobin, sticky, cooperative-sticky)
//! - Exactly-once semantics (idempotent producer, transactions)
//! - Message compression (gzip, snappy, lz4, zstd)
//! - Log compaction and retention policies
//! - ACLs (per topic, group, cluster)
//! - Quotas (produce/fetch byte rate, request rate)
//! - MirrorMaker pattern (cross-cluster replication)

pub mod acl;
pub mod broker;
pub mod compression;
pub mod connect;
pub mod connect_rest;
pub mod connect_worker;
pub mod consumer_group;
pub mod error;
pub mod idempotent_producer;
pub mod incremental_rebalance;
pub mod kafka_wire;
pub mod kraft;
pub mod log_compaction;
pub mod mirror;
pub mod partitioned_topic;
pub mod protocol;
pub mod pulsar_admin;
pub mod pulsar_dispatch;
pub mod pulsar_topic;
pub mod pulsar_wire;
pub mod quota;
pub mod routes;
pub mod schema_evolution;
pub mod schema_registry;
pub mod segment_log;
pub mod server;
pub mod tenant;
pub mod tiered_storage;
pub mod transactions;
pub mod txn_markers;
pub mod unified_cursor;

use axum::Router;
use std::sync::Arc;

pub use broker::Broker;
pub use error::{StreamsError, StreamsResult};
pub use pulsar_admin::PulsarAdminCluster;

pub const MODULE_NAME: &str = "streams";
/// Default Kafka wire protocol port
pub const KAFKA_PORT: u16 = 9092;
/// Default Pulsar binary protocol port
pub const PULSAR_PORT: u16 = 6650;
/// Default Schema Registry port
pub const SCHEMA_REGISTRY_PORT: u16 = 8081;

/// Shared application state for the streams module.
pub struct StreamsState {
    pub broker: Arc<Broker>,
    pub pulsar_admin: Arc<PulsarAdminCluster>,
}

impl Default for StreamsState {
    fn default() -> Self {
        Self {
            broker: Arc::new(Broker::new(broker::BrokerConfig::default())),
            pulsar_admin: PulsarAdminCluster::new(),
        }
    }
}

/// Build Axum management/REST router (Schema Registry + Connect + admin endpoints).
pub fn router(state: Arc<StreamsState>) -> Router {
    routes::create_router(state)
}
