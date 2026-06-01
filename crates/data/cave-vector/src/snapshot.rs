// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Collection snapshots.
//!
//! Port of the Qdrant `lib/collection/src/collection/snapshots.rs` surface,
//! reduced to an in-memory artifact: a [`Snapshot`] captures a collection's
//! schema + points plus a content checksum (Qdrant ships a `.snapshot`
//! tarball with a sibling `.checksum`). [`SnapshotStore`] is the named
//! registry create/list/restore/delete operate against.

use crate::collection::Collection;
use crate::error::VectorError;
use crate::models::{Point, VectorParams};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn checksum(schema: &VectorParams, points: &[Point]) -> u64 {
    // canonical: serialize schema + each point (points are already id-sorted by
    // the BTreeMap iteration that produced them) and FNV-1a the bytes.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |s: &str| {
        for &b in s.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(&serde_json::to_string(schema).unwrap_or_default());
    for p in points {
        feed(&serde_json::to_string(p).unwrap_or_default());
    }
    h
}

/// A point-in-time capture of a collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Snapshot name.
    pub name: String,
    /// Source collection name.
    pub collection: String,
    /// Creation time (caller-supplied unix seconds — keeps this deterministic).
    pub created_unix: u64,
    /// Captured schema.
    pub schema: VectorParams,
    /// Captured points (id-sorted).
    pub points: Vec<Point>,
    /// FNV-1a checksum over schema + points.
    pub checksum: u64,
}

impl Snapshot {
    /// Recompute the checksum and compare — detects corruption/tampering.
    pub fn verify(&self) -> bool {
        checksum(&self.schema, &self.points) == self.checksum
    }
}

/// Named registry of snapshots.
#[derive(Debug, Default)]
pub struct SnapshotStore {
    snaps: HashMap<String, Snapshot>,
}

impl SnapshotStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture `collection` as a new snapshot. Errors if the name is taken.
    pub fn create(
        &mut self,
        name: &str,
        collection: &str,
        c: &Collection,
        now_unix: u64,
    ) -> Result<&Snapshot, VectorError> {
        if self.snaps.contains_key(name) {
            return Err(VectorError::Invalid(format!("snapshot {name:?} already exists")));
        }
        let points: Vec<Point> = c.points.values().cloned().collect();
        let snap = Snapshot {
            name: name.to_string(),
            collection: collection.to_string(),
            created_unix: now_unix,
            schema: c.params.clone(),
            checksum: checksum(&c.params, &points),
            points,
        };
        Ok(self.snaps.entry(name.to_string()).or_insert(snap))
    }

    /// Sorted snapshot names.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.snaps.keys().cloned().collect();
        names.sort();
        names
    }

    /// Fetch a snapshot.
    pub fn get(&self, name: &str) -> Option<&Snapshot> {
        self.snaps.get(name)
    }

    /// Delete a snapshot; returns whether it existed.
    pub fn delete(&mut self, name: &str) -> bool {
        self.snaps.remove(name).is_some()
    }

    /// Rebuild a [`Collection`] from a snapshot, verifying the checksum first.
    pub fn restore(&self, name: &str) -> Result<Collection, VectorError> {
        let snap = self
            .snaps
            .get(name)
            .ok_or_else(|| VectorError::Invalid(format!("snapshot {name:?} not found")))?;
        if !snap.verify() {
            return Err(VectorError::Invalid(format!("snapshot {name:?} checksum mismatch")));
        }
        let mut c = Collection::new(snap.schema.clone());
        for p in &snap.points {
            c.upsert(p.clone())?;
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Distance, Payload, PointId};

    fn seeded_collection() -> Collection {
        let mut c = Collection::new(VectorParams {
            size: 2,
            distance: Distance::Cosine,
            hnsw_config: None,
            quantization: None,
        });
        c.upsert(Point { id: PointId::Num(1), vector: vec![1.0, 0.0], payload: Payload::new() })
            .unwrap();
        c.upsert(Point { id: PointId::Num(2), vector: vec![0.0, 1.0], payload: Payload::new() })
            .unwrap();
        c
    }

    #[test]
    fn create_captures_schema_and_points() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        let snap = s.create("snap-1", "docs", &c, 1_700_000_000).unwrap();
        assert_eq!(snap.collection, "docs");
        assert_eq!(snap.created_unix, 1_700_000_000);
        assert_eq!(snap.schema.size, 2);
        assert_eq!(snap.points.len(), 2);
        assert!(snap.checksum != 0);
    }

    #[test]
    fn create_duplicate_name_errors() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        s.create("snap-1", "docs", &c, 1).unwrap();
        assert!(s.create("snap-1", "docs", &c, 2).is_err());
    }

    #[test]
    fn list_and_delete() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        s.create("a", "docs", &c, 1).unwrap();
        s.create("b", "docs", &c, 1).unwrap();
        assert_eq!(s.list(), vec!["a".to_string(), "b".to_string()]);
        assert!(s.delete("a"));
        assert!(!s.delete("a"));
        assert_eq!(s.list(), vec!["b".to_string()]);
    }

    #[test]
    fn verify_detects_tampering() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        let snap = s.create("snap-1", "docs", &c, 1).unwrap().clone();
        assert!(snap.verify());
        let mut tampered = snap.clone();
        tampered.points[0].vector = vec![9.9, 9.9];
        assert!(!tampered.verify());
    }

    #[test]
    fn restore_round_trips_collection() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        s.create("snap-1", "docs", &c, 1).unwrap();
        let restored = s.restore("snap-1").unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.params.distance, Distance::Cosine);
        assert_eq!(restored.get(&PointId::Num(1)).unwrap().vector, vec![1.0, 0.0]);
    }

    #[test]
    fn restore_missing_errors() {
        let s = SnapshotStore::new();
        assert!(s.restore("nope").is_err());
    }

    #[test]
    fn restore_rejects_corrupted_checksum() {
        let mut s = SnapshotStore::new();
        let c = seeded_collection();
        s.create("snap-1", "docs", &c, 1).unwrap();
        // corrupt the stored snapshot's checksum.
        if let Some(snap) = s.snaps.get_mut("snap-1") {
            snap.checksum ^= 0xDEAD;
        }
        assert!(s.restore("snap-1").is_err());
    }
}
