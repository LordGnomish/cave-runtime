// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tiered storage (KIP-405) — skeleton.
//!
//! Mirrors `storage/internals/log/RemoteLogManager.java` and the
//! supporting `RemoteStorageManager` + `RemoteLogMetadataManager`
//! plugin interfaces from upstream Apache Kafka 4.2.0. The cave
//! port ships the trait surface, an in-memory exerciser, an
//! LRU index cache, and a `RemoteLogManager` orchestrator that
//! is enough to cover the upstream test cases for the manager
//! state machine.
//!
//! ## Honest scope
//!
//! * No S3 / HDFS / GCS plugin in this batch.
//! * No tiered fetch wiring through the broker Fetch RPC.
//! * Retention is offset-based; time-based retention is tracked.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::error::{StreamsError, StreamsResult};

/// `TopicIdPartition` — mirrors upstream `TopicIdPartition`. The
/// `topic_uuid` is the KIP-516 universally-unique topic id;
/// zero is treated as "not yet assigned" in this batch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicIdPartition {
    pub topic: String,
    pub topic_uuid: u64,
    pub partition: u32,
}

/// Identifier for a single remote-stored log segment. The
/// `segment_uuid` is the unique handle the metadata manager
/// hands to the storage manager to dereference the bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RemoteLogSegmentId {
    pub topic_partition: TopicIdPartition,
    pub segment_uuid: u64,
}

/// Lifecycle of a remote segment per KIP-405.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLogSegmentState {
    CopyStarted,
    CopyFinished,
    DeletePartitionStarted,
    DeletePartitionFinished,
}

/// Carrier for everything the metadata manager needs to know
/// about a single remote-stored segment. Mirrors upstream
/// `RemoteLogSegmentMetadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLogSegmentMetadata {
    pub id: RemoteLogSegmentId,
    pub start_offset: u64,
    pub end_offset: u64,
    pub max_timestamp_ms: i64,
    pub broker_id: i32,
    pub event_timestamp_ms: i64,
    pub segment_size_bytes: u64,
    pub state: RemoteLogSegmentState,
}

impl RemoteLogSegmentMetadata {
    pub fn contains_offset(&self, offset: u64) -> bool {
        offset >= self.start_offset && offset <= self.end_offset
    }
}

// ── Plugin traits ────────────────────────────────────────────────────────────

/// Mirrors `RemoteStorageManager` — moves segment *bytes*.
pub trait RemoteStorageManager: Send + Sync {
    fn copy_log_segment(
        &mut self,
        metadata: &RemoteLogSegmentMetadata,
        bytes: Vec<u8>,
    ) -> StreamsResult<()>;

    fn fetch_log_segment(
        &self,
        id: &RemoteLogSegmentId,
        start: u64,
        end: u64,
    ) -> StreamsResult<Vec<u8>>;

    fn delete_log_segment(&mut self, id: &RemoteLogSegmentId) -> StreamsResult<()>;
}

/// Mirrors `RemoteLogMetadataManager` — owns the *index* of
/// segments per partition.
pub trait RemoteLogMetadataManager: Send + Sync {
    fn add_remote_log_segment(&mut self, metadata: RemoteLogSegmentMetadata) -> StreamsResult<()>;

    fn update_remote_log_segment_state(
        &mut self,
        id: &RemoteLogSegmentId,
        state: RemoteLogSegmentState,
    ) -> StreamsResult<()>;

    fn list_segments(&self, partition: &TopicIdPartition) -> Vec<RemoteLogSegmentMetadata>;

    fn remove_remote_log_segment(&mut self, id: &RemoteLogSegmentId) -> StreamsResult<()>;
}

// ── In-memory exercisers ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryRemoteStorageManager {
    bytes: BTreeMap<RemoteLogSegmentId, Vec<u8>>,
}

impl InMemoryRemoteStorageManager {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RemoteStorageManager for InMemoryRemoteStorageManager {
    fn copy_log_segment(
        &mut self,
        metadata: &RemoteLogSegmentMetadata,
        bytes: Vec<u8>,
    ) -> StreamsResult<()> {
        if self.bytes.contains_key(&metadata.id) {
            return Err(StreamsError::Internal(format!(
                "segment {:?} already stored",
                metadata.id
            )));
        }
        self.bytes.insert(metadata.id.clone(), bytes);
        Ok(())
    }

    fn fetch_log_segment(
        &self,
        id: &RemoteLogSegmentId,
        start: u64,
        end: u64,
    ) -> StreamsResult<Vec<u8>> {
        let bytes = self
            .bytes
            .get(id)
            .ok_or_else(|| StreamsError::Internal(format!("segment {id:?} not stored")))?;
        let s = start as usize;
        let e = (end as usize).min(bytes.len());
        if s > bytes.len() || s > e {
            return Err(StreamsError::Internal("fetch out of range".into()));
        }
        Ok(bytes[s..e].to_vec())
    }

