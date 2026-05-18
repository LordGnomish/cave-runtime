// SPDX-License-Identifier: AGPL-3.0-or-later
//! Source connector trait and shared types.
//!
//! Cite: debezium-connector-common
//! `pipeline/source/spi/StreamingChangeEventSource.java` (streaming
//! source) + `SnapshotChangeEventSource.java` (snapshot source) +
//! `BaseSourceTask.java` (lifecycle), debezium-api
//! `engine/DebeziumEngine.java` (engine façade).

use crate::error::{CdcError, CdcResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Cite: debezium `Envelope.Operation` — `c` create, `u` update,
/// `d` delete, `r` snapshot read, `t` truncate, `m` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeOperation {
    Create,
    Update,
    Delete,
    Read,
    Truncate,
    Message,
}

impl ChangeOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "c", Self::Update => "u", Self::Delete => "d",
            Self::Read   => "r", Self::Truncate => "t", Self::Message => "m",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "c" | "create"   => Some(Self::Create),
            "u" | "update"   => Some(Self::Update),
            "d" | "delete"   => Some(Self::Delete),
            "r" | "read"     => Some(Self::Read),
            "t" | "truncate" => Some(Self::Truncate),
            "m" | "message"  => Some(Self::Message),
            _ => None,
        }
    }
}

/// Cite: debezium `SourceInfoStructMaker` — connector-agnostic source
/// metadata that lands in the envelope's `source` field. cave keeps a
/// trimmed view that all three connectors populate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub connector: String,
    pub name: String,
    pub tenant_id: String,
    pub db: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub ts_ms: i64,
    /// Snapshot phase: `false` / `true` / `last`. Cite: debezium
    /// `SnapshotRecord` enum.
    pub snapshot: Option<String>,
}

/// Cite: debezium `Envelope.FieldName` + envelope JSON shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub op: ChangeOperation,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub source: SourceMetadata,
    pub ts_ms: i64,
    pub transaction: Option<TransactionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionMetadata {
    pub id: String,
    pub total_order: u64,
    pub data_collection_order: u64,
}

/// Cite: debezium `SourceRecord` (Kafka Connect API). cave's
/// equivalent envelope keys the record by tenant + topic + partition
/// + key bytes, and ships an opaque value blob (already serialised to
/// the negotiated wire format).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub tenant_id: String,
    pub topic: String,
    pub partition: i32,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub source_ts_ms: i64,
    pub created_at: DateTime<Utc>,
}

/// Cite: debezium `BaseSourceTask::State` — Initial / Running /
/// Stopped. cave adds `Snapshotting` to make the snapshot phase
/// visible to control-plane observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectorState {
    Initial,
    Snapshotting,
    Streaming,
    Stopped,
    Failed,
}

impl ConnectorState {
    /// Cite: debezium `BaseSourceTask` start/stop guard — calling
    /// `start()` on a Running connector is a no-op; `stop()` from
    /// Initial is also a no-op. The forward path is
    /// Initial → Snapshotting → Streaming → Stopped (or Failed sink).
    pub fn can_transition_to(self, next: ConnectorState) -> bool {
        use ConnectorState::*;
        match (self, next) {
            (Initial, Snapshotting)       => true,
            (Initial, Streaming)          => true, // skip-snapshot mode
            (Snapshotting, Streaming)     => true,
            (Snapshotting, Stopped)       => true,
            (Streaming, Stopped)          => true,
            (Initial | Snapshotting | Streaming, Failed) => true,
            (a, b) if a == b              => true, // self-loop (idempotent)
            _ => false,
        }
    }
}

/// Common contract every cave-cdc source connector implements. Cite:
/// debezium `Connector` + `BaseSourceTask`.
pub trait SourceConnector {
    fn name(&self) -> &str;
    fn tenant_id(&self) -> &str;
    fn state(&self) -> ConnectorState;
    fn validate(&self) -> CdcResult<()>;
    /// Cite: debezium `BaseSourceTask::doStart`.
    fn start(&mut self) -> CdcResult<()>;
    /// Cite: debezium `BaseSourceTask::doStop`.
    fn stop(&mut self) -> CdcResult<()>;
}

/// Helper: tenant-scope guard for stores that hold per-tenant state.
pub fn require_tenant(store_tenant: &str, requesting_tenant: &str) -> CdcResult<()> {
    if store_tenant != requesting_tenant {
        return Err(CdcError::CrossTenantDenied {
            store: store_tenant.to_string(),
            req: requesting_tenant.to_string(),
        });
    }
    Ok(())
}
