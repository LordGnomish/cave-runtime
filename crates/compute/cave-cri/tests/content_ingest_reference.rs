// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Paths the content store builds from what its callers hand it.
//!
//! `LocalStore` keeps an ingest in `<root>/ingest/<reference>` and a blob in
//! `<root>/blobs/<algorithm>/<hex>`. The reference is chosen by the caller
//! and the digest can come out of a deserializer, so both are checked
//! before they become part of a path:
//!
//! * an ingest reference has to be a plain file name. One that is empty,
//!   is `.` or `..`, or contains a path separator or a NUL is refused, and
//!   refusing it touches nothing on disk;
//! * the ingest file is a file of the store's own, not what a symlink of
//!   that name leads to;
//! * a `Digest` only exists with a hex part that is valid for its
//!   algorithm, however it was constructed.

use cave_cri::content::{ContentStore, Digest, DigestAlgorithm, LocalStore, StoreError};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One store in a scratch directory: `<tmp>/store` is its root,
/// `<tmp>/outside/sentinel.txt` stands for the rest of the host.
struct Scratch {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    store_root: PathBuf,
    sentinel: PathBuf,
    store: LocalStore,
    /// A blob that is in the store from the start.
    blob: Digest,
}

const BLOB: &[u8] = b"a blob that is already in the store";

impl Scratch {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let store_root = root.join("store");
        let sentinel = root.join("outside").join("sentinel.txt");
        std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        std::fs::write(&sentinel, b"host data").unwrap();
        let store = LocalStore::open(&store_root).unwrap();
        let blob = Digest::compute(DigestAlgorithm::Sha256, BLOB);
        let mut writer = store.writer("seed".into(), blob.clone()).unwrap();
        writer.write_all(BLOB).unwrap();
        writer.commit().unwrap();
        Scratch {
            _tmp: tmp,
            root,
            store_root,
            sentinel,
            store,
            blob,
        }
    }

    /// Every path below the scratch directory with its kind and content.
    fn snapshot(&self) -> BTreeMap<PathBuf, String> {
        fn walk(dir: &Path, seen: &mut BTreeMap<PathBuf, String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = std::fs::symlink_metadata(&path).unwrap();
                let state = if meta.file_type().is_symlink() {
                    format!("symlink to {:?}", std::fs::read_link(&path).unwrap())
                } else if meta.is_dir() {
                    walk(&path, seen);
                    "dir".to_string()
                } else {
                    format!(
                        "file {:?}",
                        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                    )
                };
                seen.insert(path, state);
            }
        }
        let mut seen = BTreeMap::new();
        walk(&self.root, &mut seen);
        seen
    }
}

#[test]
fn writer_refuses_a_reference_that_is_not_a_plain_file_name() {
    let existing_blob = format!(
        "../blobs/sha256/{}",
        Digest::compute(DigestAlgorithm::Sha256, BLOB).hex()
    );
    let references = [
        "",
        ".",
        "..",
        "../escaped",
        "../../outside/sentinel.txt",
        "../../outside/created.txt",
        existing_blob.as_str(),
        "sub/ref",
        "sub/../ref",
        "./ref",
        "ref/",
        "/ref",
        "a\\b",
        "..\\escaped",
        "ref\0",
        "re\0f",
    ];
    for reference in references {
        let scratch = Scratch::new();
        let before = scratch.snapshot();

        let result = scratch
            .store
            .writer(reference.to_string(), scratch.blob.clone());

        let after = scratch.snapshot();
        assert_eq!(
            before, after,
            "refusing the reference {reference:?} changed files"
        );
        match result {
            Err(err) => assert!(
                err.to_string().contains("invalid ingest reference"),
                "{reference:?} was not refused as a reference: {err}"
            ),
            Ok(_) => panic!("the reference {reference:?} must be refused"),
        }
        assert!(
            scratch.store.list_active().is_empty(),
            "the refused reference {reference:?} is listed as active"
        );
    }

    // An absolute path is refused as well: here the path of a file that
    // exists outside the store.
    let scratch = Scratch::new();
    let before = scratch.snapshot();
    let reference = scratch.sentinel.to_str().unwrap().to_string();
    let result = scratch.store.writer(reference, scratch.blob.clone());
    assert_eq!(before, scratch.snapshot());
    assert!(result.is_err(), "an absolute path was taken as a reference");
}

