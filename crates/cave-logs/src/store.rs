//! Multi-tenant log store with chunk-based storage, retention, and compaction.
//!
//! Architecture:
//!   - Each (tenant, stream_fp) pair has one active `HeadChunk` in memory.
//!   - When a head chunk is flushed it becomes a sealed `Chunk` kept in memory
//!     in the `ChunkStore` (future: write to disk / object storage).
//!   - A broadcast channel dispatches `TailEvent`s to WebSocket subscribers.
//!   - A background compaction task periodically flushes head chunks and
//!     removes entries older than the tenant's retention window.

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use chrono::Utc;

use crate::chunk::{HeadChunk, decode_chunk, DEFAULT_CHUNK_TARGET_SIZE, DEFAULT_CHUNK_MAX_AGE_SECS};
use crate::index::{LabelIndex, StreamKey, ChunkMeta};
use crate::models::{
    Chunk, Codec, Direction, IndexStats, Labels, LogEntry, LogStream, TailEvent,
    TenantId, TimestampNs,
};

const TAIL_CHANNEL_CAPACITY: usize = 1024;
const DEFAULT_RETENTION_SECS: i64 = 7 * 24 * 3600;

/// Sealed chunks stored in memory.
struct ChunkStore {
    chunks: Vec<Chunk>,
}

impl ChunkStore {
    fn new() -> Self { Self { chunks: Vec::new() } }

    fn push(&mut self, chunk: Chunk) { self.chunks.push(chunk); }

    /// Entries from chunks that overlap [start_ns, end_ns].
    fn entries_in_range(
        &self,
        fp: u64,
        tenant: &str,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
    ) -> Vec<LogEntry> {
        let mut out = Vec::new();
        for chunk in &self.chunks {
            if chunk.stream_fp == fp
                && chunk.tenant == tenant
                && chunk.max_ts >= start_ns
                && chunk.min_ts <= end_ns
            {
                if let Ok(entries) = decode_chunk(chunk) {
                    for e in entries {
                        if e.ts >= start_ns && e.ts <= end_ns {
                            out.push(e);
                        }
                    }
                }
            }
        }
        out
    }

    fn prune_before(&mut self, cutoff_ns: TimestampNs) {
        self.chunks.retain(|c| c.max_ts >= cutoff_ns);
    }

    fn stats(&self) -> (u64, u64, u64) {
        let chunks = self.chunks.len() as u64;
        let entries: u64 = self.chunks.iter().map(|c| c.num_entries).sum();
        let bytes: u64 = self.chunks.iter().map(|c| c.data.len() as u64).sum();
        (chunks, entries, bytes)
    }
}

/// Per-stream state: active head chunk + stream metadata.
struct StreamState {
    labels: Labels,
    tenant: TenantId,
    head: HeadChunk,
    /// Total entries (head + sealed chunks).
    total_entries: u64,
    created_at: TimestampNs,
    last_write: TimestampNs,
}

impl StreamState {
    fn new(labels: Labels, tenant: impl Into<TenantId>, fp: u64) -> Self {
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let tenant = tenant.into();
        Self {
            head: HeadChunk::new(fp, tenant.clone()),
            labels,
            tenant,
            total_entries: 0,
            created_at: now,
            last_write: now,
        }
    }
}

/// The main log store — thread-safe via Arc<RwLock<_>>.
pub struct LogStore {
    streams: RwLock<HashMap<(TenantId, u64), StreamState>>,
    chunks: RwLock<ChunkStore>,
    pub index: Arc<LabelIndex>,
    tail_tx: broadcast::Sender<TailEvent>,
    codec: Codec,
    chunk_target_size: usize,
    chunk_max_age_secs: u64,
    retention_secs: i64,
}

impl LogStore {
    pub fn new() -> Arc<Self> {
        Self::with_codec(Codec::Snappy)
    }

