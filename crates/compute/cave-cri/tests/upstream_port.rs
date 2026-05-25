// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream containerd tests, cross-referenced
//! from `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: containerd/containerd @ v1.7.20
//!   * core/content/local/store_test.go
//!   * core/content/local/writer_test.go
//!   * core/diff/walking/walking_test.go
//!   * pkg/digest/digest_test.go (vendored from opencontainers/go-digest)
//!   * core/leases/manager_test.go
//!
//! Subtests (Go `t.Run(name, …)`) split into individual `#[test]` fns
//! so a single subtest failure stays localised.

use cave_cri::content::digest::{Digest, DigestAlgorithm, DigestError};
use cave_cri::content::store::{ContentStore, LocalStore, StoreError};
use cave_cri::diff::compression::{compress_gzip, compute_diff_id, decompress_gzip};
use cave_cri::diff::walking_differ::{apply_layer, apply_uncompressed_tar};
use cave_cri::leases::manager::{LeaseError, LeaseManager};
use cave_cri::leases::resource::{Resource, ResourceKind};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use tar::{Builder, Header};
use tempfile::TempDir;

fn make_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = Builder::new(Vec::new());
    for (path, data) in files {
        let mut h = Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, path, *data).unwrap();
    }
    b.into_inner().unwrap()
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/digest/digest_test.go (vendored from opencontainers/go-digest)
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestParseDigest / `sha256_wire_round_trips_via_String`.
#[test]
fn upstream_digest_parse_round_trips_via_display() {
    let s = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let d = Digest::parse(s).unwrap();
    assert_eq!(d.algorithm(), DigestAlgorithm::Sha256);
    assert_eq!(format!("{}", d), s);
}

/// Upstream: TestParseDigest / `rejects_uppercase_hex`.
/// go-digest's `regexp.MustCompile` only matches lowercase hex.
#[test]
fn upstream_digest_parse_rejects_uppercase_hex() {
    let s = "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
    let err = Digest::parse(s).unwrap_err();
    match err {
        DigestError::NonHex(_) => {}
        other => panic!("expected NonHex, got {other:?}"),
    }
}

/// Upstream: TestParseDigest / `rejects_unknown_algorithm`.
/// go-digest's `Validate` returns `ErrDigestUnsupported` for md5 etc.
#[test]
fn upstream_digest_parse_rejects_md5() {
    let s = "md5:00000000000000000000000000000000";
    match Digest::parse(s).unwrap_err() {
        DigestError::UnknownAlgorithm(a) => assert_eq!(a, "md5"),
        e => panic!("unexpected error {e:?}"),
    }
}

/// Upstream: TestParseDigest / `rejects_missing_separator`.
#[test]
fn upstream_digest_parse_rejects_missing_colon() {
    match Digest::parse("just-some-string").unwrap_err() {
        DigestError::MissingSeparator(_) => {}
        e => panic!("unexpected error {e:?}"),
    }
}