    fn delete_log_segment(&mut self, id: &RemoteLogSegmentId) -> StreamsResult<()> {
        self.bytes.remove(id);
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryRemoteLogMetadataManager {
    /// Partition → ordered (by start_offset) list of segments.
    by_partition: BTreeMap<TopicIdPartition, Vec<RemoteLogSegmentMetadata>>,
}

impl InMemoryRemoteLogMetadataManager {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RemoteLogMetadataManager for InMemoryRemoteLogMetadataManager {
    fn add_remote_log_segment(&mut self, metadata: RemoteLogSegmentMetadata) -> StreamsResult<()> {
        let part = metadata.id.topic_partition.clone();
        let list = self.by_partition.entry(part).or_default();
        // Overlap check: any existing segment whose [start..=end]
        // intersects this one is a hard error.
        for existing in list.iter() {
            let no_overlap = metadata.end_offset < existing.start_offset
                || metadata.start_offset > existing.end_offset;
            if !no_overlap {
                return Err(StreamsError::Internal(format!(
                    "segment overlaps existing {:?}",
                    existing.id
                )));
            }
        }
        list.push(metadata);
        list.sort_by_key(|m| m.start_offset);
        Ok(())
    }

    fn update_remote_log_segment_state(
        &mut self,
        id: &RemoteLogSegmentId,
        state: RemoteLogSegmentState,
    ) -> StreamsResult<()> {
        for list in self.by_partition.values_mut() {
            for m in list.iter_mut() {
                if &m.id == id {
                    m.state = state;
                    return Ok(());
                }
            }
        }
        Err(StreamsError::Internal(format!("segment {id:?} not found")))
    }

    fn list_segments(&self, partition: &TopicIdPartition) -> Vec<RemoteLogSegmentMetadata> {
        self.by_partition
            .get(partition)
            .cloned()
            .unwrap_or_default()
    }

    fn remove_remote_log_segment(&mut self, id: &RemoteLogSegmentId) -> StreamsResult<()> {
        for list in self.by_partition.values_mut() {
            if let Some(pos) = list.iter().position(|m| &m.id == id) {
                list.remove(pos);
                return Ok(());
            }
        }
        Err(StreamsError::Internal(format!("segment {id:?} not found")))
    }
}

// ── LRU index cache ──────────────────────────────────────────────────────────

/// Capacity-bounded LRU. Used by the `RemoteLogManager` to keep
/// hot remote indexes in memory. Tracks insertion order in a
/// `VecDeque` and rebuilds it on every hit; correct, O(N) per
/// hit which is fine for typical cache sizes (≤ a few thousand).
pub struct RemoteIndexCache<K: Hash + Eq + Clone, V: Clone> {
    capacity: usize,
    order: VecDeque<K>,
    data: BTreeMap<K, V>,
}

impl<K: Hash + Eq + Clone + Ord, V: Clone> RemoteIndexCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::with_capacity(capacity.max(1)),
            data: BTreeMap::new(),
        }
    }

    pub fn put(&mut self, k: K, v: V) {
        if self.data.contains_key(&k) {
            self.promote(&k);
            self.data.insert(k, v);
            return;
        }
        if self.data.len() == self.capacity {
            if let Some(evict) = self.order.pop_front() {
                self.data.remove(&evict);
            }
        }
        self.order.push_back(k.clone());
        self.data.insert(k, v);
    }

    pub fn get(&mut self, k: &K) -> Option<&V> {
        if self.data.contains_key(k) {
            self.promote(k);
            self.data.get(k)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn promote(&mut self, k: &K) {
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
            self.order.push_back(k.clone());
        }
    }
}

// ── Orchestrator ─────────────────────────────────────────────────────────────

pub struct RemoteLogManager {
    rsm: Box<dyn RemoteStorageManager>,
    rmm: Box<dyn RemoteLogMetadataManager>,
}

impl RemoteLogManager {
    pub fn new(rsm: Box<dyn RemoteStorageManager>, rmm: Box<dyn RemoteLogMetadataManager>) -> Self {
        Self { rsm, rmm }
    }

    /// Copy a local segment to remote storage + register its
    /// metadata. Mirrors `RemoteLogManager.copyLogSegment()`.
    pub fn copy_log_segment(
        &mut self,
        metadata: RemoteLogSegmentMetadata,
        bytes: Vec<u8>,
    ) -> StreamsResult<()> {
        self.rsm.copy_log_segment(&metadata, bytes)?;
        let mut finished = metadata;
        finished.state = RemoteLogSegmentState::CopyFinished;
        self.rmm.add_remote_log_segment(finished)
    }