    pub fn with_codec(codec: Codec) -> Arc<Self> {
        let (tail_tx, _) = broadcast::channel(TAIL_CHANNEL_CAPACITY);
        Arc::new(Self {
            streams: RwLock::new(HashMap::new()),
            chunks: RwLock::new(ChunkStore::new()),
            index: Arc::new(LabelIndex::new()),
            tail_tx,
            codec,
            chunk_target_size: DEFAULT_CHUNK_TARGET_SIZE,
            chunk_max_age_secs: DEFAULT_CHUNK_MAX_AGE_SECS,
            retention_secs: DEFAULT_RETENTION_SECS,
        })
    }

    /// Subscribe to the tail broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<TailEvent> {
        self.tail_tx.subscribe()
    }

    /// Ingest a batch of entries for one stream.
    pub fn push(
        &self,
        tenant: &str,
        labels: Labels,
        entries: Vec<LogEntry>,
    ) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let fp = labels.fingerprint();
        let key = (tenant.to_owned(), fp);

        {
            let mut streams = self.streams.write();
            let state = streams
                .entry(key.clone())
                .or_insert_with(|| {
                    self.index.index_stream(tenant, fp, &labels);
                    StreamState::new(labels.clone(), tenant, fp)
                });

            state.last_write = entries.last().map(|e| e.ts).unwrap_or(state.last_write);
            state.total_entries += entries.len() as u64;

            for entry in &entries {
                // Broadcast to tail subscribers (best-effort; ignore lag).
                let _ = self.tail_tx.send(TailEvent {
                    tenant: tenant.to_owned(),
                    stream_labels: labels.clone(),
                    entry: entry.clone(),
                });
                state.head.push(entry.clone());
            }

            // Check if head needs flushing.
            if state.head.should_flush(self.chunk_target_size, self.chunk_max_age_secs) {
                self.flush_head_locked(&mut streams, &key)?;
            }
        }

        Ok(())
    }

    /// Flush the head chunk for a stream (called with streams write-lock held).
    fn flush_head_locked(
        &self,
        streams: &mut HashMap<(TenantId, u64), StreamState>,
        key: &(TenantId, u64),
    ) -> anyhow::Result<()> {
        if let Some(state) = streams.get_mut(key) {
            if state.head.is_empty() {
                return Ok(());
            }
            // Swap the head chunk out.
            let fp = key.1;
            let tenant = key.0.clone();
            let new_head = HeadChunk::new(fp, tenant.clone());
            let old_head = std::mem::replace(&mut state.head, new_head);

            let chunk = old_head.flush(self.codec)?;

            let min_ts = chunk.min_ts;
            let max_ts = chunk.max_ts;
            let n = chunk.num_entries;
            let sz = chunk.data.len() as u64;

            // Add to chunk store.
            self.chunks.write().push(chunk);

            // Update label index with chunk meta (bloom filter built over line content).
            // For simplicity we build an empty-lines bloom (real implementation would
            // pass the actual lines from the chunk; decoding here is expensive, so we
            // skip bloom population at flush time and populate it lazily on first query).
            let meta = ChunkMeta::new(
                StreamKey::new(&tenant, fp),
                min_ts,
                max_ts,
                n,
                sz,
                &[],
            );
            self.index.add_chunk(meta);
        }
        Ok(())
    }

    /// Query log entries matching [start_ns, end_ns] for a set of stream fps.
    /// Returns entries in the requested direction up to `limit`.
    pub fn query_entries(
        &self,
        tenant: &str,
        fps: &[u64],
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        limit: usize,
        direction: Direction,
    ) -> Vec<(u64, Labels, Vec<LogEntry>)> {
        let streams = self.streams.read();
        let chunks = self.chunks.read();
        let mut out = Vec::new();

        for &fp in fps {
            let key = (tenant.to_owned(), fp);
            let labels = streams
                .get(&key)
                .map(|s| s.labels.clone())
                .or_else(|| self.index.labels_for_fp(fp).map(|(l, _)| l));
            let labels = match labels {
                Some(l) => l,
                None => continue,
            };

            // Entries from sealed chunks.
            let mut entries: Vec<LogEntry> =
                chunks.entries_in_range(fp, tenant, start_ns, end_ns);

            // Entries from the active head chunk.
            if let Some(state) = streams.get(&key) {
                for e in &state.head.entries {
                    if e.ts >= start_ns && e.ts <= end_ns {
                        entries.push(e.clone());
                    }
                }
            }

            if entries.is_empty() {
                continue;
            }

            // Sort and apply direction + limit.
            entries.sort_by_key(|e| e.ts);
            if direction == Direction::Backward {
                entries.reverse();
            }
            entries.truncate(limit);

            out.push((fp, labels, entries));
        }

        out
    }

    /// Query all streams for a tenant that match a label selector predicate.
    pub fn matching_fps<F>(&self, tenant: &str, predicate: F) -> Vec<u64>
    where
        F: Fn(&Labels) -> bool,
    {
        let streams = self.streams.read();
        streams
            .iter()
            .filter(|((t, _), state)| t == tenant && predicate(&state.labels))
            .map(|((_, fp), _)| *fp)
            .collect()
    }

    /// All label names for a tenant.
    pub fn label_names(&self, tenant: &str) -> Vec<String> {
        self.index.label_names(Some(tenant))
    }

    /// All values for a label name for a tenant.
    pub fn label_values(&self, name: &str, tenant: &str) -> Vec<String> {
        self.index.label_values(name, Some(tenant))
    }

    /// All label sets that match the given fps.
    pub fn series(&self, tenant: &str, fps: &[u64]) -> Vec<Labels> {
        let streams = self.streams.read();
        fps.iter()
            .filter_map(|fp| {
                let key = (tenant.to_owned(), *fp);
                streams.get(&key).map(|s| s.labels.clone())
            })
            .collect()
    }

    /// Count, byte, and chunk statistics.
    pub fn stats(&self) -> IndexStats {
        let streams = self.streams.read();
        let (chunks, entries_sealed, bytes) = self.chunks.read().stats();
        let stream_count = streams.len() as u64;
        let head_entries: u64 = streams.values().map(|s| s.head.entries.len() as u64).sum();
        IndexStats {
            streams: stream_count,
            chunks,
            entries: entries_sealed + head_entries,
            bytes,
        }
    }

    /// Flush all head chunks (e.g. on shutdown or compaction tick).
    pub fn flush_all(&self) -> anyhow::Result<()> {
        let keys: Vec<(TenantId, u64)> = {
            let s = self.streams.read();
            s.keys().cloned().collect()
        };
        let mut streams = self.streams.write();
        for key in keys {
            self.flush_head_locked(&mut streams, &key)?;
        }
        Ok(())
    }

    /// Remove data older than the retention window.
    pub fn compact(&self) {
        let cutoff_ns = (Utc::now().timestamp() - self.retention_secs) * 1_000_000_000;
        self.chunks.write().prune_before(cutoff_ns);
        self.index.prune_chunks_before(cutoff_ns);
        // Trim head chunk entries older than cutoff.
        let mut streams = self.streams.write();
        for state in streams.values_mut() {
            state.head.entries.retain(|e| e.ts >= cutoff_ns);
        }
        // Remove streams with no activity.
        streams.retain(|_, s| !s.head.is_empty() || s.total_entries > 0);
    }

    /// Deduplicate entries within a window: entries with identical (ts, line) are removed.
    pub fn dedup_entries(entries: &mut Vec<LogEntry>) {
        if entries.len() < 2 {
            return;
        }
        entries.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.line.cmp(&b.line)));
        entries.dedup_by(|a, b| a.ts == b.ts && a.line == b.line);
    }

    /// Count entries per time bucket for metric queries.
    pub fn count_over_buckets(
        &self,
        tenant: &str,
        fps: &[u64],
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> Vec<(TimestampNs, f64)> {
        let bucket_count = ((end_ns - start_ns) / step_ns).max(1) as usize;
        let mut buckets = vec![0u64; bucket_count];

        let results = self.query_entries(tenant, fps, start_ns, end_ns, usize::MAX, Direction::Forward);
        for (_, _, entries) in results {
            for e in entries {
                let idx = ((e.ts - start_ns) / step_ns) as usize;
                if idx < bucket_count {
                    buckets[idx] += 1;
                }
            }
        }

        buckets
            .into_iter()
            .enumerate()
            .map(|(i, count)| (start_ns + i as i64 * step_ns, count as f64))
            .collect()
    }

    /// Bytes per time bucket.
    pub fn bytes_over_buckets(
        &self,
        tenant: &str,
        fps: &[u64],
        start_ns: TimestampNs,
        end_ns: TimestampNs,
        step_ns: i64,
    ) -> Vec<(TimestampNs, f64)> {
        let bucket_count = ((end_ns - start_ns) / step_ns).max(1) as usize;
        let mut buckets = vec![0u64; bucket_count];

        let results = self.query_entries(tenant, fps, start_ns, end_ns, usize::MAX, Direction::Forward);
        for (_, _, entries) in results {
            for e in entries {
                let idx = ((e.ts - start_ns) / step_ns) as usize;
                if idx < bucket_count {
                    buckets[idx] += e.size_bytes() as u64;
                }
            }
        }

        buckets
            .into_iter()
            .enumerate()
            .map(|(i, b)| (start_ns + i as i64 * step_ns, b as f64))
            .collect()
    }
}