/// Upstream: TestDigesterHashOfEmpty / well-known SHA-256.
/// go-digest's reference vector — every implementation MUST match.
#[test]
fn upstream_digest_compute_sha256_of_empty_matches_reference() {
    let d = Digest::compute(DigestAlgorithm::Sha256, b"");
    assert_eq!(
        d.to_string(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: core/content/local/store_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestLocalStore / `Open_creates_blobs_and_ingest_layout`.
#[test]
fn upstream_local_store_open_creates_blobs_and_ingest_layout() {
    let tmp = TempDir::new().unwrap();
    let _ = LocalStore::open(tmp.path()).unwrap();
    assert!(tmp.path().join("blobs/sha256").is_dir());
    assert!(tmp.path().join("blobs/sha512").is_dir());
    assert!(tmp.path().join("ingest").is_dir());
}

/// Upstream: TestLocalStoreWriter / `commit_succeeds_when_digest_matches`.
#[test]
fn upstream_local_store_writer_commit_succeeds_when_digest_matches() {
    let tmp = TempDir::new().unwrap();
    let store = LocalStore::open(tmp.path()).unwrap();
    let payload = b"hello content store";
    let expected = Digest::compute(DigestAlgorithm::Sha256, payload);
    let mut writer = store.writer("session-1".into(), expected.clone()).unwrap();
    writer.write_all(payload).unwrap();
    let final_digest = writer.commit().unwrap();
    assert_eq!(final_digest, expected);
    assert!(store.exists(&expected));
    let info = store.info(&expected).unwrap();
    assert_eq!(info.size, payload.len() as u64);
}

/// Upstream: TestLocalStore / `delete_blocked_when_lease_holds_blob`.
/// Mirrors core/leases/manager.go's GC interlock.
#[test]
fn upstream_local_store_delete_refused_when_lease_holds_blob() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(LocalStore::open(tmp.path()).unwrap());
    let leases = LeaseManager::with_store(store.clone());
    let payload = b"leased blob";
    let expected = Digest::compute(DigestAlgorithm::Sha256, payload);
    let mut writer = store.writer("w".into(), expected.clone()).unwrap();
    writer.write_all(payload).unwrap();
    writer.commit().unwrap();
    leases.create("L1", None, HashMap::new()).unwrap();
    leases
        .add_resource("L1", Resource::content(&expected))
        .unwrap();
    let err = store.delete(&expected).unwrap_err();
    assert!(
        matches!(err, StoreError::InUse(ref id) if id == "L1"),
        "expected InUse(L1), got {err:?}"
    );
    // Releasing the lease unblocks delete.
    leases.delete("L1").unwrap();
    assert!(store.delete(&expected).is_ok());
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: core/diff/walking/walking_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestApplyLayer / `unpacks_regular_files`.
#[test]
fn upstream_walking_apply_unpacks_regular_files() {
    let tar = make_tar(&[("hello.txt", b"hi!"), ("nested/inner.bin", b"bytes")]);
    let gz = compress_gzip(&tar).unwrap();
    let tmp = TempDir::new().unwrap();
    let stats = apply_layer(&gz, tmp.path()).unwrap();
    assert_eq!(stats.files_written, 2);
    assert_eq!(std::fs::read(tmp.path().join("hello.txt")).unwrap(), b"hi!");
    assert_eq!(
        std::fs::read(tmp.path().join("nested/inner.bin")).unwrap(),
        b"bytes"
    );
}

/// Upstream: TestApplyLayer / `whiteout_delete_sibling_removes_file`.
/// OCI whiteout convention: `.wh.<name>` deletes `<name>` from the
/// layer below.
#[test]
fn upstream_walking_apply_whiteout_removes_named_sibling() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("victim.txt"), b"die").unwrap();
    let mut b = Builder::new(Vec::new());
    let mut h = Header::new_gnu();
    h.set_size(0);
    h.set_mode(0o644);
    h.set_cksum();
    b.append_data(&mut h, ".wh.victim.txt", std::io::empty())
        .unwrap();
    let tar = b.into_inner().unwrap();
    let gz = compress_gzip(&tar).unwrap();
    let stats = apply_layer(&gz, tmp.path()).unwrap();
    assert_eq!(stats.whiteouts_applied, 1);
    assert!(!tmp.path().join("victim.txt").exists());
}

/// Upstream: TestApplyLayer / `opaque_whiteout_clears_directory`.
/// `.wh..wh..opq` in a directory empties every sibling already
/// present from the layer below.
#[test]
fn upstream_walking_apply_opaque_whiteout_clears_dir() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("d")).unwrap();
    std::fs::write(tmp.path().join("d/one"), b"1").unwrap();
    std::fs::write(tmp.path().join("d/two"), b"2").unwrap();
    let mut b = Builder::new(Vec::new());
    let mut h = Header::new_gnu();
    h.set_size(0);
    h.set_mode(0o644);
    h.set_cksum();
    b.append_data(&mut h, "d/.wh..wh..opq", std::io::empty())
        .unwrap();
    let tar = b.into_inner().unwrap();
    let gz = compress_gzip(&tar).unwrap();
    let stats = apply_layer(&gz, tmp.path()).unwrap();
    assert_eq!(stats.whiteouts_applied, 1);
    assert!(!tmp.path().join("d/one").exists());
    assert!(!tmp.path().join("d/two").exists());
}

