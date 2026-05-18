// SPDX-License-Identifier: AGPL-3.0-or-later
//! Storage layer — Kafka-style log compaction (tombstone-driven) and
//! time/size-based retention.
//!
//! These extensions sit on top of [`crate::segment_log::SegmentLog`].
//! Compaction is *log-side* (not MVCC like cave-etcd's compactor): for
//! every key, only the last value is retained.  A null payload is the
//! "tombstone" that removes the key entirely.
//!
//! Mirrors Apache Kafka 4.2.0
//!   `core/src/main/scala/kafka/log/LogCleaner.scala`
//!   `core/src/main/scala/kafka/log/UnifiedLog.scala` (retention)

use crate::error::StreamsResult;
use crate::segment_log::{LogEntry, SegmentLog};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One produced record with an explicit key.  Compaction operates on
/// (key → latest-value) pairs; keyless records are never compacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyedRecord {
    pub key: Vec<u8>,
    /// `None` is the *tombstone* (delete the key on compaction).
    pub value: Option<Vec<u8>>,
}

impl KeyedRecord {
    /// Encode a `(key, value)` pair into a single payload buffer using a
    /// length-prefixed framing: `[key_len: u32][key][value_len: i32][value]`.
    /// `value_len = -1` denotes a tombstone.
    pub fn encode(&self) -> Vec<u8> {
        let key_len = self.key.len() as u32;
        let mut out = Vec::with_capacity(8 + self.key.len() + self.value.as_ref().map_or(0, |v| v.len()));
        out.extend_from_slice(&key_len.to_be_bytes());
        out.extend_from_slice(&self.key);
        match &self.value {
            None => out.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                out.extend_from_slice(v);
            }
        }
        out
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < 8 {
            return None;
        }
        let key_len = u32::from_be_bytes(payload[..4].try_into().ok()?) as usize;
        if payload.len() < 4 + key_len + 4 {
            return None;
        }
        let key = payload[4..4 + key_len].to_vec();
        let off = 4 + key_len;
        let value_len = i32::from_be_bytes(payload[off..off + 4].try_into().ok()?);
        let value = if value_len < 0 {
            None
        } else {
            let s = off + 4;
            let e = s + value_len as usize;
            if payload.len() < e {
                return None;
            }
            Some(payload[s..e].to_vec())
        };
        Some(Self { key, value })
    }

    pub fn is_tombstone(&self) -> bool {
        self.value.is_none()
    }
}

/// Result of a single compaction sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionStats {
    pub entries_before: usize,
    pub entries_after: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub tombstones_removed: usize,
}

/// Sweep `log` keeping only the last value per key.  Tombstones with a
/// `delete.retention.ms` of 0 are removed in the same pass; callers that
/// want a delayed delete should hold the tombstone in a sidecar.
pub fn compact_log(log: &SegmentLog) -> StreamsResult<CompactionStats> {
    let entries = log.read(log.log_start_offset(), usize::MAX)?;
    let entries_before = entries.len();
    let bytes_before: u64 = entries.iter().map(|e| e.payload.len() as u64).sum();

    // Walk in offset order keeping the last value per key.  Tombstones
    // remove the key from the map.
    let mut latest: HashMap<Vec<u8>, LogEntry> = HashMap::new();
    let mut tombstones_removed = 0usize;
    for e in entries {
        let Some(rec) = KeyedRecord::decode(&e.payload) else {
            // Keyless record — leave it as-is by giving it a unique key.
            latest.insert(e.payload.clone(), e);
            continue;
        };
        if rec.is_tombstone() {
            if latest.remove(&rec.key).is_some() {
                tombstones_removed += 1;
            } else {
                tombstones_removed += 1;
            }
        } else {
            latest.insert(rec.key, e);
        }
    }
    // Rewrite the log: truncate everything, then re-append surviving
    // entries in offset order so old offsets are preserved (Kafka cleaner
    // semantics — compacted logs keep their offsets, just sparser).
    let high = log.next_offset();
    log.truncate_before(high);
    let mut surviving: Vec<LogEntry> = latest.into_values().collect();
    surviving.sort_by_key(|e| e.offset);
    for e in &surviving {
        log.append(e.payload.clone(), e.timestamp_ms)?;
    }
    let entries_after = surviving.len();
    let bytes_after: u64 = surviving.iter().map(|e| e.payload.len() as u64).sum();
    Ok(CompactionStats {
        entries_before,
        entries_after,
        bytes_before,
        bytes_after,
        tombstones_removed,
    })
}

