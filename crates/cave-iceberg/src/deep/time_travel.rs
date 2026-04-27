//! Iceberg time-travel queries — snapshot lookup by id, branch, tag, or
//! wall-clock timestamp.
//!
//! Mirrors the spec at <https://iceberg.apache.org/spec/#snapshots>:
//!
//! * Snapshots form a linked list via `parent_snapshot_id`.
//! * Branches and tags are named refs that point at a specific snapshot id.
//! * Time-travel selects the snapshot whose `timestamp_ms` ≤ the requested
//!   timestamp on the chosen ref's history.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TimeTravelError {
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(i64),
    #[error("ref not found: {0}")]
    RefNotFound(String),
    #[error("no snapshot at or before timestamp {0}")]
    NoSnapshotAtTime(u64),
    #[error("ref already exists: {0}")]
    RefExists(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    pub timestamp_ms: u64,
    pub manifest_list: String,
    pub summary: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Branch,
    Tag,
}

#[derive(Debug, Clone)]
pub struct SnapshotRef {
    pub name: String,
    pub kind: RefKind,
    pub snapshot_id: i64,
    /// For branches: maximum age in days the snapshot can be before being
    /// expired. None = no expiration.
    pub max_age_days: Option<i32>,
}

#[derive(Debug, Default)]
pub struct SnapshotRegistry {
    snapshots: BTreeMap<i64, Snapshot>,
    refs: BTreeMap<String, SnapshotRef>,
}

