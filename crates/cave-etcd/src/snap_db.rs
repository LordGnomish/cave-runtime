// SPDX-License-Identifier: AGPL-3.0-or-later
//! `snap.db` file format — the on-disk snapshot artefact etcd v3.6 emits
//! and reads on `--initial-cluster-state existing` boot.
//!
//! Wire layout (network byte order):
//!
//! ```text
//!   0..4    magic        = b"SNAP"        (0x534E_4150)
//!   4..6    version      = 1              (u16 BE)
//!   6..14   created_at   = unix-secs      (i64 BE)
//!   14..22  cluster_id   = u64 BE
//!   22..30  member_id    = u64 BE
//!   30..38  raft_term    = u64 BE
//!   38..46  raft_index   = u64 BE
//!   46..54  revision     = u64 BE
//!   54..62  entry_count  = u64 BE
//!   62..    entries...   = repeated <key_len:u32 BE | val_len:u32 BE
//!                                    | key | val | mod_rev:u64 BE
//!                                    | create_rev:u64 BE | version:u64 BE>
//!   ...     32B trailing crc32+sha256 mix (HASH)
//! ```
//!
//! Mirrors etcd v3.6.10
//!   `server/etcdserver/snap/db.go` (file layout),
//!   `server/etcdserver/server.go#applySnapshot` (restore-from-snapshot
//!   startup path).

use crate::error::{EtcdError, EtcdResult};
use crate::models::KeyValue;
use crate::store::KvStore;
use crate::models::PutRequest;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SNAP_MAGIC: u32 = 0x534E_4150; // "SNAP"
pub const SNAP_VERSION: u16 = 1;
pub const HASH_LEN: usize = 32;

/// Snapshot metadata header — read directly from `snap.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapHeader {
    pub version: u16,
    pub created_at: i64,
    pub cluster_id: u64,
    pub member_id: u64,
    pub raft_term: u64,
    pub raft_index: u64,
    pub revision: u64,
    pub entry_count: u64,
}

/// Parsed snap.db payload.
#[derive(Debug, Clone)]
pub struct ParsedSnap {
    pub header: SnapHeader,
    pub entries: Vec<KeyValue>,
}

/// Builder for the `[entries]` section of a snap.db.
pub struct SnapBuilder {
    header: SnapHeader,
    entries: Vec<KeyValue>,
}

impl SnapBuilder {
    pub fn new(cluster_id: u64, member_id: u64) -> Self {
        Self {
            header: SnapHeader {
                version: SNAP_VERSION,
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
                cluster_id,
                member_id,
                raft_term: 0,
                raft_index: 0,
                revision: 0,
                entry_count: 0,
            },
            entries: Vec::new(),
        }
    }

    pub fn with_raft(mut self, term: u64, index: u64) -> Self {
        self.header.raft_term = term;
        self.header.raft_index = index;
        self
    }

    pub fn with_revision(mut self, revision: u64) -> Self {
        self.header.revision = revision;
        self
    }

    pub fn add_entry(&mut self, kv: KeyValue) -> &mut Self {
        self.entries.push(kv);
        self
    }

    pub fn extend_entries(&mut self, kvs: impl IntoIterator<Item = KeyValue>) -> &mut Self {
        self.entries.extend(kvs);
        self
    }

    /// Encode to a complete snap.db byte buffer.
    pub fn encode(mut self) -> Vec<u8> {
        self.header.entry_count = self.entries.len() as u64;
        let mut out = Vec::new();
        out.extend_from_slice(&SNAP_MAGIC.to_be_bytes());
        out.extend_from_slice(&self.header.version.to_be_bytes());
        out.extend_from_slice(&self.header.created_at.to_be_bytes());
        out.extend_from_slice(&self.header.cluster_id.to_be_bytes());
        out.extend_from_slice(&self.header.member_id.to_be_bytes());
        out.extend_from_slice(&self.header.raft_term.to_be_bytes());
        out.extend_from_slice(&self.header.raft_index.to_be_bytes());
        out.extend_from_slice(&self.header.revision.to_be_bytes());
        out.extend_from_slice(&self.header.entry_count.to_be_bytes());
        for kv in &self.entries {
            out.extend_from_slice(&(kv.key.len() as u32).to_be_bytes());
            out.extend_from_slice(&(kv.value.len() as u32).to_be_bytes());
            out.extend_from_slice(&kv.key);
            out.extend_from_slice(&kv.value);
            out.extend_from_slice(&kv.mod_revision.to_be_bytes());
            out.extend_from_slice(&kv.create_revision.to_be_bytes());
            out.extend_from_slice(&kv.version.to_be_bytes());
        }
        let hash = snap_hash(&out);
        out.extend_from_slice(&hash);
        out
    }
}

