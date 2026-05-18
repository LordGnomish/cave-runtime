// SPDX-License-Identifier: AGPL-3.0-or-later
//! MySQL binlog connector.
//!
//! Cite: debezium-connector-mysql v3.5.0.Final
//! `MySqlConnector.java`, `MySqlStreamingChangeEventSource.java`. We
//! parse the canonical binlog event header + map the row-event types
//! onto our `BinlogEventType` enum.

use crate::connector::{ConnectorState, SourceConnector};
use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

/// Cite: MySQL `Log_event_type` enum — the subset relevant to CDC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinlogEventType {
    /// `WRITE_ROWS_EVENT` (type code 30 in MySQL 5.6+).
    WriteRows,
    /// `UPDATE_ROWS_EVENT` (type code 31).
    UpdateRows,
    /// `DELETE_ROWS_EVENT` (type code 32).
    DeleteRows,
    /// `QUERY_EVENT` (DDL).
    Query,
    /// `XID_EVENT` (transaction commit marker).
    Xid,
    /// `TABLE_MAP_EVENT` (relation metadata).
    TableMap,
    /// `GTID_EVENT` (Global Transaction Identifier).
    Gtid,
    /// `ROTATE_EVENT` (binlog file rotation).
    Rotate,
    /// `FORMAT_DESCRIPTION_EVENT` (header at file start).
    FormatDescription,
}

impl BinlogEventType {
    /// Cite: MySQL `log_event.h::Log_event_type` numeric codes.
    pub fn type_code(&self) -> u8 {
        match self {
            Self::Query             => 2,
            Self::Rotate            => 4,
            Self::FormatDescription => 15,
            Self::Xid               => 16,
            Self::TableMap          => 19,
            Self::WriteRows         => 30,
            Self::UpdateRows        => 31,
            Self::DeleteRows        => 32,
            Self::Gtid              => 33,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            2  => Some(Self::Query),
            4  => Some(Self::Rotate),
            15 => Some(Self::FormatDescription),
            16 => Some(Self::Xid),
            19 => Some(Self::TableMap),
            30 => Some(Self::WriteRows),
            31 => Some(Self::UpdateRows),
            32 => Some(Self::DeleteRows),
            33 => Some(Self::Gtid),
            _  => None,
        }
    }

    /// Cite: debezium-connector-mysql `MySqlStreamingChangeEventSource`
    /// — only the three row-event variants emit ChangeEvents on the
    /// downstream pipeline; the rest are bookkeeping.
    pub fn is_row_event(&self) -> bool {
        matches!(self, Self::WriteRows | Self::UpdateRows | Self::DeleteRows)
    }
}

/// Position inside a single binlog file. Cite: MySQL `SHOW MASTER
/// STATUS` output (`File`, `Position`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinlogPosition {
    pub file: String,
    pub pos: u64,
}

impl BinlogPosition {
    /// Cite: MySQL binlog file naming `<basename>.<6-digit-seq>` (e.g.
    /// `mysql-bin.000123`). cave validates the suffix is purely digits.
    pub fn validate(&self) -> CdcResult<()> {
        if self.file.is_empty() {
            return Err(CdcError::InvalidBinlogPosition {
                file: self.file.clone(), pos: self.pos,
            });
        }
        let Some(dot_idx) = self.file.rfind('.') else {
            return Err(CdcError::InvalidBinlogPosition {
                file: self.file.clone(), pos: self.pos,
            });
        };
        let suffix = &self.file[dot_idx + 1..];
        if !suffix.chars().all(|c| c.is_ascii_digit()) || suffix.is_empty() {
            return Err(CdcError::InvalidBinlogPosition {
                file: self.file.clone(), pos: self.pos,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinlogEvent {
    pub kind: BinlogEventType,
    pub server_id: u32,
    pub timestamp_secs: u32,
    pub position: BinlogPosition,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub gtid: Option<String>,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct MySqlConnector {
    pub name: String,
    pub tenant_id: String,
    pub db: String,
    pub include_schemas: Vec<String>,
    pub server_id: u32,
    state: ConnectorState,
    /// Cite: debezium-connector-mysql
    /// `MySqlStreamingChangeEventSource::sendHeartbeat` — checkpoint
    /// position the connector has acknowledged.
    last_committed_position: BinlogPosition,
    /// Cite: debezium-connector-mysql `GtidSet`. cave keeps the latest
    /// GTID seen so restarts can resume from the correct point.
    last_gtid: Option<String>,
}

impl MySqlConnector {
    pub fn new(
        name: impl Into<String>,
        tenant_id: impl Into<String>,
        db: impl Into<String>,
        server_id: u32,
    ) -> Self {
        Self {
            name: name.into(),
            tenant_id: tenant_id.into(),
            db: db.into(),
            include_schemas: Vec::new(),
            server_id,
            state: ConnectorState::Initial,
            last_committed_position: BinlogPosition { file: String::new(), pos: 0 },
            last_gtid: None,
        }
    }

    /// Cite: debezium-connector-mysql `MySqlStreamingChangeEventSource`
    /// — the connector emits a downstream record only for events whose
    /// schema is in `include_schemas` (when non-empty).
    pub fn should_emit(&self, schema: &str) -> bool {
        self.include_schemas.is_empty()
            || self.include_schemas.iter().any(|s| s == schema)
    }

    pub fn record_position(&mut self, pos: BinlogPosition, gtid: Option<String>) -> CdcResult<()> {
        pos.validate()?;
        self.last_committed_position = pos;
        if let Some(g) = gtid { self.last_gtid = Some(g); }
        Ok(())
    }

    pub fn last_committed_position(&self) -> &BinlogPosition { &self.last_committed_position }
    pub fn last_gtid(&self) -> Option<&str> { self.last_gtid.as_deref() }
}

impl SourceConnector for MySqlConnector {
    fn name(&self) -> &str { &self.name }
    fn tenant_id(&self) -> &str { &self.tenant_id }
    fn state(&self) -> ConnectorState { self.state }

    fn validate(&self) -> CdcResult<()> {
        if self.tenant_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig("tenant_id must be non-empty".into()));
        }
        if self.server_id == 0 {
            return Err(CdcError::InvalidConfig(
                "MySQL server_id must be != 0 (replication identity)".into()));
        }
        Ok(())
    }

    fn start(&mut self) -> CdcResult<()> {
        if self.state == ConnectorState::Streaming {
            return Err(CdcError::AlreadyRunning);
        }
        self.validate()?;
        self.state = ConnectorState::Streaming;
        Ok(())
    }

    fn stop(&mut self) -> CdcResult<()> {
        self.state = ConnectorState::Stopped;
        Ok(())
    }
}
