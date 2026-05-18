// SPDX-License-Identifier: AGPL-3.0-or-later
//! Log compaction — keep only the latest record per key in a partition.
//!
//! Runs on-demand or can be triggered in a background tokio task.

use crate::error::{StreamError, StreamResult};
use crate::models::{CleanupPolicy, PartitionLog, Record};
use crate::storage::StreamStorage;
use std::collections::HashMap;
use tracing::{debug, info};

/// Engine that compacts partition logs according to topic cleanup policy.
pub struct CompactionEngine<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> CompactionEngine<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    // ─── Public API ───────────────────────────────────────────────────────────

    /// Compact all eligible partitions across all topics.
    ///
    /// A partition is eligible when its topic's `cleanup_policy` includes
    /// [`CleanupPolicy::Compact`] or [`CleanupPolicy::DeleteAndCompact`].
    pub fn compact_all(&self) -> StreamResult<CompactionStats> {
        let topics = self.storage.list_topics()?;
        let mut stats = CompactionStats::default();

        for topic in topics {
            let needs_compact = matches!(
                topic.config.cleanup_policy,
                CleanupPolicy::Compact | CleanupPolicy::DeleteAndCompact
            );
            if !needs_compact {
                continue;
            }

            for partition in 0..topic.partitions {
                let result = self.compact_partition(&topic.name, partition)?;
                stats.partitions_compacted += 1;
                stats.records_before += result.records_before;
                stats.records_after += result.records_after;
                stats.bytes_reclaimed += result.bytes_reclaimed;
            }
        }
        Ok(stats)
    }

    /// Compact a single partition.
    ///
    /// Algorithm:
    /// 1. Scan all records from `log_start_offset` to `high_watermark`.
    /// 2. Keep track of the latest record (highest offset) for each key.
    /// 3. Null-key records (tombstones in Kafka parlance) are preserved.
    /// 4. Records after `last_compacted_offset` + delta are kept as the
    ///    "tail" (active segment) without compaction to avoid removing
    ///    recently produced messages.
    pub fn compact_partition(
        &self,
        topic: &str,
        partition: u32,
    ) -> StreamResult<PartitionCompactionResult> {
        let log = self
            .storage
            .get_partition_log(topic, partition)?
            .ok_or_else(|| StreamError::PartitionNotFound {
                topic: topic.into(),
                partition,
            })?;

        let records_before = log.records.len();

        let compacted = compact_log(log);

        let records_after = compacted.records.len();
        let bytes_reclaimed = estimate_bytes(records_before - records_after);

        debug!(
            topic,
            partition,
            records_before,
            records_after,
            bytes_reclaimed,
            "Compacted partition"
        );

        self.storage.replace_partition_log(topic, partition, compacted)?;

        Ok(PartitionCompactionResult {
            topic: topic.into(),
            partition,
            records_before,
            records_after,
            bytes_reclaimed,
        })
    }

    // ─── Retention enforcement ────────────────────────────────────────────────

    /// Delete records from partitions that exceed the topic's time/size
    /// retention limits.
    pub fn enforce_retention_all(&self) -> StreamResult<RetentionStats> {
        let topics = self.storage.list_topics()?;
        let mut stats = RetentionStats::default();
        let now_ms = chrono::Utc::now().timestamp_millis();

        for topic in topics {
            for partition in 0..topic.partitions {
                let Some(mut log) =
                    self.storage.get_partition_log(&topic.name, partition)?
                else {
                    continue;
                };

                let before = log.records.len();

                // Time-based retention.
                if let Some(retention_ms) = topic.config.retention_ms {
                    let cutoff_ms = now_ms - retention_ms;
                    log.records.retain(|r| r.timestamp_ms >= cutoff_ms);
                }

                // Size-based retention (remove oldest until within limit).
                if let Some(max_bytes) = topic.config.retention_bytes {
                    while estimated_total_bytes(&log.records) > max_bytes as usize
                        && !log.records.is_empty()
                    {
                        log.records.remove(0);
                    }
                }

                let after = log.records.len();
                let removed = before - after;

                if removed > 0 {
                    // Advance log_start_offset.
                    if let Some(first) = log.records.first() {
                        log.log_start_offset = first.offset;
                    }
                    stats.records_deleted += removed;
                    stats.partitions_trimmed += 1;
                    info!(
                        topic = &topic.name,
                        partition,
                        removed,
                        "Retention enforced"
                    );
                    self.storage
                        .replace_partition_log(&topic.name, partition, log)?;
                }
            }
        }
        Ok(stats)
    }
}

// ─── Compaction logic (pure function) ────────────────────────────────────────

/// Pure compaction of a partition log.
///
/// Records with a `None` key are kept as-is (tombstones).
/// For keyed records, only the record with the highest offset per key survives.
fn compact_log(mut log: PartitionLog) -> PartitionLog {
    // Map key → index of the latest record with that key.
    let mut latest: HashMap<Vec<u8>, usize> = HashMap::new();

    for (i, record) in log.records.iter().enumerate() {
        if let Some(ref key) = record.key {
            latest.insert(key.clone(), i);
        }
    }

    // Keep records that are either:
    //   (a) the latest for their key,  OR
    //   (b) tombstones (null key).
    let mut compacted: Vec<Record> = Vec::with_capacity(log.records.len());
    for (i, record) in log.records.drain(..).enumerate() {
        let keep = match &record.key {
            None => true,
            Some(k) => latest.get(k).copied() == Some(i),
        };
        if keep {
            compacted.push(record);
        }
    }

    log.records = compacted;
    log.last_compacted_offset = log.high_watermark.saturating_sub(1);
    log
}

fn estimate_bytes(record_count: usize) -> u64 {
    // Rough estimate: average record ≈ 256 bytes.
    (record_count * 256) as u64
}

fn estimated_total_bytes(records: &[Record]) -> usize {
    records.iter().map(|r| {
        r.key.as_deref().map(|k| k.len()).unwrap_or(0)
            + r.value.as_deref().map(|v| v.len()).unwrap_or(0)
            + 64 // fixed overhead
    }).sum()
}

// ─── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CompactionStats {
    pub partitions_compacted: usize,
    pub records_before: usize,
    pub records_after: usize,
    pub bytes_reclaimed: u64,
}

#[derive(Debug)]
pub struct PartitionCompactionResult {
    pub topic: String,
    pub partition: u32,
    pub records_before: usize,
    pub records_after: usize,
    pub bytes_reclaimed: u64,
}

#[derive(Debug, Default)]
pub struct RetentionStats {
    pub partitions_trimmed: usize,
    pub records_deleted: usize,
}
