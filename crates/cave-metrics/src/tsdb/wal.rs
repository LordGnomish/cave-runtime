// SPDX-License-Identifier: AGPL-3.0-or-later
//! TSDB write-ahead log — thin domain wrapper over cave-core::wal.
//!
//! `WalRecord` owns the Prometheus-specific entry shape.  The actual
//! append/replay machinery lives in `cave_core::wal::{AppendLog, replay}`.

use std::path::Path;
use serde::{Deserialize, Serialize};

pub use cave_core::wal::{AppendLog, WalError, WalResult, replay};

use crate::model::{Labels, Sample};

// ── Domain record type ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub enum WalRecord {
    Sample {
        labels: std::collections::BTreeMap<String, String>,
        timestamp_ms: i64,
        value: f64,
    },
    Checkpoint {
        ts: i64,
    },
}

// ── Convenience helpers ───────────────────────────────────────────────────────

/// Build a `WalRecord::Sample` from domain types and append it to the log.
pub fn append_sample(
    log: &mut AppendLog,
    labels: &Labels,
    sample: &Sample,
) -> WalResult<()> {
    let record = WalRecord::Sample {
        labels: labels.0.clone(),
        timestamp_ms: sample.timestamp_ms,
        value: sample.value,
    };
    log.append(&record)
}

/// Replay all `WalRecord`s from `path`, calling `f` for each valid entry.
pub fn replay_records<F: FnMut(WalRecord)>(
    path: impl AsRef<Path>,
    f: F,
) -> WalResult<()> {
    replay::<WalRecord, F>(path, f)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_wal_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut log = AppendLog::open(path).unwrap();
            let labels = Labels::from_pairs([("__name__", "cpu"), ("job", "test")]);
            append_sample(&mut log, &labels, &Sample::new(1000, 0.5)).unwrap();
            append_sample(&mut log, &labels, &Sample::new(2000, 0.6)).unwrap();
        }

        let mut records = Vec::new();
        replay_records(path, |rec| records.push(rec)).unwrap();
        assert_eq!(records.len(), 2);
    }
}