impl SnapshotRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, snap: Snapshot) {
        self.snapshots.insert(snap.snapshot_id, snap);
    }

    pub fn count(&self) -> usize {
        self.snapshots.len()
    }

    pub fn create_ref(&mut self, r: SnapshotRef) -> Result<(), TimeTravelError> {
        if self.refs.contains_key(&r.name) {
            return Err(TimeTravelError::RefExists(r.name));
        }
        if !self.snapshots.contains_key(&r.snapshot_id) {
            return Err(TimeTravelError::SnapshotNotFound(r.snapshot_id));
        }
        self.refs.insert(r.name.clone(), r);
        Ok(())
    }

    pub fn get_ref(&self, name: &str) -> Option<&SnapshotRef> {
        self.refs.get(name)
    }

    pub fn move_branch(&mut self, name: &str, snapshot_id: i64) -> Result<(), TimeTravelError> {
        let r = self
            .refs
            .get_mut(name)
            .ok_or_else(|| TimeTravelError::RefNotFound(name.into()))?;
        if r.kind != RefKind::Branch {
            return Err(TimeTravelError::RefExists(format!("{} is a tag", name)));
        }
        if !self.snapshots.contains_key(&snapshot_id) {
            return Err(TimeTravelError::SnapshotNotFound(snapshot_id));
        }
        r.snapshot_id = snapshot_id;
        Ok(())
    }

    pub fn drop_ref(&mut self, name: &str) -> bool {
        self.refs.remove(name).is_some()
    }

    /// Walk parent links from `head_id` and return the linear ancestor list.
    pub fn ancestors(&self, head_id: i64) -> Result<Vec<&Snapshot>, TimeTravelError> {
        let mut current = self
            .snapshots
            .get(&head_id)
            .ok_or(TimeTravelError::SnapshotNotFound(head_id))?;
        let mut out = Vec::new();
        loop {
            out.push(current);
            match current.parent_snapshot_id {
                Some(p) => match self.snapshots.get(&p) {
                    Some(parent) => current = parent,
                    None => break,
                },
                None => break,
            }
        }
        Ok(out)
    }

    /// Return the latest snapshot in the ancestry of `head_id` whose
    /// `timestamp_ms <= ts_ms`.
    pub fn snapshot_at(&self, head_id: i64, ts_ms: u64) -> Result<&Snapshot, TimeTravelError> {
        let chain = self.ancestors(head_id)?;
        chain
            .into_iter()
            .find(|s| s.timestamp_ms <= ts_ms)
            .ok_or(TimeTravelError::NoSnapshotAtTime(ts_ms))
    }

    /// Returns the snapshot pointed at by a ref + optional time-travel.
    pub fn resolve(&self, ref_name: &str, ts_ms: Option<u64>) -> Result<&Snapshot, TimeTravelError> {
        let r = self
            .refs
            .get(ref_name)
            .ok_or_else(|| TimeTravelError::RefNotFound(ref_name.into()))?;
        match ts_ms {
            None => self
                .snapshots
                .get(&r.snapshot_id)
                .ok_or(TimeTravelError::SnapshotNotFound(r.snapshot_id)),
            Some(ts) => self.snapshot_at(r.snapshot_id, ts),
        }
    }

    /// Expire snapshots older than `cutoff_ms` that are not protected by any ref.
    pub fn expire(&mut self, cutoff_ms: u64) -> Vec<i64> {
        let protected: std::collections::BTreeSet<i64> =
            self.refs.values().map(|r| r.snapshot_id).collect();
        let to_remove: Vec<i64> = self
            .snapshots
            .iter()
            .filter(|(id, s)| s.timestamp_ms < cutoff_ms && !protected.contains(id))
            .map(|(id, _)| *id)
            .collect();
        for id in &to_remove {
            self.snapshots.remove(id);
        }
        to_remove
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(id: i64, parent: Option<i64>, ts: u64) -> Snapshot {
        Snapshot {
            snapshot_id: id,
            parent_snapshot_id: parent,
            timestamp_ms: ts,
            manifest_list: format!("ml-{}.avro", id),
            summary: BTreeMap::new(),
        }
    }

    #[test]
    fn append_then_count() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        assert_eq!(r.count(), 2);
    }

    #[test]
    fn ancestors_walks_parent_chain() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        r.append(snap(3, Some(2), 300));
        let chain = r.ancestors(3).unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].snapshot_id, 3);
        assert_eq!(chain[2].snapshot_id, 1);
    }

    #[test]
    fn snapshot_at_returns_latest_before_ts() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        r.append(snap(3, Some(2), 300));
        let s = r.snapshot_at(3, 250).unwrap();
        assert_eq!(s.snapshot_id, 2);
    }

    #[test]
    fn snapshot_at_no_match_errors() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        let err = r.snapshot_at(1, 50).unwrap_err();
        assert!(matches!(err, TimeTravelError::NoSnapshotAtTime(_)));
    }

    #[test]
    fn create_branch_and_resolve() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 1,
            max_age_days: None,
        })
        .unwrap();
        let s = r.resolve("main", None).unwrap();
        assert_eq!(s.snapshot_id, 1);
    }

    #[test]
    fn create_ref_unknown_snapshot_errors() {
        let mut r = SnapshotRegistry::new();
        let err = r
            .create_ref(SnapshotRef {
                name: "x".into(),
                kind: RefKind::Branch,
                snapshot_id: 99,
                max_age_days: None,
            })
            .unwrap_err();
        assert_eq!(err, TimeTravelError::SnapshotNotFound(99));
    }

    #[test]
    fn create_duplicate_ref_errors() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 1,
            max_age_days: None,
        })
        .unwrap();
        let err = r
            .create_ref(SnapshotRef {
                name: "main".into(),
                kind: RefKind::Tag,
                snapshot_id: 1,
                max_age_days: None,
            })
            .unwrap_err();
        assert_eq!(err, TimeTravelError::RefExists("main".into()));
    }

    #[test]
    fn move_branch_advances_pointer() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 1,
            max_age_days: None,
        })
        .unwrap();
        r.move_branch("main", 2).unwrap();
        assert_eq!(r.get_ref("main").unwrap().snapshot_id, 2);
    }

    #[test]
    fn move_branch_unknown_ref_errors() {
        let mut r = SnapshotRegistry::new();
        let err = r.move_branch("nope", 1).unwrap_err();
        assert!(matches!(err, TimeTravelError::RefNotFound(_)));
    }

    #[test]
    fn drop_ref_returns_true() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 1,
            max_age_days: None,
        })
        .unwrap();
        assert!(r.drop_ref("main"));
    }

    #[test]
    fn resolve_branch_with_time_travel() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        r.append(snap(3, Some(2), 300));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 3,
            max_age_days: None,
        })
        .unwrap();
        let s = r.resolve("main", Some(150)).unwrap();
        assert_eq!(s.snapshot_id, 1);
    }

    #[test]
    fn expire_drops_old_unprotected_snapshots() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.append(snap(2, Some(1), 200));
        r.append(snap(3, Some(2), 300));
        r.create_ref(SnapshotRef {
            name: "main".into(),
            kind: RefKind::Branch,
            snapshot_id: 3,
            max_age_days: None,
        })
        .unwrap();
        let removed = r.expire(250);
        // snapshots 1 and 2 are older than 250 and not protected (only 3 is).
        assert_eq!(removed.len(), 2);
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn expire_protects_referenced_snapshots() {
        let mut r = SnapshotRegistry::new();
        r.append(snap(1, None, 100));
        r.create_ref(SnapshotRef {
            name: "release-1".into(),
            kind: RefKind::Tag,
            snapshot_id: 1,
            max_age_days: None,
        })
        .unwrap();
        let removed = r.expire(1000);
        assert!(removed.is_empty());
    }

    #[test]
    fn ancestors_unknown_head_errors() {
        let r = SnapshotRegistry::new();
        let err = r.ancestors(99).unwrap_err();
        assert!(matches!(err, TimeTravelError::SnapshotNotFound(_)));
    }
}
