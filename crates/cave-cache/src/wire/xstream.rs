//! XADD / XREAD / XREADGROUP / XACK consumer-group state machine.
//!
//! Models the Redis Streams data structure as documented at
//! <https://redis.io/docs/data-types/streams/> and implemented in
//! `t_stream.c` in the Redis source. Each stream entry has an `(ms, seq)`
//! id; the server enforces strict monotonic ordering. Consumer groups track
//! a `last_delivered_id` plus a per-consumer Pending Entries List (PEL).

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamId {
    pub ms: u64,
    pub seq: u64,
}

impl StreamId {
    pub const MIN: StreamId = StreamId { ms: 0, seq: 0 };
    pub const MAX: StreamId = StreamId {
        ms: u64::MAX,
        seq: u64::MAX,
    };

    pub fn new(ms: u64, seq: u64) -> Self {
        Self { ms, seq }
    }

    pub fn parse(s: &str) -> Result<Self, StreamError> {
        if let Some((ms, seq)) = s.split_once('-') {
            let ms = ms.parse::<u64>().map_err(|_| StreamError::InvalidId)?;
            let seq = seq.parse::<u64>().map_err(|_| StreamError::InvalidId)?;
            Ok(Self { ms, seq })
        } else {
            let ms = s.parse::<u64>().map_err(|_| StreamError::InvalidId)?;
            Ok(Self { ms, seq: 0 })
        }
    }

    pub fn format(&self) -> String {
        format!("{}-{}", self.ms, self.seq)
    }

    pub fn next(&self) -> Self {
        if self.seq == u64::MAX {
            Self {
                ms: self.ms + 1,
                seq: 0,
            }
        } else {
            Self {
                ms: self.ms,
                seq: self.seq + 1,
            }
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StreamError {
    #[error("ID is not greater than the last id")]
    IdNotGreater,
    #[error("invalid id format")]
    InvalidId,
    #[error("group does not exist: {0}")]
    NoSuchGroup(String),
    #[error("consumer does not exist: {0}")]
    NoSuchConsumer(String),
    #[error("stream does not exist")]
    NoSuchStream,
}

#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: StreamId,
    pub fields: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Debug)]
pub struct Stream {
    entries: BTreeMap<StreamId, Vec<(Vec<u8>, Vec<u8>)>>,
    last_id: StreamId,
    groups: BTreeMap<String, ConsumerGroup>,
    /// Tombstoned ids (XDEL).
    deleted: BTreeMap<StreamId, ()>,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            last_id: StreamId::MIN,
            groups: BTreeMap::new(),
            deleted: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct ConsumerGroup {
    last_delivered_id: StreamId,
    consumers: BTreeMap<String, Consumer>,
    /// Pending Entries List: id → consumer_name.
    pel: BTreeMap<StreamId, String>,
}

#[derive(Debug, Default)]
pub struct Consumer {
    /// Number of pending entries currently held by this consumer.
    pending_count: usize,
}

impl Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// XADD — assign id (auto if explicit_id is `None`) and append the entry.
    pub fn xadd(
        &mut self,
        explicit_id: Option<StreamId>,
        fields: Vec<(Vec<u8>, Vec<u8>)>,
        now_ms: u64,
    ) -> Result<StreamId, StreamError> {
        let id = match explicit_id {
            Some(id) => {
                if id <= self.last_id && self.last_id != StreamId::MIN {
                    return Err(StreamError::IdNotGreater);
                }
                id
            }
            None => {
                if now_ms > self.last_id.ms {
                    StreamId::new(now_ms, 0)
                } else {
                    StreamId::new(self.last_id.ms, self.last_id.seq + 1)
                }
            }
        };
        self.entries.insert(id, fields);
        self.last_id = id;
        Ok(id)
    }

