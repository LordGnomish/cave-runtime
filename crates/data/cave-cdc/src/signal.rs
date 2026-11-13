// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Debezium signal table — ad-hoc signals to a running connector.
//!
//! Cite: debezium docs "Sending signals to a Debezium connector"
//! (v3.5.0.Final) + `debezium-core`
//! `io.debezium.pipeline.signal.SignalRecord` +
//! `io.debezium.pipeline.signal.actions.ExecuteSnapshot` +
//! `io.debezium.pipeline.signal.actions.Log`.
//!
//! The signal table is a database table (`debezium_signal` by default)
//! that the application can INSERT into; the CDC connector polls it and
//! dispatches each row as a `Signal`. cave models the in-process side
//! of this polling: the connector pushes `Signal` records here as it
//! reads the table, and the pipeline drains them.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Cite: debezium `SignalRecord.type` — the signal type string.
/// v3.5.0.Final ships `execute-snapshot`, `stop-snapshot`,
/// `log`, and `schema-changes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalKind {
    /// Cite: debezium `ExecuteSnapshot` — triggers an incremental
    /// snapshot of the listed data-collections.
    ExecuteSnapshot,
    /// Cite: debezium `StopSnapshot` — stops an ongoing incremental
    /// snapshot.
    StopSnapshot,
    /// Cite: debezium `Log` — logs a message at INFO level and
    /// optionally emits a heartbeat.
    Log,
    /// Cite: debezium `SchemaChanges` signal — forces a schema-history
    /// entry for DDL that was not captured by the connector.
    SchemaChanges,
}

/// Cite: debezium `SignalRecord` — one row of the signal table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// Cite: `id` column — arbitrary unique identifier; the connector
    /// uses it for idempotent processing (same id → skip).
    pub id: String,
    pub signal_type: SignalKind,
    /// Cite: `data` column — JSON payload interpreted per `signal_type`.
    pub data: serde_json::Value,
}

impl Signal {
    pub fn validate(&self) -> CdcResult<()> {
        if self.id.trim().is_empty() {
            return Err(CdcError::InvalidConfig(
                "signal id must be non-empty".into(),
            ));
        }
        Ok(())
    }
}

/// In-process signal table. Cite: debezium
/// `io.debezium.pipeline.signal.SignalProcessor` — the processor
/// polls the signal table, deduplicates by `id`, and dispatches
/// pending signals in order. cave models the pending queue + the
/// seen-ids set so the pipeline can drain signals at will.
#[derive(Debug)]
pub struct SignalTable {
    pub tenant_id: String,
    /// Logical name matching the physical `<schema>.<table>` path.
    pub table_name: String,
    pending: Vec<Signal>,
    seen_ids: HashSet<String>,
}

impl SignalTable {
    pub fn new(tenant_id: impl Into<String>, table_name: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            table_name: table_name.into(),
            pending: Vec::new(),
            seen_ids: HashSet::new(),
        }
    }

    /// Push a signal row from the table-poll reader.  Duplicate `id`
    /// values are rejected idempotently per the debezium spec.
    pub fn push(&mut self, signal: Signal) -> CdcResult<()> {
        signal.validate()?;
        if !self.seen_ids.insert(signal.id.clone()) {
            return Err(CdcError::DuplicateOutboxEventId(signal.id.clone()));
        }
        self.pending.push(signal);
        Ok(())
    }

    /// Drain all pending signals. The caller (pipeline main loop)
    /// processes each one and advances the table read-cursor.
    pub fn drain(&mut self) -> Vec<Signal> {
        std::mem::take(&mut self.pending)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check whether a signal with this `id` was already processed.
    pub fn was_seen(&self, id: &str) -> bool {
        self.seen_ids.contains(id)
    }
}
