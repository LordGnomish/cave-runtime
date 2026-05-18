// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-cdc — Debezium-parity Change Data Capture pipeline.
//!
//! Sources:
//! * Postgres — logical replication via `pgoutput` / `wal2json`
//! * MySQL    — binary log (binlog) row-based events
//! * MongoDB  — change streams (oplog tail)
//! * Outbox   — transactional outbox table polling + dedupe
//!
//! Sink: cave-streams producer (no Kafka Connect intermediary). Topic
//! routing is tenant-scoped; schema evolution covers Avro/Protobuf/JSON.
//!
//! Pinned to debezium/debezium v3.5.0.Final (latest stable, 2026-03-31).

pub mod connector;
pub mod error;
pub mod mongo;
pub mod mysql;
pub mod outbox;
pub mod postgres;
pub mod routing;
pub mod schema;
pub mod snapshot;
pub mod streams_sink;

pub use connector::{
    ChangeEvent, ChangeOperation, ConnectorState, SourceConnector, SourceMetadata, SourceRecord,
};
pub use error::{CdcError, CdcResult};
pub use mongo::{MongoDbConnector, OplogEvent, OplogOp};
pub use mysql::{BinlogEvent, BinlogEventType, MySqlConnector};
pub use outbox::{OutboxEntry, OutboxEventRouter};
pub use postgres::{PostgresConnector, ReplicationSlotConfig, WalEvent, WalEventKind};
pub use routing::{RoutingPolicy, TopicRouter};
pub use schema::{Compatibility, Schema, SchemaFormat, SchemaRegistry};
pub use snapshot::{SnapshotMode, SnapshotProgress};
pub use streams_sink::{StreamsSink, ProduceResult};

pub const MODULE_NAME: &str = "cdc";