    pub fn last_id(&self) -> StreamId {
        self.last_id
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn xrange(&self, start: StreamId, end: StreamId) -> Vec<StreamEntry> {
        self.entries
            .range(start..=end)
            .map(|(id, fields)| StreamEntry {
                id: *id,
                fields: fields.clone(),
            })
            .collect()
    }

    pub fn xrevrange(&self, start: StreamId, end: StreamId) -> Vec<StreamEntry> {
        // Redis XREVRANGE swaps argument order: end is the lower bound.
        let lo = std::cmp::min(start, end);
        let hi = std::cmp::max(start, end);
        let mut out: Vec<StreamEntry> = self
            .entries
            .range(lo..=hi)
            .map(|(id, fields)| StreamEntry {
                id: *id,
                fields: fields.clone(),
            })
            .collect();
        out.reverse();
        out
    }

    pub fn xread(&self, after: StreamId, count: Option<usize>) -> Vec<StreamEntry> {
        use std::ops::Bound::{Excluded, Unbounded};
        let iter = self.entries.range((Excluded(after), Unbounded));
        let mut out: Vec<_> = iter
            .map(|(id, fields)| StreamEntry {
                id: *id,
                fields: fields.clone(),
            })
            .collect();
        if let Some(c) = count {
            out.truncate(c);
        }
        out
    }

    pub fn xdel(&mut self, id: StreamId) -> bool {
        let removed = self.entries.remove(&id).is_some();
        if removed {
            self.deleted.insert(id, ());
        }
        removed
    }

    pub fn xlen(&self) -> usize {
        self.entries.len()
    }

    // ── Consumer groups ─────────────────────────────────────────────────

    pub fn xgroup_create(&mut self, name: &str, start: StreamId) -> Result<(), StreamError> {
        if self.groups.contains_key(name) {
            return Err(StreamError::NoSuchGroup(format!("BUSYGROUP {}", name)));
        }
        self.groups.insert(
            name.to_string(),
            ConsumerGroup {
                last_delivered_id: start,
                consumers: BTreeMap::new(),
                pel: BTreeMap::new(),
            },
        );
        Ok(())
    }

    pub fn xgroup_destroy(&mut self, name: &str) -> bool {
        self.groups.remove(name).is_some()
    }

    pub fn xreadgroup(
        &mut self,
        group: &str,
        consumer: &str,
        count: Option<usize>,
        deliver_new: bool,
    ) -> Result<Vec<StreamEntry>, StreamError> {
        let last_id = self.last_id;
        let g = self
            .groups
            .get_mut(group)
            .ok_or_else(|| StreamError::NoSuchGroup(group.into()))?;
        let entries: Vec<StreamEntry>;
        if deliver_new {
            use std::ops::Bound::{Excluded, Unbounded};
            entries = self
                .entries
                .range((Excluded(g.last_delivered_id), Unbounded))
                .map(|(id, f)| StreamEntry {
                    id: *id,
                    fields: f.clone(),
                })
                .take(count.unwrap_or(usize::MAX))
                .collect();
            for e in &entries {
                g.pel.insert(e.id, consumer.to_string());
                if e.id > g.last_delivered_id {
                    g.last_delivered_id = e.id;
                }
            }
        } else {
            // History: deliver entries already in PEL for this consumer.
            entries = g
                .pel
                .iter()
                .filter(|(_, c)| c.as_str() == consumer)
                .filter_map(|(id, _)| {
                    self.entries.get(id).map(|f| StreamEntry {
                        id: *id,
                        fields: f.clone(),
                    })
                })
                .take(count.unwrap_or(usize::MAX))
                .collect();
        }
        let c = g.consumers.entry(consumer.to_string()).or_default();
        c.pending_count = g.pel.values().filter(|name| name.as_str() == consumer).count();
        let _ = last_id; // currently unused but kept for future trim sentinel
        Ok(entries)
    }

    pub fn xack(&mut self, group: &str, ids: &[StreamId]) -> Result<usize, StreamError> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or_else(|| StreamError::NoSuchGroup(group.into()))?;
        let mut acked = 0;
        for id in ids {
            if let Some(consumer) = g.pel.remove(id) {
                if let Some(c) = g.consumers.get_mut(&consumer) {
                    c.pending_count = c.pending_count.saturating_sub(1);
                }
                acked += 1;
            }
        }
        Ok(acked)
    }

    pub fn xpending_count(&self, group: &str) -> Result<usize, StreamError> {
        let g = self
            .groups
            .get(group)
            .ok_or_else(|| StreamError::NoSuchGroup(group.into()))?;
        Ok(g.pel.len())
    }