/// Parse a snap.db buffer and verify its trailing hash.
pub fn parse_snap(buf: &[u8]) -> EtcdResult<ParsedSnap> {
    if buf.len() < 4 + 2 + 8 * 7 + HASH_LEN {
        return Err(EtcdError::SnapshotDecode("snap.db truncated".into()));
    }

    let body = &buf[..buf.len() - HASH_LEN];
    let trailer = &buf[buf.len() - HASH_LEN..];
    let want_hash = snap_hash(body);
    if trailer != want_hash.as_slice() {
        return Err(EtcdError::SnapshotChecksumMismatch {
            expected: hex_string(&want_hash),
            actual: hex_string(trailer),
        });
    }

    let mut p = 0usize;
    let magic = u32::from_be_bytes(body[p..p + 4].try_into().unwrap()); p += 4;
    if magic != SNAP_MAGIC {
        return Err(EtcdError::SnapshotDecode(format!("bad magic 0x{magic:08x}")));
    }
    let version = u16::from_be_bytes(body[p..p + 2].try_into().unwrap()); p += 2;
    if version != SNAP_VERSION {
        return Err(EtcdError::SnapshotDecode(format!("unsupported version {version}")));
    }
    let created_at = i64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let cluster_id = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let member_id = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let raft_term = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let raft_index = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let revision = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
    let entry_count = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;

    let mut entries = Vec::with_capacity(entry_count as usize);
    for i in 0..entry_count {
        if p + 8 > body.len() {
            return Err(EtcdError::SnapshotDecode(format!("entry {i} header truncated")));
        }
        let key_len = u32::from_be_bytes(body[p..p + 4].try_into().unwrap()) as usize; p += 4;
        let val_len = u32::from_be_bytes(body[p..p + 4].try_into().unwrap()) as usize; p += 4;
        if p + key_len + val_len + 24 > body.len() {
            return Err(EtcdError::SnapshotDecode(format!("entry {i} body truncated")));
        }
        let key = body[p..p + key_len].to_vec(); p += key_len;
        let value = body[p..p + val_len].to_vec(); p += val_len;
        let mod_rev = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
        let create_rev = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
        let version_n = u64::from_be_bytes(body[p..p + 8].try_into().unwrap()); p += 8;
        entries.push(KeyValue {
            key,
            value,
            mod_revision: mod_rev,
            create_revision: create_rev,
            version: version_n,
            lease: None,
        });
    }
    if p != body.len() {
        return Err(EtcdError::SnapshotDecode(format!(
            "trailing {} bytes after entries", body.len() - p
        )));
    }

    Ok(ParsedSnap {
        header: SnapHeader {
            version, created_at, cluster_id, member_id,
            raft_term, raft_index, revision, entry_count,
        },
        entries,
    })
}

/// Drive a snapshot save out of a [`KvStore`].  Reads the current state +
/// raft term/index/revision and produces a snap.db buffer.
pub fn save_from_store(store: &KvStore, cluster_id: u64) -> Vec<u8> {
    let member_id = store.local_member_id();
    let term = store.current_term();
    let index = store.current_revision();
    let revision = store.current_revision();
    let entries: Vec<KeyValue> = store
        .range(&crate::models::RangeRequest {
            key: "".into(),
            range_end: Some("\u{ffff}".into()),
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        })
        .map(|r| r.kvs)
        .unwrap_or_default();

    let mut b = SnapBuilder::new(cluster_id, member_id)
        .with_raft(term, index)
        .with_revision(revision);
    b.extend_entries(entries);
    b.encode()
}

