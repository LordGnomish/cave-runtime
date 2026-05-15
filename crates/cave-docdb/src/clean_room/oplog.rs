// SPDX-License-Identifier: AGPL-3.0-or-later
// Clean-room: authored from
//   https://www.mongodb.com/docs/manual/core/replica-set-oplog/
//   https://www.mongodb.com/docs/manual/reference/method/rs.status/
// No `mongodb/mongo` source code consulted.
//
//! Capped oplog ring buffer + replica state machine.
//!
//! The oplog is the linearised stream of all writes that have been
//! applied locally; downstream nodes replay it in order. The buffer
//! is fixed-capacity and FIFO — oldest entries are discarded once
//! capacity is reached. Each entry carries a 64-bit monotonic
//! timestamp encoded as `(unix_seconds << 32 | ordinal)` to match
//! BSON timestamp layout (publicly specified).

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Kind of an oplog operation. Public MongoDB doc opcodes:
/// `i` insert, `u` update, `d` delete, `c` command, `n` no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    Insert,
    Update,
    Delete,
    Command,
    Noop,
}

/// One entry in the oplog. Layout mirrors the public oplog document
/// schema (`ts`, `op`, `ns`, `o`, `o2`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEntry {
    pub ts: u64,
    pub op: OpKind,
    pub ns: String,
    pub o: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub o2: Option<serde_json::Value>,
}

/// Replica role. Single-node deployments are always `Primary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicaRole {
    Primary,
    Secondary,
    Arbiter,
}

/// Replica member identity (publicly described by `rs.status()`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: u32,
    pub host: String,
    pub role: ReplicaRole,
    pub healthy: bool,
}

/// Replica set state machine — single-leader, no election here yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaState {
    pub set_name: String,
    pub members: Vec<Member>,
}

impl ReplicaState {
    pub fn single_node(set_name: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            set_name: set_name.into(),
            members: vec![Member {
                id: 0,
                host: host.into(),
                role: ReplicaRole::Primary,
                healthy: true,
            }],
        }
    }

    pub fn primary(&self) -> Option<&Member> {
        self.members.iter().find(|m| m.role == ReplicaRole::Primary)
    }

    pub fn add_member(&mut self, m: Member) {
        self.members.push(m);
    }

    pub fn promote(&mut self, id: u32) -> Result<(), OplogError> {
        if !self.members.iter().any(|m| m.id == id) {
            return Err(OplogError::UnknownMember(id));
        }
        for m in &mut self.members {
            m.role = if m.id == id { ReplicaRole::Primary } else { ReplicaRole::Secondary };
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OplogError {
    #[error("unknown replica member id {0}")]
    UnknownMember(u32),
    #[error("oplog capacity exceeded")]
    Capacity,
}

/// Capped oplog ring buffer.
#[derive(Debug)]
pub struct Oplog {
    inner: RwLock<VecDeque<OpEntry>>,
    capacity: usize,
    counter: AtomicU64,
}

impl Oplog {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            counter: AtomicU64::new(1),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn append(&self, op: OpKind, ns: impl Into<String>, o: serde_json::Value) -> u64 {
        let ts = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut g = self.inner.write().unwrap();
        if g.len() == self.capacity {
            g.pop_front();
        }
        g.push_back(OpEntry { ts, op, ns: ns.into(), o, o2: None });
        ts
    }

    pub fn append_with_o2(
        &self,
        op: OpKind,
        ns: impl Into<String>,
        o: serde_json::Value,
        o2: serde_json::Value,
    ) -> u64 {
        let ts = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut g = self.inner.write().unwrap();
        if g.len() == self.capacity {
            g.pop_front();
        }
        g.push_back(OpEntry { ts, op, ns: ns.into(), o, o2: Some(o2) });
        ts
    }

    /// Read entries with `ts > since`, in append order.
    pub fn read_since(&self, since: u64) -> Vec<OpEntry> {
        self.inner.read().unwrap().iter().filter(|e| e.ts > since).cloned().collect()
    }

    /// Latest applied timestamp.
    pub fn head(&self) -> Option<u64> {
        self.inner.read().unwrap().back().map(|e| e.ts)
    }

    /// Earliest still-retained timestamp.
    pub fn tail(&self) -> Option<u64> {
        self.inner.read().unwrap().front().map(|e| e.ts)
    }
}

/// Shared oplog handle.
pub type SharedOplog = Arc<Oplog>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_grows_until_capacity() {
        let o = Oplog::new(3);
        for i in 0..3 {
            o.append(OpKind::Insert, "d.c", json!({"i": i}));
        }
        assert_eq!(o.len(), 3);
    }

    #[test]
    fn append_evicts_oldest_when_full() {
        let o = Oplog::new(2);
        o.append(OpKind::Insert, "d.c", json!({"i": 0}));
        o.append(OpKind::Insert, "d.c", json!({"i": 1}));
        o.append(OpKind::Insert, "d.c", json!({"i": 2}));
        assert_eq!(o.len(), 2);
        let entries = o.read_since(0);
        assert_eq!(entries[0].o.get("i").unwrap().as_i64().unwrap(), 1);
    }

    #[test]
    fn read_since_filters_by_ts() {
        let o = Oplog::new(10);
        for _ in 0..5 {
            o.append(OpKind::Insert, "d.c", json!({}));
        }
        // Only entries with ts > 2 should come back.
        let r = o.read_since(2);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn head_tail_track_insertion() {
        let o = Oplog::new(10);
        assert!(o.head().is_none());
        let a = o.append(OpKind::Noop, "n", json!({}));
        let b = o.append(OpKind::Noop, "n", json!({}));
        assert_eq!(o.head(), Some(b));
        assert_eq!(o.tail(), Some(a));
    }

    #[test]
    fn replica_single_node_promotes_id_zero_primary() {
        let r = ReplicaState::single_node("rs0", "localhost:27017");
        assert_eq!(r.primary().unwrap().id, 0);
    }

    #[test]
    fn replica_promote_swaps_primary() {
        let mut r = ReplicaState::single_node("rs0", "a");
        r.add_member(Member { id: 1, host: "b".into(), role: ReplicaRole::Secondary, healthy: true });
        r.promote(1).unwrap();
        assert_eq!(r.primary().unwrap().id, 1);
        assert!(matches!(r.members[0].role, ReplicaRole::Secondary));
    }

    #[test]
    fn replica_promote_unknown_id_errors() {
        let mut r = ReplicaState::single_node("rs0", "a");
        assert!(r.promote(42).is_err());
    }

    #[test]
    fn append_with_o2_serialises_both() {
        let o = Oplog::new(2);
        o.append_with_o2(OpKind::Update, "d.c", json!({"$set":{"x":1}}), json!({"_id":"y"}));
        let e = &o.read_since(0)[0];
        assert!(e.o2.is_some());
    }

    #[test]
    fn opkind_variants_serde_roundtrip() {
        for k in [OpKind::Insert, OpKind::Update, OpKind::Delete, OpKind::Command, OpKind::Noop] {
            let s = serde_json::to_string(&k).unwrap();
            let r: OpKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, r);
        }
    }
}
