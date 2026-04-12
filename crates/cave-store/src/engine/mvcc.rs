//! MVCC key-value store with revision tracking, leases, and watches.

use std::{
    collections::{BTreeMap, HashMap},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::broadcast;

use crate::error::{Result, StoreError};

// ─── Core types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KeyValue {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub create_revision: i64,
    pub mod_revision: i64,
    pub version: i64,
    pub lease: i64,
}

/// A single revision snapshot for a key (None value = tombstone).
#[derive(Debug, Clone)]
struct MvccRevision {
    revision: i64,
    value: Option<Vec<u8>>,
    lease_id: i64,
}

#[derive(Debug, Clone)]
pub struct LeaseInfo {
    pub ttl: i64,
    pub granted_at: u64,
    pub keys: Vec<Vec<u8>>,
}

impl LeaseInfo {
    pub fn remaining_ttl(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let elapsed = now.saturating_sub(self.granted_at) as i64;
        (self.ttl - elapsed).max(0)
    }

    pub fn is_expired(&self) -> bool {
        self.remaining_ttl() == 0
    }
}

// ─── Watch types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub watch_id: i64,
    pub event_type: WatchEventType,
    pub kv: KeyValue,
    pub prev_kv: Option<KeyValue>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WatchEventType {
    Put,
    Delete,
}

#[derive(Debug, Clone)]
pub struct WatchFilter {
    pub key: Vec<u8>,
    pub range_end: Vec<u8>,
    pub start_rev: i64,
    pub progress_notify: bool,
    pub prev_kv: bool,
    pub no_put: bool,
    pub no_delete: bool,
}

impl WatchFilter {
    pub fn matches(&self, key: &[u8], event_type: &WatchEventType) -> bool {
        if event_type == &WatchEventType::Put && self.no_put {
            return false;
        }
        if event_type == &WatchEventType::Delete && self.no_delete {
            return false;
        }
        key_in_range(key, &self.key, &self.range_end)
    }
}

// ─── Compare / Txn types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CompareResult {
    Equal,
    Greater,
    Less,
    NotEqual,
}

#[derive(Debug, Clone)]
pub enum CompareTarget {
    Version(i64),
    CreateRevision(i64),
    ModRevision(i64),
    Value(Vec<u8>),
    Lease(i64),
}

#[derive(Debug, Clone)]
pub struct Compare {
    pub key: Vec<u8>,
    pub range_end: Vec<u8>,
    pub result: CompareResult,
    pub target: CompareTarget,
}

#[derive(Debug, Clone)]
pub enum TxnOp {
    Range {
        key: Vec<u8>,
        range_end: Vec<u8>,
        limit: i64,
        revision: Option<i64>,
        keys_only: bool,
        count_only: bool,
    },
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        lease_id: i64,
        prev_kv: bool,
    },
    Delete {
        key: Vec<u8>,
        range_end: Vec<u8>,
        prev_kv: bool,
    },
}

#[derive(Debug, Clone)]
pub enum TxnResult {
    Range { kvs: Vec<KeyValue>, count: i64, more: bool },
    Put { prev_kv: Option<KeyValue> },
    Delete { deleted: i64, prev_kvs: Vec<KeyValue> },
}

// ─── MVCC Store ───────────────────────────────────────────────────────────────

pub struct MvccStore {
    /// key → sorted list of MvccRevision (oldest first)
    data: BTreeMap<Vec<u8>, Vec<MvccRevision>>,
    compacted_rev: i64,
    current_rev: i64,
    leases: HashMap<i64, LeaseInfo>,
    watches: HashMap<i64, WatchFilter>,
    next_watch_id: i64,
    watch_tx: broadcast::Sender<WatchEvent>,
}