    pub fn list_segments(
        &self,
        partition: &TopicIdPartition,
    ) -> std::vec::IntoIter<RemoteLogSegmentMetadata> {
        self.rmm.list_segments(partition).into_iter()
    }

    /// First segment whose [start_offset..=end_offset] contains
    /// `offset`. Mirrors `RemoteLogManager.findRemoteSegmentForOffset()`.
    pub fn find_segment_for_offset(
        &self,
        partition: &TopicIdPartition,
        offset: u64,
    ) -> Option<RemoteLogSegmentMetadata> {
        self.rmm
            .list_segments(partition)
            .into_iter()
            .find(|m| m.contains_offset(offset))
    }

    /// Retention pass — remove every segment whose entire range
    /// is below `min_kept_offset`. Returns the removed metadata
    /// for the caller to log / metric.
    pub fn apply_retention(
        &mut self,
        partition: &TopicIdPartition,
        min_kept_offset: u64,
    ) -> Vec<RemoteLogSegmentMetadata> {
        let snap = self.rmm.list_segments(partition);
        let mut removed = Vec::new();
        for m in snap {
            if m.end_offset < min_kept_offset {
                if self.rsm.delete_log_segment(&m.id).is_ok()
                    && self.rmm.remove_remote_log_segment(&m.id).is_ok()
                {
                    removed.push(m);
                }
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tp(t: &str, p: u32) -> TopicIdPartition {
        TopicIdPartition {
            topic: t.into(),
            topic_uuid: 0,
            partition: p,
        }
    }
    fn meta(t: &str, p: u32, base: u64, size: u64) -> RemoteLogSegmentMetadata {
        let id = RemoteLogSegmentId {
            topic_partition: tp(t, p),
            segment_uuid: base,
        };
        RemoteLogSegmentMetadata {
            id,
            start_offset: base,
            end_offset: base + size - 1,
            max_timestamp_ms: 0,
            broker_id: 1,
            event_timestamp_ms: 0,
            segment_size_bytes: size,
            state: RemoteLogSegmentState::CopyStarted,
        }
    }

    #[test]
    fn metadata_contains_offset_bounds() {
        let m = meta("o", 0, 0, 100);
        assert!(m.contains_offset(0));
        assert!(m.contains_offset(99));
        assert!(!m.contains_offset(100));
    }

    #[test]
    fn metadata_state_default_is_copy_started() {
        let m = meta("o", 0, 0, 1);
        assert_eq!(m.state, RemoteLogSegmentState::CopyStarted);
    }

    #[test]
    fn in_memory_rsm_put_then_fetch() {
        let mut rsm = InMemoryRemoteStorageManager::new();
        let m = meta("o", 0, 0, 4);
        rsm.copy_log_segment(&m, vec![1, 2, 3, 4]).unwrap();
        let chunk = rsm.fetch_log_segment(&m.id, 1, 3).unwrap();
        assert_eq!(chunk, vec![2, 3]);
    }

    #[test]
    fn in_memory_rsm_put_rejects_duplicate() {
        let mut rsm = InMemoryRemoteStorageManager::new();
        let m = meta("o", 0, 0, 1);
        rsm.copy_log_segment(&m, vec![1]).unwrap();
        assert!(rsm.copy_log_segment(&m, vec![1]).is_err());
    }

    #[test]
    fn in_memory_rmm_overlap_rejected() {
        let mut rmm = InMemoryRemoteLogMetadataManager::new();
        rmm.add_remote_log_segment(meta("o", 0, 0, 10)).unwrap();
        assert!(rmm.add_remote_log_segment(meta("o", 0, 5, 10)).is_err());
    }

    #[test]
    fn in_memory_rmm_update_state_round_trips() {
        let mut rmm = InMemoryRemoteLogMetadataManager::new();
        let m = meta("o", 0, 0, 1);
        rmm.add_remote_log_segment(m.clone()).unwrap();
        rmm.update_remote_log_segment_state(&m.id, RemoteLogSegmentState::CopyFinished)
            .unwrap();
        let listed = rmm.list_segments(&tp("o", 0));
        assert_eq!(listed[0].state, RemoteLogSegmentState::CopyFinished);
    }

    #[test]
    fn in_memory_rmm_list_returns_sorted() {
        let mut rmm = InMemoryRemoteLogMetadataManager::new();
        rmm.add_remote_log_segment(meta("o", 0, 100, 100)).unwrap();
        rmm.add_remote_log_segment(meta("o", 0, 0, 100)).unwrap();
        let listed = rmm.list_segments(&tp("o", 0));
        assert_eq!(listed[0].start_offset, 0);
        assert_eq!(listed[1].start_offset, 100);
    }

    #[test]
    fn lru_evicts_oldest() {
        let mut c: RemoteIndexCache<u64, &str> = RemoteIndexCache::new(2);
        c.put(1, "a");
        c.put(2, "b");
        c.put(3, "c");
        assert!(c.get(&1).is_none());
        assert!(c.get(&2).is_some());
    }

    #[test]
    fn lru_hit_promotes_entry() {
        let mut c: RemoteIndexCache<u64, &str> = RemoteIndexCache::new(2);
        c.put(1, "a");
        c.put(2, "b");
        let _ = c.get(&1);
        c.put(3, "c");
        assert!(c.get(&2).is_none());
        assert!(c.get(&1).is_some());
    }

    #[test]
    fn lru_re_put_promotes_and_overwrites() {
        let mut c: RemoteIndexCache<u64, &str> = RemoteIndexCache::new(2);
        c.put(1, "a");
        c.put(2, "b");
        c.put(1, "A");
        c.put(3, "c");
        assert!(c.get(&2).is_none());
        assert_eq!(c.get(&1), Some(&"A"));
    }

    #[test]
    fn remote_log_mgr_copy_emits_metadata() {
        let rsm = InMemoryRemoteStorageManager::new();
        let rmm = InMemoryRemoteLogMetadataManager::new();
        let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
        let m = meta("o", 0, 0, 100);
        mgr.copy_log_segment(m, vec![0; 100]).unwrap();
        let listed: Vec<_> = mgr.list_segments(&tp("o", 0)).collect();
        assert_eq!(listed.len(), 1);
        // copy_log_segment flips state to CopyFinished.
        assert_eq!(listed[0].state, RemoteLogSegmentState::CopyFinished);
    }

    #[test]
    fn remote_log_mgr_find_segment_for_offset() {
        let rsm = InMemoryRemoteStorageManager::new();
        let rmm = InMemoryRemoteLogMetadataManager::new();
        let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
        mgr.copy_log_segment(meta("o", 0, 0, 100), vec![0; 100])
            .unwrap();
        mgr.copy_log_segment(meta("o", 0, 100, 100), vec![0; 100])
            .unwrap();
        let hit = mgr.find_segment_for_offset(&tp("o", 0), 50);
        assert_eq!(hit.unwrap().start_offset, 0);
        let hit2 = mgr.find_segment_for_offset(&tp("o", 0), 150);
        assert_eq!(hit2.unwrap().start_offset, 100);
        assert!(mgr.find_segment_for_offset(&tp("o", 0), 999).is_none());
    }

    #[test]
    fn remote_log_mgr_retention_removes_old() {
        let rsm = InMemoryRemoteStorageManager::new();
        let rmm = InMemoryRemoteLogMetadataManager::new();
        let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
        mgr.copy_log_segment(meta("o", 0, 0, 100), vec![0; 100])
            .unwrap();
        mgr.copy_log_segment(meta("o", 0, 100, 100), vec![0; 100])
            .unwrap();
        let removed = mgr.apply_retention(&tp("o", 0), 100);
        assert_eq!(removed.len(), 1);
        let remaining: Vec<_> = mgr.list_segments(&tp("o", 0)).collect();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].start_offset, 100);
    }

    #[test]
    fn remote_log_mgr_delete_unknown_is_idempotent() {
        let rsm = InMemoryRemoteStorageManager::new();
        let rmm = InMemoryRemoteLogMetadataManager::new();
        let mut mgr = RemoteLogManager::new(Box::new(rsm), Box::new(rmm));
        // No segments registered — retention is a no-op.
        let removed = mgr.apply_retention(&tp("o", 0), 100);
        assert!(removed.is_empty());
    }

    #[test]
    fn rmm_remove_unknown_errors() {
        let mut rmm = InMemoryRemoteLogMetadataManager::new();
        let id = RemoteLogSegmentId {
            topic_partition: tp("o", 0),
            segment_uuid: 42,
        };
        assert!(rmm.remove_remote_log_segment(&id).is_err());
    }

    #[test]
    fn rmm_update_unknown_errors() {
        let mut rmm = InMemoryRemoteLogMetadataManager::new();
        let id = RemoteLogSegmentId {
            topic_partition: tp("o", 0),
            segment_uuid: 42,
        };
        assert!(
            rmm.update_remote_log_segment_state(&id, RemoteLogSegmentState::CopyFinished)
                .is_err()
        );
    }
}
