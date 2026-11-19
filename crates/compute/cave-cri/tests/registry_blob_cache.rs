// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The blob cache behind `RegistryClient::pull_blob`.
//!
//! A blob is kept in `<cache>/blobs` under a file name made from its digest.
//! Two things follow from that, and are pinned here:
//!
//! * the digest of a descriptor has to be a well-formed digest before it is
//!   used as a file name. The grammar is the one of the OCI image
//!   specification (descriptor.md, "Digests"): `algorithm ":" encoded`, and
//!   for the registered algorithms `sha256` and `sha512` the encoded part is
//!   64 respectively 128 lowercase hex characters;
//! * a file found in the cache is only handed out if its content has that
//!   digest.
//!
//! No test here talks to a registry. The registry they name is a closed
//! port on the loopback interface, so a download can only fail, and the
//! only way to get a path back is through the cache.

use cave_cri::error::{CriError, CriResult};
use cave_cri::models::{ImageReference, OciDescriptor};
use cave_cri::registry::RegistryClient;
use sha2::{Digest as _, Sha256, Sha512};
use std::path::{Path, PathBuf};

const LAYER: &[u8] = b"layer bytes as the registry serves them";

fn sha256_of(data: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

fn sha512_of(data: &[u8]) -> String {
    format!("sha512:{}", hex::encode(Sha512::digest(data)))
}

/// The file name the cache uses for a well-formed digest.
fn cache_name(digest: &str) -> String {
    digest.replace(':', "_")
}

/// A registry nobody listens on.
fn unreachable_registry() -> ImageReference {
    ImageReference {
        registry: "127.0.0.1:1".into(),
        repository: "library/test".into(),
        tag: Some("latest".into()),
        digest: None,
    }
}

async fn pull(cache: &Path, digest: &str) -> CriResult<PathBuf> {
    let descriptor = OciDescriptor {
        media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
        digest: digest.into(),
        size: LAYER.len() as u64,
    };
    RegistryClient::new(cache.to_path_buf())
        .pull_blob(&unreachable_registry(), &descriptor)
        .await
}

/// Create `path` with `content`, and the directories above it.
fn plant(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[tokio::test]
async fn pull_blob_refuses_a_digest_that_is_not_well_formed() {
    let hex64 = "a".repeat(64);
    let hex128 = "b".repeat(128);
    let scratch = tempfile::tempdir().unwrap();
    let elsewhere = scratch.path().join("elsewhere").join("host.bin");
    plant(&elsewhere, b"not a blob");

    // (digest, a file below the cache directory that exists while the
    // digest is pulled). The file is where a lookup that takes the digest
    // as it is would find something to return.
    let cases: Vec<(String, Option<String>)> = vec![
        // Not `algorithm ":" encoded` at all.
        (String::new(), None),
        ("sha256".into(), Some("blobs/sha256".into())),
        (hex64.clone(), Some(format!("blobs/{hex64}"))),
        ("sha256:".into(), Some("blobs/sha256_".into())),
        (format!(":{hex64}"), Some(format!("blobs/_{hex64}"))),
        // An encoded part of the wrong length for the algorithm.
        ("sha256:abc123".into(), Some("blobs/sha256_abc123".into())),
        (
            format!("sha256:{}", &hex64[1..]),
            Some(format!("blobs/sha256_{}", &hex64[1..])),
        ),
        (
            format!("sha256:{hex64}a"),
            Some(format!("blobs/sha256_{hex64}a")),
        ),
        (
            format!("sha512:{hex64}"),
            Some(format!("blobs/sha512_{hex64}")),
        ),
        (
            format!("sha256:{hex128}"),
            Some(format!("blobs/sha256_{hex128}")),
        ),
        // Characters outside `[a-f0-9]`.
        (
            format!("sha256:{}", hex64.to_uppercase()),
            Some(format!("blobs/sha256_{}", hex64.to_uppercase())),
        ),
        (
            format!("sha256:{}g", &hex64[1..]),
            Some(format!("blobs/sha256_{}g", &hex64[1..])),
        ),
        (
            format!("sha256:{} ", &hex64[1..]),
            Some(format!("blobs/sha256_{} ", &hex64[1..])),
        ),
        (
            format!("sha256:{hex64}\n"),
            Some(format!("blobs/sha256_{hex64}\n")),
        ),
        (
            format!(" sha256:{hex64}"),
            Some(format!("blobs/ sha256_{hex64}")),
        ),
        (format!("sha256:{}\0", &hex64[1..]), None),
        // An algorithm that is not registered, or not spelled as registered.
        (
            format!("md5:{}", &hex64[..32]),
            Some(format!("blobs/md5_{}", &hex64[..32])),
        ),
        (
            format!("sha384:{}", "c".repeat(96)),
            Some(format!("blobs/sha384_{}", "c".repeat(96))),
        ),
        (
            format!("SHA256:{hex64}"),
            Some(format!("blobs/SHA256_{hex64}")),
        ),
        (
            format!("sha256+b64:{hex64}"),
            Some(format!("blobs/sha256+b64_{hex64}")),
        ),
        // More than one separator.
        (
            format!("sha256:{hex64}:{hex64}"),
            Some(format!("blobs/sha256_{hex64}_{hex64}")),
        ),
        // Path syntax in place of a digest.
        ("../stray.bin".into(), Some("stray.bin".into())),
        ("sha256:x/../../stray.bin".into(), Some("stray.bin".into())),
        (
            format!("sha256:{hex64}/../stray.bin"),
            Some("blobs/stray.bin".into()),
        ),
        (
            format!("sha256:{hex64}/nested"),
            Some(format!("blobs/sha256_{hex64}/nested")),
        ),
        (".".into(), None),
        ("..".into(), None),
        (elsewhere.to_str().unwrap().to_string(), None),
    ];

    for (digest, present) in cases {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(cache.join("blobs")).unwrap();
        if let Some(present) = &present {
            plant(&cache.join(present), LAYER);
        }
        // Directories that make the path syntax above resolvable.
        std::fs::create_dir_all(cache.join("blobs/sha256_x")).unwrap();
        if digest.contains("/../") && digest.starts_with("sha256:a") {
            std::fs::create_dir_all(cache.join(format!("blobs/sha256_{hex64}"))).unwrap();
        }

        match pull(&cache, &digest).await {
            Err(CriError::Registry(msg)) => assert!(
                msg.contains("invalid blob digest"),
                "{digest:?} was not refused as a digest: {msg}"
            ),
            other => panic!("{digest:?} must be refused as a digest, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn pull_blob_does_not_return_a_cached_file_with_other_content() {
    for digest in [sha256_of(LAYER), sha512_of(LAYER)] {
        for cached in [&b"other bytes"[..], &LAYER[..LAYER.len() - 1], b""] {
            let dir = tempfile::tempdir().unwrap();
            let cached_path = dir.path().join("blobs").join(cache_name(&digest));
            plant(&cached_path, cached);

            // Nothing can be downloaded, so the only path there is to
            // return is the cached file, and its content is not the blob.
            let result = pull(dir.path(), &digest).await;

            assert!(
                result.is_err(),
                "a cached file with content {:?} was returned for {digest}: {result:?}",
                String::from_utf8_lossy(cached)
            );
        }
    }
}

/// The cache holds regular files. An entry of another kind is not read as
/// a blob, whatever it leads to.
#[tokio::test]
async fn pull_blob_does_not_use_a_cache_entry_that_is_not_a_regular_file() {
    let digest = sha256_of(LAYER);

    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere.bin");
    plant(&elsewhere, LAYER);
    let entry = dir.path().join("cache/blobs").join(cache_name(&digest));
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &entry).unwrap();
    let result = pull(&dir.path().join("cache"), &digest).await;
    assert!(result.is_err(), "a symlink was used as a blob: {result:?}");
    assert_eq!(std::fs::read(&elsewhere).unwrap(), LAYER);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("blobs").join(cache_name(&digest))).unwrap();
    let result = pull(dir.path(), &digest).await;
    assert!(
        result.is_err(),
        "a directory was used as a blob: {result:?}"
    );
}

/// A cached blob whose content has the digest is returned as before,
/// without a download.
#[tokio::test]
async fn pull_blob_returns_a_cached_blob_with_matching_content() {
    for digest in [sha256_of(LAYER), sha512_of(LAYER)] {
        let dir = tempfile::tempdir().unwrap();
        let cached_path = dir.path().join("blobs").join(cache_name(&digest));
        plant(&cached_path, LAYER);

        let path = pull(dir.path(), &digest).await.unwrap();

        assert_eq!(path, cached_path);
        assert_eq!(std::fs::read(&path).unwrap(), LAYER);
    }
}