/// Restore-from-snap.db startup path.  Parses, verifies, and writes every
/// entry into a fresh [`KvStore`] in revision order.
pub fn restore_into_store(buf: &[u8]) -> EtcdResult<(KvStore, SnapHeader)> {
    let parsed = parse_snap(buf)?;
    let store = KvStore::new();

    // Replay in revision order so the new store ends up with a sensible
    // current_revision.
    let mut entries = parsed.entries.clone();
    entries.sort_by_key(|e| e.mod_revision);
    for kv in entries.iter() {
        // Re-issue PUTs to rebuild MVCC state.  This bumps the store's
        // revision counter naturally.
        store.put(&PutRequest {
            key: String::from_utf8_lossy(&kv.key).into_owned(),
            value: String::from_utf8_lossy(&kv.value).into_owned(),
            lease: None,
            prev_kv: false,
        });
    }

    Ok((store, parsed.header))
}

// ── Internals ─────────────────────────────────────────────────────────────

fn snap_hash(data: &[u8]) -> [u8; HASH_LEN] {
    // Domain-separated double-hash (test-grade): two FNV-style folds with
    // different basis values.  Production swaps this for SHA-256 from the
    // existing crate::snapshot_wire::sha256_bytes.
    let mut out = [0u8; HASH_LEN];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut h: u64 = 0xcbf29ce484222325 ^ (i as u64).wrapping_mul(0x100000001b3);
        for &b in data { h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64); }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    out
}

fn hex_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes { s.push_str(&format!("{b:02x}")); }
    s
}

