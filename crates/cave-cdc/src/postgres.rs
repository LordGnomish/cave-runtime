// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Postgres logical replication connector.
//!
//! Cite: debezium-connector-postgres v3.5.0.Final —
//! `PostgresConnector.java`,
//! `PostgresStreamingChangeEventSource.java`,
//! `connection/PostgresReplicationConnection.java`. cave builds an
//! in-memory model of the replication-slot lifecycle + the WAL event
//! shape so the rest of the pipeline (routing, sink, schema) can be
//! exercised without a live Postgres.

use crate::connector::{ConnectorState, SourceConnector};
use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

/// Cite: debezium-connector-postgres `LogicalDecodingMessageMonitor`
/// + connector config — Postgres logical-decoding plugins supported by
/// upstream are `pgoutput` (default) and `wal2json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecodingPlugin {
    Pgoutput,
    Wal2json,
}

/// Cite: debezium-connector-postgres `PostgresConnectorConfig` slot
/// + publication options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationSlotConfig {
    pub slot_name: String,
    pub publication_name: String,
    pub plugin: DecodingPlugin,
    pub drop_slot_on_stop: bool,
}

impl ReplicationSlotConfig {
    /// Cite: debezium-connector-postgres `PostgresConnector::validate`
    /// — slot + publication names must be lowercase identifiers
    /// (Postgres folds unquoted names) and ≤ 63 bytes (NAMEDATALEN-1).
    pub fn validate(&self) -> CdcResult<()> {
        for (label, name) in [
            ("slot_name", &self.slot_name),
            ("publication_name", &self.publication_name),
        ] {
            if name.is_empty() {
                return Err(CdcError::InvalidConfig(format!(
                    "{} must be non-empty",
                    label
                )));
            }
            if name.len() >= 64 {
                return Err(CdcError::InvalidConfig(format!(
                    "{} '{}' exceeds 63-byte Postgres identifier limit",
                    label, name,
                )));
            }
            if name != &name.to_lowercase() {
                return Err(CdcError::InvalidConfig(format!(
                    "{} '{}' must be lowercase (Postgres folds unquoted names)",
                    label, name,
                )));
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(CdcError::InvalidConfig(format!(
                    "{} '{}' may only contain [a-z0-9_]",
                    label, name,
                )));
            }
        }
        Ok(())
    }
}

/// Cite: debezium-connector-postgres `MessageType` — the subset of
/// pgoutput message types the streaming source dispatches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalEventKind {
    Begin,
    Commit,
    Insert,
    Update,
    Delete,
    Truncate,
    Relation,
    Type,
    Origin,
    Message,
}

/// LSN (Log Sequence Number) as a 64-bit unsigned. Cite: Postgres
/// `pg_lsn` — usually printed as `XXX/XXX` (high/low 32-bit halves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Lsn(pub u64);

impl Lsn {
    /// Cite: Postgres docs `pg_lsn` text format — uppercase hex pair
    /// joined by `/`. cave parses both `XXX/XXX` and bare hex.
    pub fn parse(s: &str) -> CdcResult<Self> {
        let trimmed = s.trim();
        let raw = if let Some((hi, lo)) = trimmed.split_once('/') {
            let hi = u32::from_str_radix(hi, 16)
                .map_err(|e| CdcError::InvalidLsn(s.to_string(), e.to_string()))?;
            let lo = u32::from_str_radix(lo, 16)
                .map_err(|e| CdcError::InvalidLsn(s.to_string(), e.to_string()))?;
            ((hi as u64) << 32) | (lo as u64)
        } else {
            u64::from_str_radix(trimmed, 16)
                .map_err(|e| CdcError::InvalidLsn(s.to_string(), e.to_string()))?
        };
        Ok(Self(raw))
    }

    pub fn as_text(&self) -> String {
        let hi = (self.0 >> 32) as u32;
        let lo = self.0 as u32;
        format!("{:X}/{:X}", hi, lo)
    }
}

/// Cite: debezium-connector-postgres `PostgresMessage` — the streaming
/// source dispatches on these fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalEvent {
    pub kind: WalEventKind,
    pub lsn: Lsn,
    pub xid: u32,
    pub schema: Option<String>,
    pub relation: Option<String>,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct PostgresConnector {
    pub name: String,
    pub tenant_id: String,
    pub db: String,
    pub slot_config: ReplicationSlotConfig,
    state: ConnectorState,
    /// Last LSN flushed back to the server (`StandbyStatusUpdate`).
    /// Cite: debezium-connector-postgres
    /// `PostgresReplicationConnection::sendStandbyStatusUpdate`.
    last_flushed_lsn: Lsn,
}

impl PostgresConnector {
    pub fn new(
        name: impl Into<String>,
        tenant_id: impl Into<String>,
        db: impl Into<String>,
        slot_config: ReplicationSlotConfig,
    ) -> Self {
        Self {
            name: name.into(),
            tenant_id: tenant_id.into(),
            db: db.into(),
            slot_config,
            state: ConnectorState::Initial,
            last_flushed_lsn: Lsn(0),
        }
    }

    /// Cite: debezium-connector-postgres
    /// `PostgresReplicationConnection::sendStandbyStatusUpdate` — the
    /// client periodically reports the LSN it has durably flushed so
    /// the server can recycle WAL.
    pub fn flush_lsn(&mut self, lsn: Lsn) -> CdcResult<()> {
        if self.state == ConnectorState::Initial {
            return Err(CdcError::NotConnected(self.name.clone()));
        }
        if lsn < self.last_flushed_lsn {
            return Err(CdcError::InvalidLsn(
                lsn.as_text(),
                format!(
                    "regression: requested {} < last_flushed {}",
                    lsn.as_text(),
                    self.last_flushed_lsn.as_text(),
                ),
            ));
        }
        self.last_flushed_lsn = lsn;
        Ok(())
    }

    pub fn last_flushed_lsn(&self) -> Lsn {
        self.last_flushed_lsn
    }
}

impl SourceConnector for PostgresConnector {
    fn name(&self) -> &str {
        &self.name
    }
    fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
    fn state(&self) -> ConnectorState {
        self.state
    }

    fn validate(&self) -> CdcResult<()> {
        if self.tenant_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig(
                "tenant_id must be non-empty".into(),
            ));
        }
        if self.db.trim().is_empty() {
            return Err(CdcError::InvalidConfig("db must be non-empty".into()));
        }
        self.slot_config.validate()
    }

    fn start(&mut self) -> CdcResult<()> {
        if self.state == ConnectorState::Streaming {
            return Err(CdcError::AlreadyRunning);
        }
        self.validate()?;
        if !self.state.can_transition_to(ConnectorState::Snapshotting) {
            return Err(CdcError::InvalidConfig(format!(
                "cannot transition from {:?} to Snapshotting",
                self.state,
            )));
        }
        self.state = ConnectorState::Snapshotting;
        // Promote to Streaming once the snapshot phase exits in
        // tests/operational reality.
        self.state = ConnectorState::Streaming;
        Ok(())
    }

    fn stop(&mut self) -> CdcResult<()> {
        if self.state == ConnectorState::Initial {
            return Ok(()); // idempotent
        }
        self.state = ConnectorState::Stopped;
        Ok(())
    }
}
