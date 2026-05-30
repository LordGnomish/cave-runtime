// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Content metadata persistence — the boltdb scope cut closed.
//!
//! Containerd's content store keeps per-blob metadata (labels +
//! created/updated timestamps) in its bolt `content` bucket
//! (`core/metadata/content.go`). The `containerd.io/gc.ref.content.*`
//! labels are load-bearing: the GC walker follows them to decide what
//! is still referenced, so they MUST survive a daemon restart — if they
//! vanish, the walker reaps blobs that are actually live.
//!
//! cave-cri's `LocalStore` rebuilt its index from a bare directory walk
//! on open, which recovered the blobs but dropped every label. These
//! tests pin the durability contract: labels and the original
//! created-at set via `label_blob` survive a close + reopen.

use cave_cri::content::digest::{Digest, DigestAlgorithm};
use cave_cri::content::store::{ContentStore, LocalStore};
use std::io::Write;
use tempfile::TempDir;

fn put(store: &LocalStore, bytes: &[u8]) -> Digest {
    let expected = Digest::compute(DigestAlgorithm::Sha256, bytes);
    let ref_ = format!("test-{}", &expected.hex()[..8]);
    let mut writer = store.writer(ref_, expected.clone()).unwrap();
    writer.write_all(bytes).unwrap();
    writer.commit().unwrap();
    expected
}

#[test]
fn gc_ref_label_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let d;
    {
        let store = LocalStore::open(dir.path()).unwrap();
        d = put(&store, b"config-blob");
        store
            .label_blob(
                &d,
                "containerd.io/gc.ref.content.config".into(),
                d.to_string(),
            )
            .unwrap();
    }
    // Reopen — the GC-ref label must still be there.
    let store2 = LocalStore::open(dir.path()).unwrap();
    let info = store2.info(&d).unwrap();
    assert_eq!(
        info.labels.get("containerd.io/gc.ref.content.config"),
        Some(&d.to_string()),
        "gc.ref label must survive reopen so the GC walker still sees the reference"
    );
}

#[test]
fn multiple_labels_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let d;
    {
        let store = LocalStore::open(dir.path()).unwrap();
        d = put(&store, b"multi-labeled");
        store.label_blob(&d, "a".into(), "1".into()).unwrap();
        store.label_blob(&d, "b".into(), "2".into()).unwrap();
    }
    let store2 = LocalStore::open(dir.path()).unwrap();
    let info = store2.info(&d).unwrap();
    assert_eq!(info.labels.get("a"), Some(&"1".to_string()));
    assert_eq!(info.labels.get("b"), Some(&"2".to_string()));
}

#[test]
fn created_at_is_preserved_across_reopen() {
    let dir = TempDir::new().unwrap();
    let (d, original_created);
    {
        let store = LocalStore::open(dir.path()).unwrap();
        d = put(&store, b"timestamped");
        store.label_blob(&d, "k".into(), "v".into()).unwrap();
        original_created = store.info(&d).unwrap().created_at_unix;
    }
    let store2 = LocalStore::open(dir.path()).unwrap();
    assert_eq!(
        store2.info(&d).unwrap().created_at_unix,
        original_created,
        "the committed created-at must persist, not be re-derived from mtime"
    );
}

#[test]
fn deleted_blob_metadata_does_not_haunt_a_replacement() {
    let dir = TempDir::new().unwrap();
    let d;
    {
        let store = LocalStore::open(dir.path()).unwrap();
        d = put(&store, b"ephemeral");
        store.label_blob(&d, "stale".into(), "label".into()).unwrap();
        store.delete(&d).unwrap();
    }
    // Reopen: the blob is gone, and a freshly re-put identical blob must
    // not inherit the deleted blob's stale label.
    let store2 = LocalStore::open(dir.path()).unwrap();
    assert!(!store2.exists(&d), "deleted blob must stay gone after reopen");
    let d2 = put(&store2, b"ephemeral");
    assert_eq!(d2, d);
    assert!(
        store2.info(&d2).unwrap().labels.get("stale").is_none(),
        "re-put blob must not inherit the deleted blob's persisted label"
    );
}

#[test]
fn unlabeled_blob_still_recovered_with_empty_labels() {
    let dir = TempDir::new().unwrap();
    let d;
    {
        let store = LocalStore::open(dir.path()).unwrap();
        d = put(&store, b"bare");
    }
    let store2 = LocalStore::open(dir.path()).unwrap();
    let info = store2.info(&d).unwrap();
    assert!(info.labels.is_empty());
    assert_eq!(info.size, 4);
}