    pub fn xclaim(
        &mut self,
        group: &str,
        new_consumer: &str,
        ids: &[StreamId],
    ) -> Result<Vec<StreamId>, StreamError> {
        let g = self
            .groups
            .get_mut(group)
            .ok_or_else(|| StreamError::NoSuchGroup(group.into()))?;
        let mut claimed = Vec::new();
        for id in ids {
            if let Some(prev) = g.pel.get_mut(id) {
                let prev_name = prev.clone();
                *prev = new_consumer.to_string();
                if let Some(c) = g.consumers.get_mut(&prev_name) {
                    c.pending_count = c.pending_count.saturating_sub(1);
                }
                let c = g.consumers.entry(new_consumer.to_string()).or_default();
                c.pending_count += 1;
                claimed.push(*id);
            }
        }
        Ok(claimed)
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> Vec<(Vec<u8>, Vec<u8>)> {
        pairs
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn stream_id_parse_with_dash() {
        let id = StreamId::parse("123-4").unwrap();
        assert_eq!(id, StreamId::new(123, 4));
    }
    #[test]
    fn stream_id_parse_no_dash_zero_seq() {
        let id = StreamId::parse("123").unwrap();
        assert_eq!(id, StreamId::new(123, 0));
    }
    #[test]
    fn stream_id_format_round_trip() {
        let id = StreamId::new(99, 42);
        assert_eq!(StreamId::parse(&id.format()).unwrap(), id);
    }
    #[test]
    fn stream_id_next_increments_seq() {
        assert_eq!(StreamId::new(1, 0).next(), StreamId::new(1, 1));
    }
    #[test]
    fn stream_id_next_overflows_seq_to_ms() {
        assert_eq!(StreamId::new(1, u64::MAX).next(), StreamId::new(2, 0));
    }
    #[test]
    fn stream_id_invalid_errors() {
        assert!(StreamId::parse("not-a-id").is_err());
    }

    #[test]
    fn xadd_auto_id_uses_now_ms() {
        let mut s = Stream::new();
        let id = s.xadd(None, fields(&[("k", "v")]), 1000).unwrap();
        assert_eq!(id, StreamId::new(1000, 0));
    }
    #[test]
    fn xadd_auto_increments_seq_when_ms_unchanged() {
        let mut s = Stream::new();
        s.xadd(None, fields(&[("k", "v")]), 1000).unwrap();
        let id = s.xadd(None, fields(&[("k", "v2")]), 1000).unwrap();
        assert_eq!(id, StreamId::new(1000, 1));
    }
    #[test]
    fn xadd_explicit_id_must_be_greater() {
        let mut s = Stream::new();
        s.xadd(Some(StreamId::new(10, 0)), fields(&[]), 0).unwrap();
        let err = s
            .xadd(Some(StreamId::new(5, 0)), fields(&[]), 0)
            .unwrap_err();
        assert_eq!(err, StreamError::IdNotGreater);
    }
    #[test]
    fn xadd_first_explicit_zero_zero_allowed() {
        let mut s = Stream::new();
        let id = s.xadd(Some(StreamId::MIN), fields(&[]), 0).unwrap();
        assert_eq!(id, StreamId::MIN);
    }

    #[test]
    fn xrange_returns_inclusive() {
        let mut s = Stream::new();
        for i in 0..5 {
            s.xadd(Some(StreamId::new(i, 0)), fields(&[]), 0).unwrap();
        }
        let out = s.xrange(StreamId::new(1, 0), StreamId::new(3, 0));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, StreamId::new(1, 0));
        assert_eq!(out[2].id, StreamId::new(3, 0));
    }

    #[test]
    fn xrevrange_returns_descending() {
        let mut s = Stream::new();
        for i in 0..3 {
            s.xadd(Some(StreamId::new(i, 0)), fields(&[]), 0).unwrap();
        }
        let out = s.xrevrange(StreamId::new(2, 0), StreamId::new(0, 0));
        assert_eq!(out[0].id, StreamId::new(2, 0));
        assert_eq!(out[2].id, StreamId::new(0, 0));
    }

    #[test]
    fn xread_returns_strictly_after() {
        let mut s = Stream::new();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[]), 0).unwrap();
        s.xadd(Some(StreamId::new(2, 0)), fields(&[]), 0).unwrap();
        let out = s.xread(StreamId::new(1, 0), None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, StreamId::new(2, 0));
    }

