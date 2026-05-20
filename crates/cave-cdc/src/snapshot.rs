// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Snapshot + incremental snapshot mode.
//!
//! Cite: debezium-connector-common
//! `pipeline/source/spi/SnapshotChangeEventSource.java` +
//! debezium-connector-postgres `PostgresSnapshotChangeEventSource`. We
//! model the snapshot phase + the chunked incremental snapshot
//! algorithm described in the Debezium signal-table spec.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

/// Cite: debezium `SnapshotMode` enum — `initial` (default) /
/// `initial_only` / `never` / `when_needed` / `schema_only` /
/// `schema_only_recovery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    /// Cite: debezium docs `snapshot.mode = initial` — full snapshot
    /// then streaming.
    Initial,
    /// Cite: debezium docs `initial_only` — full snapshot, no streaming.
    InitialOnly,
    /// Cite: debezium docs `never` — start streaming from current LSN.
    Never,
    /// Cite: debezium docs `when_needed` — snapshot only if no offset
    /// committed yet.
    WhenNeeded,
    /// Cite: debezium docs `schema_only` — capture schema, skip data.
    SchemaOnly,
    /// Cite: debezium docs `schema_only_recovery` — emergency rebuild
    /// of the schema-history topic.
    SchemaOnlyRecovery,
}

impl SnapshotMode {
    pub fn captures_data(&self) -> bool {
        matches!(self, Self::Initial | Self::InitialOnly | Self::WhenNeeded)
    }

    pub fn streams_after(&self) -> bool {
        // InitialOnly and SchemaOnlyRecovery exit after the snapshot phase.
        matches!(
            self,
            Self::Initial | Self::Never | Self::WhenNeeded | Self::SchemaOnly
        )
    }

    pub fn parse(s: &str) -> CdcResult<Self> {
        match s.trim() {
            "initial" => Ok(Self::Initial),
            "initial_only" => Ok(Self::InitialOnly),
            "never" => Ok(Self::Never),
            "when_needed" => Ok(Self::WhenNeeded),
            "schema_only" => Ok(Self::SchemaOnly),
            "schema_only_recovery" => Ok(Self::SchemaOnlyRecovery),
            other => Err(CdcError::InvalidConfig(format!(
                "unknown snapshot mode '{}'",
                other
            ))),
        }
    }
}

/// Cite: debezium `IncrementalSnapshotContext` — chunk-based snapshot
/// that interleaves with streaming. Each chunk advances a watermark
/// over the table's primary-key range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotProgress {
    pub tenant_id: String,
    pub table_id: String,
    pub mode: SnapshotMode,
    /// Chunks already emitted (cite: debezium `IncrementalSnapshotContext.chunkId`).
    pub chunks_completed: u64,
    /// Total chunks the planner intends to emit. None ⇒ unbounded /
    /// not-yet-determined.
    pub chunks_total: Option<u64>,
    pub last_low_watermark: serde_json::Value,
    pub last_high_watermark: serde_json::Value,
    pub completed: bool,
}

impl SnapshotProgress {
    pub fn new(
        tenant_id: impl Into<String>,
        table_id: impl Into<String>,
        mode: SnapshotMode,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            table_id: table_id.into(),
            mode,
            chunks_completed: 0,
            chunks_total: None,
            last_low_watermark: serde_json::Value::Null,
            last_high_watermark: serde_json::Value::Null,
            completed: false,
        }
    }

    /// Cite: debezium `IncrementalSnapshotContext::nextChunkId` —
    /// chunks advance monotonically; an empty chunk does not bump the
    /// counter (the chunk is retried).
    pub fn complete_chunk(
        &mut self,
        low: serde_json::Value,
        high: serde_json::Value,
    ) -> CdcResult<()> {
        if self.completed {
            return Err(CdcError::InvalidConfig(format!(
                "snapshot for {} already completed",
                self.table_id,
            )));
        }
        self.last_low_watermark = low;
        self.last_high_watermark = high;
        self.chunks_completed += 1;
        if let Some(total) = self.chunks_total {
            if self.chunks_completed >= total {
                self.completed = true;
            }
        }
        Ok(())
    }

    pub fn mark_complete(&mut self) {
        self.completed = true;
    }

    pub fn percent(&self) -> Option<f64> {
        let total = self.chunks_total?;
        if total == 0 {
            return Some(100.0);
        }
        Some((self.chunks_completed as f64 / total as f64) * 100.0)
    }
}