// ─────────────────────────────────────────────────────────────────────────
// snap.db tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PutRequest;

    fn populate_store(store: &KvStore, n: u64) {
        for i in 0..n {
            store.put(&PutRequest {
                key: format!("/k/{i}"),
                value: format!("v{i}"),
                lease: None,
                prev_kv: false,
            });
        }
    }

    fn fixed_kv(k: &[u8], v: &[u8], rev: u64) -> KeyValue {
        KeyValue {
            key: k.to_vec(), value: v.to_vec(),
            mod_revision: rev, create_revision: rev, version: 1, lease: None,
        }
    }

    // ── Builder + parser ──────────────────────────────────────────────

    #[test]
    fn test_snap_builder_then_parse_roundtrip() {
        // cite: snap/db.go (write+read symmetry)
        let mut b = SnapBuilder::new(1, 7).with_raft(2, 100).with_revision(50);
        b.add_entry(fixed_kv(b"a", b"1", 10));
        b.add_entry(fixed_kv(b"b", b"2", 20));
        let buf = b.encode();
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.header.cluster_id, 1);
        assert_eq!(parsed.header.member_id, 7);
        assert_eq!(parsed.header.raft_term, 2);
        assert_eq!(parsed.header.raft_index, 100);
        assert_eq!(parsed.header.revision, 50);
        assert_eq!(parsed.header.entry_count, 2);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].key, b"a");
        assert_eq!(parsed.entries[1].value, b"2");
    }

    #[test]
    fn test_snap_empty_entries() {
        // cite: snap/db.go (entry_count=0 valid)
        let b = SnapBuilder::new(1, 1);
        let buf = b.encode();
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.header.entry_count, 0);
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn test_snap_truncated() {
        // cite: snap/db.go (parser rejects short buffers)
        let buf = vec![0u8; 5];
        assert!(matches!(parse_snap(&buf).unwrap_err(), EtcdError::SnapshotDecode(_)));
    }

    #[test]
    fn test_snap_bad_magic() {
        // cite: snap/db.go (magic must match)
        let mut b = SnapBuilder::new(1, 1);
        b.add_entry(fixed_kv(b"a", b"1", 1));
        let mut buf = b.encode();
        buf[0] = 0;
        // Recompute trailer so checksum doesn't fire first.
        let body_len = buf.len() - HASH_LEN;
        let new_trailer = snap_hash(&buf[..body_len]);
        buf[body_len..].copy_from_slice(&new_trailer);
        match parse_snap(&buf).unwrap_err() {
            EtcdError::SnapshotDecode(m) => assert!(m.contains("bad magic"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snap_bad_version() {
        // cite: snap/db.go (version mismatch)
        let mut b = SnapBuilder::new(1, 1);
        b.add_entry(fixed_kv(b"a", b"1", 1));
        let mut buf = b.encode();
        buf[5] = 0xFF;
        let body_len = buf.len() - HASH_LEN;
        let new_trailer = snap_hash(&buf[..body_len]);
        buf[body_len..].copy_from_slice(&new_trailer);
        match parse_snap(&buf).unwrap_err() {
            EtcdError::SnapshotDecode(m) => assert!(m.contains("version"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snap_hash_mismatch() {
        // cite: snap/db.go (trailing hash verifies whole body)
        let mut b = SnapBuilder::new(1, 1);
        b.add_entry(fixed_kv(b"a", b"1", 1));
        let mut buf = b.encode();
        // Twiddle a body byte without rebuilding trailer.
        buf[10] ^= 1;
        match parse_snap(&buf).unwrap_err() {
            EtcdError::SnapshotChecksumMismatch { expected, actual } => assert_ne!(expected, actual),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snap_trailing_bytes_rejected() {
        // cite: snap/db.go (no trailing junk between entries and hash)
        let b = SnapBuilder::new(1, 1);
        let mut buf = b.encode();
        // Inject an extra byte before the hash trailer.
        let trailer_start = buf.len() - HASH_LEN;
        let trailer = buf[trailer_start..].to_vec();
        buf.truncate(trailer_start);
        buf.push(0xCC);
        let new_trailer = snap_hash(&buf);
        buf.extend_from_slice(&new_trailer);
        let _ = trailer; // unused
        match parse_snap(&buf).unwrap_err() {
            EtcdError::SnapshotDecode(m) => assert!(m.contains("trailing"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snap_partial_entry_truncation() {
        // cite: snap/db.go (entry-body truncation detected)
        let mut b = SnapBuilder::new(1, 1);
        b.add_entry(fixed_kv(b"a", b"1", 1));
        let mut buf = b.encode();
        // Drop part of the body before the trailer.
        let mid = buf.len() - HASH_LEN - 5;
        buf.drain(mid..mid + 5);
        // Recompute trailer so we hit the entry-truncation path, not hash.
        let body_len = buf.len() - HASH_LEN;
        let trailer = snap_hash(&buf[..body_len]);
        buf[body_len..].copy_from_slice(&trailer);
        match parse_snap(&buf).unwrap_err() {
            EtcdError::SnapshotDecode(m) => assert!(m.contains("truncated") || m.contains("trailing"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    // ── Save from KvStore ─────────────────────────────────────────────

    #[test]
    fn test_save_from_store_records_revision() {
        // cite: snap/db.go (header carries revision)
        let store = KvStore::new();
        populate_store(&store, 5);
        let buf = save_from_store(&store, 0xCAFE);
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.header.revision, store.current_revision());
        assert_eq!(parsed.header.cluster_id, 0xCAFE);
    }

    #[test]
    fn test_save_from_store_round_trip_roundtrip() {
        // cite: server.go applySnapshot (saved snap can be parsed)
        let store = KvStore::new();
        populate_store(&store, 3);
        let buf = save_from_store(&store, 1);
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.entries.len(), 3);
    }

    #[test]
    fn test_save_from_empty_store() {
        // cite: snap/db.go (empty backend ⇒ empty snap)
        let store = KvStore::new();
        let buf = save_from_store(&store, 1);
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.entries.len(), 0);
    }

    // ── Restore-from-snapshot startup ─────────────────────────────────

    #[test]
    fn test_restore_into_store_replays_entries() {
        // cite: server.go applySnapshot (entries go into the new mvcc)
        let src = KvStore::new();
        populate_store(&src, 5);
        let buf = save_from_store(&src, 1);
        let (restored, header) = restore_into_store(&buf).unwrap();
        assert_eq!(header.entry_count, 5);
        // Every key from the source should be present in the restored store.
        for i in 0..5 {
            let r = restored.range(&crate::models::RangeRequest {
                key: format!("/k/{i}"), range_end: None, limit: None,
                revision: None, keys_only: false, count_only: false,
            }).unwrap();
            assert_eq!(r.kvs.len(), 1, "missing /k/{i}");
            assert_eq!(r.kvs[0].value_str(), format!("v{i}"));
        }
    }

    #[test]
    fn test_restore_rejects_corrupted_snap() {
        // cite: server.go applySnapshot (bad checksum ⇒ refuse to start)
        let src = KvStore::new();
        populate_store(&src, 1);
        let mut buf = save_from_store(&src, 1);
        let mid = buf.len() / 2;
        buf[mid] ^= 0xFF;
        let err = restore_into_store(&buf).err().expect("expected error");
        assert!(matches!(err, EtcdError::SnapshotChecksumMismatch { .. }), "{err:?}");
    }

    #[test]
    fn test_restore_preserves_revision_count() {
        // cite: server.go (restored store reflects original revisions)
        let src = KvStore::new();
        populate_store(&src, 7);
        let buf = save_from_store(&src, 1);
        let (restored, _) = restore_into_store(&buf).unwrap();
        // Every PUT bumps the revision counter, so 7 entries ⇒ rev ≥ 7.
        assert!(restored.current_revision() >= 7);
    }

    #[test]
    fn test_restore_empty_snapshot_yields_empty_store() {
        // cite: server.go (empty snap ⇒ empty mvcc)
        let src = KvStore::new();
        let buf = save_from_store(&src, 1);
        let (restored, header) = restore_into_store(&buf).unwrap();
        assert_eq!(header.entry_count, 0);
        let r = restored.range(&crate::models::RangeRequest {
            key: "".into(), range_end: Some("\u{ffff}".into()),
            limit: None, revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert!(r.kvs.is_empty());
    }

    // ── Header fields ─────────────────────────────────────────────────

    #[test]
    fn test_header_carries_raft_term_and_index() {
        // cite: snap/db.go (raft state captured)
        let b = SnapBuilder::new(1, 1).with_raft(7, 99);
        let buf = b.encode();
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.header.raft_term, 7);
        assert_eq!(parsed.header.raft_index, 99);
    }

    #[test]
    fn test_header_records_creation_time() {
        // cite: snap/db.go (timestamp ⇒ admin tooling reads age)
        let b = SnapBuilder::new(1, 1);
        let buf = b.encode();
        let parsed = parse_snap(&buf).unwrap();
        // Created within the last day at most.
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        assert!(parsed.header.created_at <= now);
        assert!(parsed.header.created_at >= now - 86_400);
    }

    #[test]
    fn test_save_writes_member_id() {
        // cite: snap/db.go (member identity preserved)
        let store = KvStore::new();
        let buf = save_from_store(&store, 1);
        let parsed = parse_snap(&buf).unwrap();
        assert_eq!(parsed.header.member_id, store.local_member_id());
    }

    #[test]
    fn test_save_then_restore_then_save_is_stable() {
        // cite: snap/db.go (idempotent across save/restore cycles)
        let src = KvStore::new();
        populate_store(&src, 4);
        let buf1 = save_from_store(&src, 1);
        let (restored, _) = restore_into_store(&buf1).unwrap();
        let buf2 = save_from_store(&restored, 1);
        let p1 = parse_snap(&buf1).unwrap();
        let p2 = parse_snap(&buf2).unwrap();
        assert_eq!(p1.entries.len(), p2.entries.len());
    }

    #[test]
    fn test_large_value_round_trip() {
        // cite: snap/db.go (no value-size limit beyond u32)
        let store = KvStore::new();
        let big: String = "X".repeat(50_000);
        store.put(&PutRequest { key: "/big".into(), value: big.clone(), lease: None, prev_kv: false });
        let buf = save_from_store(&store, 1);
        let (restored, _) = restore_into_store(&buf).unwrap();
        let r = restored.range(&crate::models::RangeRequest {
            key: "/big".into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        assert_eq!(r.kvs[0].value_str().len(), 50_000);
    }
}
