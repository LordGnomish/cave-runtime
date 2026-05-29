// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schema history — DDL journal for connector restart recovery.
//!
//! Cite: debezium-storage
//! `io.debezium.storage.SchemaHistory` interface +
//! `io.debezium.relational.history.MemorySchemaHistory` +
//! `io.debezium.relational.history.HistoryRecord`.
//!
//! Every time a connector detects a DDL change (ALTER TABLE, CREATE
//! TABLE, …) it appends a `HistoryRecord` to the schema history store.
//! On restart the connector replays the full history to reconstruct
//! the latest table structure before resuming streaming.
//!
//! cave's in-memory implementation mirrors `MemorySchemaHistory`
//! but adds tenant-isolation guards and a JSON serialisation path so
//! operators can snapshot the store.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

/// Source position at which the DDL was observed.  Cite: debezium
/// `HistoryRecord.Fields.SOURCE` — a nested map carrying at minimum
/// the connector name and the position (LSN / binlog offset / etc.).
/// cave uses a trimmed struct instead of an opaque map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySource {
    pub connector: String,
    pub db: String,
    pub schema: String,
    /// Millisecond epoch timestamp of the DDL event.  Cite: debezium
    /// `HistoryRecord.Fields.TIMESTAMP`.
    pub ts_ms: i64,
}

/// One DDL event in the schema history.  Cite: debezium
/// `HistoryRecord` — carries `source` (position), `position`
/// (struct), `ddl` (the raw DDL string), and `tableChanges`
/// (list of affected `<schema>.<table>` identifiers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub tenant_id: String,
    pub source: HistorySource,
    /// Raw DDL statement.  Cite: `HistoryRecord.Fields.DDL`.
    pub ddl: String,
    /// Affected table identifiers (`<schema>.<table>`).  Cite:
    /// `HistoryRecord.Fields.TABLE_CHANGES`.
    pub table_changes: Vec<String>,
}

impl HistoryRecord {
    pub fn validate(&self) -> CdcResult<()> {
        if self.tenant_id.trim().is_empty() {
            return Err(CdcError::InvalidConfig(
                "HistoryRecord tenant_id must be non-empty".into(),
            ));
        }
        if self.ddl.trim().is_empty() {
            return Err(CdcError::InvalidConfig(
                "HistoryRecord ddl must be non-empty".into(),
            ));
        }
        Ok(())
    }
}

/// In-memory schema history.  Cite: debezium
/// `MemorySchemaHistory` — appends records to a `List` and replays
/// them in insertion order on `recover()`. cave adds tenant isolation
/// and per-table / per-timestamp query helpers.
#[derive(Debug)]
pub struct SchemaHistory {
    pub tenant_id: String,
    records: Vec<HistoryRecord>,
}

impl SchemaHistory {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            records: Vec::new(),
        }
    }

    /// Append a DDL record.  Cite: debezium `SchemaHistory::record` —
    /// the connector calls this each time it sees a DDL event.
    pub fn record(&mut self, rec: HistoryRecord) -> CdcResult<()> {
        rec.validate()?;
        if rec.tenant_id != self.tenant_id {
            return Err(CdcError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: rec.tenant_id.clone(),
            });
        }
        self.records.push(rec);
        Ok(())
    }

    /// Total number of DDL events recorded.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Return all records that mention `table` in their `table_changes`
    /// list.  Cite: debezium `SchemaHistory::recover` — on restart the
    /// connector calls this to replay only the DDL relevant to each
    /// table in its filter list.
    pub fn records_for_table(&self, table: &str) -> Vec<&HistoryRecord> {
        self.records
            .iter()
            .filter(|r| r.table_changes.iter().any(|t| t == table))
            .collect()
    }

    /// Return all records whose `source.ts_ms` is strictly greater
    /// than `since_ts`.  Useful for resuming after a known checkpoint.
    pub fn records_since_ts(&self, since_ts: i64) -> Vec<&HistoryRecord> {
        self.records
            .iter()
            .filter(|r| r.source.ts_ms > since_ts)
            .collect()
    }

    /// Ordered replay of all records.  Cite: debezium
    /// `SchemaHistory::recover(HistoryRecord.Comparator)` — the
    /// connector streams through the history in insertion order to
    /// rebuild the table structures.
    pub fn all_records(&self) -> &[HistoryRecord] {
        &self.records
    }

    /// Serialize the full history to a JSON array for persistence.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.records).unwrap_or(serde_json::Value::Array(vec![]))
    }
}