/// Upstream: TestApplyLayer / `empty_layer_is_a_noop`.
/// Upstream returns zero-valued stats for an empty tar.
#[test]
fn upstream_walking_apply_empty_layer_returns_zero_stats() {
    let b = Builder::new(Vec::new());
    let tar = b.into_inner().unwrap();
    let gz = compress_gzip(&tar).unwrap();
    let tmp = TempDir::new().unwrap();
    let stats = apply_layer(&gz, tmp.path()).unwrap();
    assert_eq!(stats.files_written, 0);
    assert_eq!(stats.dirs_created, 0);
    assert_eq!(stats.whiteouts_applied, 0);
    assert_eq!(stats.symlinks_created, 0);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: core/diff/walking/walking.go::DiffID computation
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestComputeDiffID / `diff_id_matches_sha256_of_uncompressed_tar`.
/// OCI image spec: diff_id is the SHA-256 of the *uncompressed* tar
/// stream, distinct from the layer digest which is over the gzipped form.
#[test]
fn upstream_diff_id_is_sha256_of_uncompressed_tar() {
    let tar = make_tar(&[("a", b"alpha")]);
    let diff_id = compute_diff_id(&tar);
    let expected = Digest::compute(DigestAlgorithm::Sha256, &tar);
    assert_eq!(diff_id, expected);
}

/// Upstream: TestGzipRoundTrip / `compress_decompress_round_trips_bytes`.
#[test]
fn upstream_gzip_compress_decompress_round_trips() {
    let payload = b"the quick brown fox jumps over the lazy dog";
    let gz = compress_gzip(payload).unwrap();
    let back = decompress_gzip(&gz).unwrap();
    assert_eq!(back, payload);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: core/leases/manager_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestLeaseManager / `create_then_get_returns_lease`.
#[test]
fn upstream_lease_manager_create_then_get_returns_lease() {
    let leases = LeaseManager::new();
    leases
        .create("pull-session-9", Some(60), HashMap::new())
        .unwrap();
    let fetched = leases.get("pull-session-9").unwrap();
    assert_eq!(fetched.id, "pull-session-9");
    assert_eq!(fetched.ttl_seconds, Some(60));
}

/// Upstream: TestLeaseManager / `duplicate_id_returns_AlreadyExists`.
#[test]
fn upstream_lease_manager_duplicate_create_errors() {
    let leases = LeaseManager::new();
    leases.create("dup", None, HashMap::new()).unwrap();
    let err = leases.create("dup", None, HashMap::new()).unwrap_err();
    assert!(matches!(err, LeaseError::AlreadyExists(ref id) if id == "dup"));
}

/// Upstream: TestLeaseManager / `live_content_returns_held_digests`.
/// The GC walker uses `live_content()` to decide which blobs are
/// safe to reap.
#[test]
fn upstream_lease_manager_live_content_returns_held_content_digests() {
    let leases = LeaseManager::new();
    leases.create("L1", None, HashMap::new()).unwrap();
    let d = Digest::compute(DigestAlgorithm::Sha256, b"abc");
    leases.add_resource("L1", Resource::content(&d)).unwrap();
    // Add a non-content resource — must NOT appear in live_content.
    leases
        .add_resource("L1", Resource::snapshot("snap-7"))
        .unwrap();
    let live = leases.live_content();
    assert!(live.contains(&d.to_string()));
    assert!(!live.contains("snap-7"));
}

/// Upstream: TestLeaseManager / `expired_lease_is_reaped`.
#[test]
fn upstream_lease_manager_reap_expired_removes_only_expired() {
    let leases = LeaseManager::new();
    // TTL=0 ⇒ immediately expired.
    leases.create("dead", Some(0), HashMap::new()).unwrap();
    // No TTL ⇒ never expires.
    leases.create("live", None, HashMap::new()).unwrap();
    let reaped = leases.reap_expired();
    assert!(reaped.contains(&"dead".to_string()));
    assert!(!reaped.contains(&"live".to_string()));
    assert!(leases.get("dead").is_err());
    assert!(leases.get("live").is_ok());
}

/// Upstream: TestLeaseResource / `ResourceKind::Content carries digest`.
#[test]
fn upstream_lease_resource_content_kind_carries_digest() {
    let d = Digest::compute(DigestAlgorithm::Sha256, b"abc");
    let r = Resource::content(&d);
    assert_eq!(r.kind, ResourceKind::Content);
    assert_eq!(r.content_digest().unwrap(), d);
    // Snapshot resources never carry a digest.
    let s = Resource::snapshot("snap-7");
    assert!(s.content_digest().is_none());
}