/// The blob that is in the store survives a reference that names it.
#[test]
fn a_refused_reference_leaves_stored_blobs_alone() {
    let scratch = Scratch::new();
    let reference = format!("../blobs/sha256/{}", scratch.blob.hex());

    let result = scratch.store.writer(reference, scratch.blob.clone());

    assert!(result.is_err());
    let mut content = Vec::new();
    std::io::Read::read_to_end(
        &mut *scratch.store.reader(&scratch.blob).unwrap(),
        &mut content,
    )
    .unwrap();
    assert_eq!(content, BLOB);
}

#[test]
fn writer_does_not_write_through_a_symlink_in_the_ingest_directory() {
    let scratch = Scratch::new();
    std::os::unix::fs::symlink(&scratch.sentinel, scratch.store_root.join("ingest/pull-1"))
        .unwrap();
    let before = scratch.snapshot();

    let result = scratch
        .store
        .writer("pull-1".into(), scratch.blob.clone())
        .and_then(|mut writer| {
            writer
                .write_all(b"ingested bytes")
                .map_err(StoreError::from)
        });

    assert_eq!(std::fs::read(&scratch.sentinel).unwrap(), b"host data");
    assert_eq!(before, scratch.snapshot());
    assert!(result.is_err(), "the ingest went through a symlink");
    assert!(scratch.store.list_active().is_empty());
}

/// References that are plain file names work as before, also unusual ones.
#[test]
fn writer_accepts_plain_file_names_as_references() {
    for reference in [
        "r1",
        "sha256-0123abcd",
        "pull.session_1",
        "with space",
        "...",
        "..hidden",
        "trailing..",
        "ünïcode",
    ] {
        let scratch = Scratch::new();
        let data = format!("content for {reference}");
        let digest = Digest::compute(DigestAlgorithm::Sha256, data.as_bytes());

        let mut writer = scratch
            .store
            .writer(reference.to_string(), digest.clone())
            .unwrap_or_else(|e| panic!("{reference:?} was refused: {e}"));
        writer.write_all(data.as_bytes()).unwrap();
        writer.flush().unwrap();

        let ingest_file = scratch.store_root.join("ingest").join(reference);
        assert_eq!(std::fs::read(&ingest_file).unwrap(), data.as_bytes());
        assert_eq!(scratch.store.list_active(), [reference.to_string()]);

        assert_eq!(writer.commit().unwrap(), digest);
        assert!(scratch.store.exists(&digest));
        assert!(!ingest_file.exists());
        assert_eq!(
            std::fs::read(scratch.store.blob_path(&digest)).unwrap(),
            data.as_bytes()
        );
    }
}

/// A `Digest` is built by `parse` or `compute`, which both leave it with a
/// valid hex part. Deserializing one has to keep that promise, because
/// `blob_path` puts the hex part into a path.
#[test]
fn a_deserialized_digest_has_a_valid_hex_part() {
    let good = Digest::compute(DigestAlgorithm::Sha256, BLOB);
    let json = serde_json::to_value(&good).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "algorithm": "Sha256", "hex": good.hex() }),
        "the serialized form of a digest changed"
    );
    assert_eq!(serde_json::from_value::<Digest>(json).unwrap(), good);

    let scratch = Scratch::new();
    for hex in [
        "../../outside/sentinel.txt".to_string(),
        "..".to_string(),
        String::new(),
        "abc".to_string(),
        good.hex().to_uppercase(),
        format!("{}/", &good.hex()[1..]),
        format!("{}0", good.hex()),
    ] {
        let json = serde_json::json!({ "algorithm": "Sha256", "hex": hex });
        match serde_json::from_value::<Digest>(json) {
            Err(_) => {}
            Ok(digest) => panic!(
                "a digest with the hex part {hex:?} was deserialized; \
                 its blob path would be {}",
                scratch.store.blob_path(&digest).display()
            ),
        }
    }
    // The hex part has the length of another algorithm.
    let json = serde_json::json!({ "algorithm": "Sha512", "hex": good.hex() });
    assert!(serde_json::from_value::<Digest>(json).is_err());
}