impl MvccStore {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(4096);
        Self {
            data: BTreeMap::new(),
            compacted_rev: 0,
            current_rev: 0,
            leases: HashMap::new(),
            watches: HashMap::new(),
            next_watch_id: 1,
            watch_tx,
        }
    }

    pub fn current_revision(&self) -> i64 {
        self.current_rev
    }

    pub fn compacted_revision(&self) -> i64 {
        self.compacted_rev
    }

    // ── Put ──────────────────────────────────────────────────────────────────

    /// Returns (new_revision, Option<prev_kv>)
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>, lease_id: i64, prev_kv: bool) -> (i64, Option<KeyValue>) {
        self.current_rev += 1;
        let rev = self.current_rev;

        let prev = if prev_kv { self.get_latest(&key) } else { None };

        {
            let revisions = self.data.entry(key.clone()).or_default();
            revisions.push(MvccRevision { revision: rev, value: Some(value.clone()), lease_id });
        }

        // Track key on lease
        if lease_id != 0 {
            if let Some(lease) = self.leases.get_mut(&lease_id) {
                if !lease.keys.contains(&key) {
                    lease.keys.push(key.clone());
                }
            }
        }

        let event_kv = self.get_latest(&key);
        if let Some(kv) = event_kv {
            let event = WatchEvent {
                watch_id: 0, // filled by watch layer
                event_type: WatchEventType::Put,
                prev_kv: prev.clone(),
                kv,
                revision: rev,
            };
            self.broadcast_event(event);
        }

        (rev, prev)
    }

    // ── Get ──────────────────────────────────────────────────────────────────

    pub fn get(&self, key: &[u8], revision: Option<i64>) -> Option<KeyValue> {
        let revs = self.data.get(key)?;
        self.resolve_at(key, revs, revision)
    }

    fn get_latest(&self, key: &[u8]) -> Option<KeyValue> {
        self.get(key, None)
    }

    // ── Range ─────────────────────────────────────────────────────────────────

    /// Returns (kvs, total_count). Handles prefix and range queries.
    pub fn range(
        &self,
        key: &[u8],
        range_end: &[u8],
        revision: Option<i64>,
        limit: i64,
        keys_only: bool,
        count_only: bool,
    ) -> (Vec<KeyValue>, i64) {
        let at_rev = revision.unwrap_or(self.current_rev);

        let iter: Box<dyn Iterator<Item = (&Vec<u8>, &Vec<MvccRevision>)>> =
            if range_end.is_empty() {
                // Single key
                if let Some(revs) = self.data.get(key) {
                    Box::new(std::iter::once((
                        self.data.keys().find(|k| k.as_slice() == key).unwrap(),
                        revs,
                    )))
                } else {
                    Box::new(std::iter::empty())
                }
            } else if range_end == [0u8] {
                // Full range (etcd uses "\0" to mean "all keys ≥ key")
                Box::new(self.data.range(key.to_vec()..))
            } else {
                Box::new(self.data.range(key.to_vec()..range_end.to_vec()))
            };

        let mut kvs = Vec::new();
        let mut count = 0i64;

        for (k, revs) in iter {
            if let Some(kv) = self.resolve_at(k, revs, Some(at_rev)) {
                count += 1;
                if !count_only {
                    if limit > 0 && kvs.len() as i64 >= limit {
                        // more = true (not returned here but caller can check count > len)
                        continue;
                    }
                    let mut kv = kv;
                    if keys_only {
                        kv.value = vec![];
                    }
                    kvs.push(kv);
                }
            }
        }

        (kvs, count)
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    pub fn delete(&mut self, key: &[u8], prev_kv: bool) -> (i64, Option<KeyValue>) {
        let prev = if prev_kv { self.get_latest(key) } else { None };

        if self.data.contains_key(key) {
            self.current_rev += 1;
            let rev = self.current_rev;
            let revisions = self.data.entry(key.to_vec()).or_default();
            revisions.push(MvccRevision { revision: rev, value: None, lease_id: 0 });

            if let Some(prev_kv) = &prev {
                let event = WatchEvent {
                    watch_id: 0,
                    event_type: WatchEventType::Delete,
                    prev_kv: Some(prev_kv.clone()),
                    kv: KeyValue {
                        key: key.to_vec(),
                        value: vec![],
                        create_revision: prev_kv.create_revision,
                        mod_revision: rev,
                        version: 0,
                        lease: 0,
                    },
                    revision: rev,
                };
                self.broadcast_event(event);
            }

            (rev, prev)
        } else {
            (self.current_rev, None)
        }
    }

    pub fn delete_range(&mut self, key: &[u8], range_end: &[u8], prev_kv: bool) -> (i64, Vec<KeyValue>) {
        let keys_to_delete: Vec<Vec<u8>> = if range_end.is_empty() {
            if self.data.contains_key(key) { vec![key.to_vec()] } else { vec![] }
        } else if range_end == [0u8] {
            self.data.range(key.to_vec()..).map(|(k, _)| k.clone()).collect()
        } else {
            self.data.range(key.to_vec()..range_end.to_vec()).map(|(k, _)| k.clone()).collect()
        };

        let mut prev_kvs = Vec::new();
        for k in keys_to_delete {
            let (_, prev) = self.delete(&k, prev_kv);
            if let Some(p) = prev {
                prev_kvs.push(p);
            }
        }
        (self.current_rev, prev_kvs)
    }

    // ── Compact ───────────────────────────────────────────────────────────────

    pub fn compact(&mut self, revision: i64) -> Result<()> {
        if revision <= self.compacted_rev {
            return Err(StoreError::RevisionCompacted(revision));
        }
        if revision > self.current_rev {
            return Err(StoreError::InvalidRequest(format!(
                "compact revision {revision} > current {}", self.current_rev
            )));
        }

        for revisions in self.data.values_mut() {
            // Keep the last entry ≤ revision (the effective state at that revision)
            // and all entries after revision
            let last_before = revisions.iter().rposition(|r| r.revision <= revision);
            if let Some(pos) = last_before {
                revisions.drain(0..pos);
            }
        }

        // Remove keys whose latest entry is a tombstone at or before compacted rev
        self.data.retain(|_, revisions| {
            !revisions.is_empty()
                && !(revisions.last().map(|r| r.value.is_none() && r.revision <= revision).unwrap_or(false))
        });

        self.compacted_rev = revision;
        Ok(())
    }

    // ── Transactions ─────────────────────────────────────────────────────────

    pub fn txn(&mut self, cmps: Vec<Compare>, success: Vec<TxnOp>, failure: Vec<TxnOp>) -> (bool, Vec<TxnResult>) {
        let succeeded = self.evaluate_compares(&cmps);
        let ops = if succeeded { success } else { failure };
        let results = self.apply_txn_ops(ops);
        (succeeded, results)
    }

    fn evaluate_compares(&self, cmps: &[Compare]) -> bool {
        for cmp in cmps {
            if !self.evaluate_compare(cmp) {
                return false;
            }
        }
        true
    }

    fn evaluate_compare(&self, cmp: &Compare) -> bool {
        let keys: Vec<&Vec<u8>> = if cmp.range_end.is_empty() {
            self.data.keys().filter(|k| k.as_slice() == cmp.key.as_slice()).collect()
        } else if cmp.range_end == [0u8] {
            self.data.range(cmp.key.clone()..).map(|(k, _)| k).collect()
        } else {
            self.data.range(cmp.key.clone()..cmp.range_end.clone()).map(|(k, _)| k).collect()
        };

        // For range compares, all keys must satisfy
        if keys.is_empty() {
            // Key doesn't exist — version/create/mod = 0
            return match &cmp.target {
                CompareTarget::Version(v) => compare_values(0i64, *v, &cmp.result),
                CompareTarget::CreateRevision(v) => compare_values(0i64, *v, &cmp.result),
                CompareTarget::ModRevision(v) => compare_values(0i64, *v, &cmp.result),
                CompareTarget::Value(v) => compare_values(0i64, v.len() as i64, &cmp.result),
                CompareTarget::Lease(v) => compare_values(0i64, *v, &cmp.result),
            };
        }

        for key in keys {
            let kv = match self.get_latest(key) {
                Some(kv) => kv,
                None => return false,
            };
            let ok = match &cmp.target {
                CompareTarget::Version(v) => compare_values(kv.version, *v, &cmp.result),
                CompareTarget::CreateRevision(v) => compare_values(kv.create_revision, *v, &cmp.result),
                CompareTarget::ModRevision(v) => compare_values(kv.mod_revision, *v, &cmp.result),
                CompareTarget::Value(v) => compare_bytes(&kv.value, v, &cmp.result),
                CompareTarget::Lease(v) => compare_values(kv.lease, *v, &cmp.result),
            };
            if !ok {
                return false;
            }
        }
        true
    }

    fn apply_txn_ops(&mut self, ops: Vec<TxnOp>) -> Vec<TxnResult> {
        ops.into_iter().map(|op| match op {
            TxnOp::Range { key, range_end, limit, revision, keys_only, count_only } => {
                let (kvs, count) = self.range(&key, &range_end, revision, limit, keys_only, count_only);
                let more = count > kvs.len() as i64;
                TxnResult::Range { kvs, count, more }
            }
            TxnOp::Put { key, value, lease_id, prev_kv } => {
                let (_, prev) = self.put(key, value, lease_id, prev_kv);
                TxnResult::Put { prev_kv: prev }
            }
            TxnOp::Delete { key, range_end, prev_kv } => {
                let (_, prev_kvs) = self.delete_range(&key, &range_end, prev_kv);
                let deleted = prev_kvs.len() as i64;
                TxnResult::Delete { deleted, prev_kvs }
            }
        }).collect()
    }

    // ── Leases ────────────────────────────────────────────────────────────────

    pub fn lease_grant(&mut self, id: i64, ttl: i64) -> i64 {
        let lease_id = if id == 0 {
            self.next_lease_id()
        } else {
            id
        };
        let granted_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.leases.insert(lease_id, LeaseInfo { ttl, granted_at, keys: vec![] });
        lease_id
    }

    pub fn lease_revoke(&mut self, id: i64) -> Result<Vec<Vec<u8>>> {
        let lease = self.leases.remove(&id).ok_or(StoreError::LeaseNotFound(id))?;
        let keys = lease.keys.clone();
        for key in &keys {
            self.delete(key, false);
        }
        Ok(keys)
    }

    pub fn lease_keep_alive(&mut self, id: i64) -> Result<i64> {
        let granted_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let lease = self.leases.get_mut(&id).ok_or(StoreError::LeaseNotFound(id))?;
        lease.granted_at = granted_at;
        Ok(lease.ttl)
    }

    pub fn lease_ttl(&self, id: i64) -> Result<(i64, i64, Vec<Vec<u8>>)> {
        let lease = self.leases.get(&id).ok_or(StoreError::LeaseNotFound(id))?;
        Ok((lease.ttl, lease.remaining_ttl(), lease.keys.clone()))
    }

    pub fn lease_list(&self) -> Vec<i64> {
        self.leases.keys().copied().collect()
    }

    pub fn expire_leases(&mut self) -> Vec<i64> {
        let expired: Vec<i64> = self.leases
            .iter()
            .filter(|(_, l)| l.is_expired())
            .map(|(id, _)| *id)
            .collect();

        for id in &expired {
            let _ = self.lease_revoke(*id);
        }
        expired
    }

    fn next_lease_id(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i64;
        let mut id = now;
        while self.leases.contains_key(&id) {
            id += 1;
        }
        id
    }

    // ── Watches ───────────────────────────────────────────────────────────────

    pub fn watch_create(
        &mut self,
        key: Vec<u8>,
        range_end: Vec<u8>,
        start_rev: i64,
        progress_notify: bool,
        prev_kv: bool,
        no_put: bool,
        no_delete: bool,
        watch_id: i64,
    ) -> i64 {
        let id = if watch_id != 0 { watch_id } else {
            let id = self.next_watch_id;
            self.next_watch_id += 1;
            id
        };
        self.watches.insert(id, WatchFilter {
            key,
            range_end,
            start_rev,
            progress_notify,
            prev_kv,
            no_put,
            no_delete,
        });
        id
    }

    pub fn watch_cancel(&mut self, watch_id: i64) {
        self.watches.remove(&watch_id);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.watch_tx.subscribe()
    }

    fn broadcast_event(&self, mut event: WatchEvent) {
        // Fan out to all matching watches
        for (watch_id, filter) in &self.watches {
            if filter.matches(&event.kv.key, &event.event_type) {
                event.watch_id = *watch_id;
                let _ = self.watch_tx.send(event.clone());
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn resolve_at(&self, key: &[u8], revs: &[MvccRevision], at: Option<i64>) -> Option<KeyValue> {
        let at_rev = at.unwrap_or(self.current_rev);

        // Find the last revision ≤ at_rev
        let rev = revs.iter().rev().find(|r| r.revision <= at_rev)?;
        // Tombstone
        if rev.value.is_none() {
            return None;
        }

        let create_revision = revs.iter().find(|r| r.revision <= at_rev && r.value.is_some())?.revision;
        let version = revs.iter().filter(|r| r.revision <= at_rev && r.value.is_some()).count() as i64;

        Some(KeyValue {
            key: key.to_vec(),
            value: rev.value.clone().unwrap_or_default(),
            create_revision,
            mod_revision: rev.revision,
            version,
            lease: rev.lease_id,
        })
    }

    fn build_kv(&self, key: &[u8], revs: &[MvccRevision]) -> Option<KeyValue> {
        self.resolve_at(key, revs, None)
    }
}

// ─── Free helpers ─────────────────────────────────────────────────────────────

pub fn key_in_range(key: &[u8], range_start: &[u8], range_end: &[u8]) -> bool {
    if range_end.is_empty() {
        key == range_start
    } else if range_end == [0u8] {
        key >= range_start
    } else {
        key >= range_start && key < range_end
    }
}

fn compare_values<T: Ord>(actual: T, target: T, result: &CompareResult) -> bool {
    match result {
        CompareResult::Equal => actual == target,
        CompareResult::Greater => actual > target,
        CompareResult::Less => actual < target,
        CompareResult::NotEqual => actual != target,
    }
}

fn compare_bytes(actual: &[u8], target: &[u8], result: &CompareResult) -> bool {
    match result {
        CompareResult::Equal => actual == target,
        CompareResult::Greater => actual > target,
        CompareResult::Less => actual < target,
        CompareResult::NotEqual => actual != target,
    }
}