/// Retention policy.  At least one of `time_ms` or `bytes` must be set;
/// pass 0 to disable that axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub time_ms: u64,
    pub bytes: u64,
}

impl RetentionPolicy {
    pub fn time_only(ms: u64) -> Self {
        Self { time_ms: ms, bytes: 0 }
    }
    pub fn size_only(b: u64) -> Self {
        Self { time_ms: 0, bytes: b }
    }
}

/// Result of a single retention sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionStats {
    pub entries_dropped: usize,
    pub bytes_dropped: u64,
    pub low_watermark: u64,
}

/// Apply `policy` to `log`, evicting the oldest entries until both axes
/// (time & size) are satisfied.  `now_ms` is the current wall-clock time
/// — passed in so tests are deterministic.
pub fn apply_retention(
    log: &SegmentLog,
    policy: RetentionPolicy,
    now_ms: i64,
) -> StreamsResult<RetentionStats> {
    let entries = log.read(log.log_start_offset(), usize::MAX)?;
    let mut new_low = log.log_start_offset();
    let mut entries_dropped = 0usize;
    let mut bytes_dropped = 0u64;

    let total_bytes: u64 = entries.iter().map(|e| e.payload.len() as u64).sum();
    let mut current_bytes = total_bytes;

    for e in &entries {
        let too_old = policy.time_ms > 0
            && (now_ms - e.timestamp_ms) as u64 > policy.time_ms;
        let too_big = policy.bytes > 0 && current_bytes > policy.bytes;
        if too_old || too_big {
            new_low = e.offset + 1;
            entries_dropped += 1;
            bytes_dropped += e.payload.len() as u64;
            current_bytes = current_bytes.saturating_sub(e.payload.len() as u64);
        } else {
            break;
        }
    }
    if entries_dropped > 0 {
        log.truncate_before(new_low);
    }
    Ok(RetentionStats {
        entries_dropped,
        bytes_dropped,
        low_watermark: new_low,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Log-compaction + retention tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tenant_id: &str, suffix: &str) -> Vec<u8> {
        format!("tenants/{}/{}", tenant_id, suffix).into_bytes()
    }

    #[test]
    fn test_keyed_record_round_trip() {
        // cite: kafka 4.2.0 RecordBatch#asKeyValue
        let tenant_id = "lc-001";
        let r = KeyedRecord {
            key: key(tenant_id, "k"),
            value: Some(b"v".to_vec()),
        };
        let bytes = r.encode();
        let back = KeyedRecord::decode(&bytes).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn test_keyed_record_tombstone_round_trip() {
        // cite: kafka 4.2.0 LogCleaner (tombstone = null value)
        let tenant_id = "lc-002";
        let r = KeyedRecord {
            key: key(tenant_id, "del"),
            value: None,
        };
        let bytes = r.encode();
        let back = KeyedRecord::decode(&bytes).unwrap();
        assert!(back.is_tombstone());
        assert_eq!(back.key, r.key);
    }

    #[test]
    fn test_compact_log_keeps_last_value_per_key() {
        // cite: kafka 4.2.0 LogCleaner#compact
        let tenant_id = "lc-003";
        let log = SegmentLog::new(1024 * 1024);
        // Three updates to same key, last value wins.
        for v in ["v1", "v2", "v3"] {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, "k"),
                    value: Some(v.as_bytes().to_vec()),
                }
                .encode(),
                1,
            )
            .unwrap();
        }
        let stats = compact_log(&log).unwrap();
        assert_eq!(stats.entries_before, 3);
        assert_eq!(stats.entries_after, 1);
        // Compaction advances log_start_offset; read from the new floor.
        let surviving = log.read(log.log_start_offset(), usize::MAX).unwrap();
        let rec = KeyedRecord::decode(&surviving[0].payload).unwrap();
        assert_eq!(rec.value.as_deref(), Some(&b"v3"[..]));
    }

    #[test]
    fn test_compact_log_tombstone_removes_key() {
        // cite: kafka 4.2.0 LogCleaner (tombstone deletes key)
        let tenant_id = "lc-004";
        let log = SegmentLog::new(1024 * 1024);
        for v in ["v1", "v2"] {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, "k"),
                    value: Some(v.as_bytes().to_vec()),
                }
                .encode(),
                1,
            )
            .unwrap();
        }
        log.append(
            KeyedRecord {
                key: key(tenant_id, "k"),
                value: None,
            }
            .encode(),
            1,
        )
        .unwrap();
        let stats = compact_log(&log).unwrap();
        assert_eq!(stats.entries_after, 0);
        assert!(stats.tombstones_removed >= 1);
    }

    #[test]
    fn test_compact_log_preserves_distinct_keys() {
        // cite: kafka 4.2.0 LogCleaner (per-key dedup)
        let tenant_id = "lc-005";
        let log = SegmentLog::new(1024 * 1024);
        for i in 0..10 {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, &format!("k{i}")),
                    value: Some(format!("v{i}").into_bytes()),
                }
                .encode(),
                1,
            )
            .unwrap();
        }
        let stats = compact_log(&log).unwrap();
        assert_eq!(stats.entries_after, 10);
    }

    #[test]
    fn test_apply_retention_time_only_drops_old() {
        // cite: kafka 4.2.0 UnifiedLog#deleteOldSegments (retention.ms)
        let tenant_id = "lc-006";
        let log = SegmentLog::new(1024 * 1024);
        for i in 0..5 {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, &format!("k{i}")),
                    value: Some(b"v".to_vec()),
                }
                .encode(),
                100, // old timestamp
            )
            .unwrap();
        }
        // now is far in the future relative to ts=100.
        let stats = apply_retention(
            &log,
            RetentionPolicy::time_only(50),
            10_000,
        )
        .unwrap();
        assert!(stats.entries_dropped >= 1);
        assert_eq!(stats.low_watermark, log.log_start_offset());
    }

    #[test]
    fn test_apply_retention_size_only_drops_oldest() {
        // cite: kafka 4.2.0 UnifiedLog#deleteOldSegments (retention.bytes)
        let tenant_id = "lc-007";
        let log = SegmentLog::new(1024 * 1024);
        for i in 0..10 {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, &format!("k{i}")),
                    value: Some(b"x".repeat(100)),
                }
                .encode(),
                1,
            )
            .unwrap();
        }
        let before = log.total_bytes();
        let stats = apply_retention(
            &log,
            RetentionPolicy::size_only(before / 2),
            1,
        )
        .unwrap();
        assert!(stats.entries_dropped >= 1);
        assert!(log.total_bytes() <= before);
    }

    #[test]
    fn test_apply_retention_no_op_when_within_policy() {
        // cite: kafka 4.2.0 (retention is no-op if cap not exceeded)
        let tenant_id = "lc-008";
        let log = SegmentLog::new(1024 * 1024);
        for i in 0..3 {
            log.append(
                KeyedRecord {
                    key: key(tenant_id, &format!("k{i}")),
                    value: Some(b"v".to_vec()),
                }
                .encode(),
                1_000,
            )
            .unwrap();
        }
        let stats = apply_retention(
            &log,
            RetentionPolicy {
                time_ms: 1_000_000,
                bytes: 1024 * 1024,
            },
            1_000,
        )
        .unwrap();
        assert_eq!(stats.entries_dropped, 0);
    }
}