    #[test]
    fn xdel_removes_entry() {
        let mut s = Stream::new();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[]), 0).unwrap();
        assert!(s.xdel(StreamId::new(1, 0)));
        assert!(s.is_empty());
        assert!(!s.xdel(StreamId::new(1, 0)));
    }

    #[test]
    fn xlen_reflects_count() {
        let mut s = Stream::new();
        for i in 0..7 {
            s.xadd(Some(StreamId::new(i, 0)), fields(&[]), 0).unwrap();
        }
        assert_eq!(s.xlen(), 7);
    }

    #[test]
    fn xgroup_create_then_destroy() {
        let mut s = Stream::new();
        s.xgroup_create("g1", StreamId::MIN).unwrap();
        assert_eq!(s.group_count(), 1);
        assert!(s.xgroup_destroy("g1"));
        assert_eq!(s.group_count(), 0);
    }

    #[test]
    fn xgroup_create_duplicate_errors() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        let err = s.xgroup_create("g", StreamId::MIN).unwrap_err();
        match err {
            StreamError::NoSuchGroup(_) => {}
            other => panic!("expected NoSuchGroup BUSYGROUP, got {:?}", other),
        }
    }

    #[test]
    fn xreadgroup_delivers_new_entries() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[("a", "1")]), 0).unwrap();
        s.xadd(Some(StreamId::new(2, 0)), fields(&[("a", "2")]), 0).unwrap();
        let out = s.xreadgroup("g", "c1", None, true).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(s.xpending_count("g").unwrap(), 2);
    }

    #[test]
    fn xreadgroup_history_returns_pending_only() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[]), 0).unwrap();
        s.xreadgroup("g", "c1", None, true).unwrap();
        // Adding more new entries should not appear in history mode.
        s.xadd(Some(StreamId::new(2, 0)), fields(&[]), 0).unwrap();
        let history = s.xreadgroup("g", "c1", None, false).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, StreamId::new(1, 0));
    }

    #[test]
    fn xack_removes_from_pel() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[]), 0).unwrap();
        s.xreadgroup("g", "c1", None, true).unwrap();
        let acked = s.xack("g", &[StreamId::new(1, 0)]).unwrap();
        assert_eq!(acked, 1);
        assert_eq!(s.xpending_count("g").unwrap(), 0);
    }

    #[test]
    fn xack_unknown_id_returns_zero() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        let acked = s.xack("g", &[StreamId::new(99, 0)]).unwrap();
        assert_eq!(acked, 0);
    }

    #[test]
    fn xclaim_transfers_ownership() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        s.xadd(Some(StreamId::new(1, 0)), fields(&[]), 0).unwrap();
        s.xreadgroup("g", "c1", None, true).unwrap();
        let claimed = s.xclaim("g", "c2", &[StreamId::new(1, 0)]).unwrap();
        assert_eq!(claimed, vec![StreamId::new(1, 0)]);
        let history_c2 = s.xreadgroup("g", "c2", None, false).unwrap();
        assert_eq!(history_c2.len(), 1);
    }

    #[test]
    fn xclaim_unknown_id_skipped() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        let claimed = s.xclaim("g", "c2", &[StreamId::new(99, 0)]).unwrap();
        assert!(claimed.is_empty());
    }

    #[test]
    fn xpending_unknown_group_errors() {
        let s = Stream::new();
        let err = s.xpending_count("missing").unwrap_err();
        assert_eq!(err, StreamError::NoSuchGroup("missing".into()));
    }

    #[test]
    fn xreadgroup_count_limits_delivery() {
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        for i in 0..10 {
            s.xadd(Some(StreamId::new(i, 0)), fields(&[]), 0).unwrap();
        }
        let out = s.xreadgroup("g", "c", Some(3), true).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn xreadgroup_advances_last_delivered_id() {
        // XGROUP CREATE start=0-0 means deliver only ids > 0-0, so we use ids 1..=5.
        let mut s = Stream::new();
        s.xgroup_create("g", StreamId::MIN).unwrap();
        for i in 1..=5 {
            s.xadd(Some(StreamId::new(i, 0)), fields(&[]), 0).unwrap();
        }
        let first = s.xreadgroup("g", "c", Some(2), true).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, StreamId::new(1, 0));
        // Next read continues from where we left off.
        let out = s.xreadgroup("g", "c", Some(10), true).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, StreamId::new(3, 0));
    }

    #[test]
    fn xrange_empty_stream() {
        let s = Stream::new();
        assert!(s.xrange(StreamId::MIN, StreamId::MAX).is_empty());
    }
}