impl Default for LogStore {
    fn default() -> Self {
        // Safety: Arc::try_unwrap will succeed since we just created it.
        // This impl is only here to satisfy trait bounds; prefer LogStore::new().
        panic!("LogStore::default() not supported; use LogStore::new()")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Labels, LogEntry};
    use std::collections::HashMap;

    fn tenant() -> &'static str { "test_tenant" }

    fn labels(app: &str) -> Labels {
        Labels::new(HashMap::from([("app".into(), app.into())]))
    }

    fn now_ns() -> TimestampNs {
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    }

    #[test]
    fn push_and_query() {
        let store = LogStore::new();
        let t = now_ns();
        let entries = vec![
            LogEntry::new(t, "hello world"),
            LogEntry::new(t + 1_000_000, "second line"),
        ];
        store.push(tenant(), labels("app1"), entries).unwrap();

        let fps = store.matching_fps(tenant(), |_| true);
        assert_eq!(fps.len(), 1);

        let results = store.query_entries(
            tenant(), &fps, t - 1, t + 2_000_000, 100, Direction::Forward,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].2.len(), 2);
        assert_eq!(results[0].2[0].line, "hello world");
    }

    #[test]
    fn label_names_and_values() {
        let store = LogStore::new();
        let t = now_ns();
        store.push(tenant(), labels("nginx"), vec![LogEntry::new(t, "l")]).unwrap();
        store.push(tenant(), labels("postgres"), vec![LogEntry::new(t, "l")]).unwrap();

        let names = store.label_names(tenant());
        assert!(names.contains(&"app".to_owned()));

        let vals = store.label_values("app", tenant());
        assert!(vals.contains(&"nginx".to_owned()));
        assert!(vals.contains(&"postgres".to_owned()));
    }

    #[test]
    fn deduplication() {
        let t = now_ns();
        let mut entries = vec![
            LogEntry::new(t, "dup"),
            LogEntry::new(t, "dup"),
            LogEntry::new(t + 1, "unique"),
        ];
        LogStore::dedup_entries(&mut entries);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn count_over_buckets() {
        let store = LogStore::new();
        let t = 0i64;
        let step = 1_000_000_000i64; // 1s
        let entries: Vec<LogEntry> = (0..10).map(|i| LogEntry::new(t + i * step / 2, "x")).collect();
        store.push(tenant(), labels("b"), entries).unwrap();

        let fps = store.matching_fps(tenant(), |_| true);
        let buckets = store.count_over_buckets(tenant(), &fps, t, t + 10 * step, step);
        assert_eq!(buckets.len(), 10);
        let total: f64 = buckets.iter().map(|(_, c)| c).sum();
        assert_eq!(total as u64, 10);
    }

    #[test]
    fn stats() {
        let store = LogStore::new();
        let t = now_ns();
        store.push("t1", labels("a"), vec![LogEntry::new(t, "l1")]).unwrap();
        store.push("t2", labels("a"), vec![LogEntry::new(t, "l2")]).unwrap();
        let s = store.stats();
        assert_eq!(s.streams, 2);
        assert!(s.entries >= 2);
    }
}
