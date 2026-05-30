// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD — Iceberg `LocalFileIo` (filesystem-backed FileIo).
//!
//! Upstream: `crates/iceberg/src/io/` ships, alongside the object-store
//! backends, a local-filesystem `FileIO` so tables can be read/written
//! against `file://` and bare-path locations without a remote store.
//! This closes the portable surface of the FileIo partial — the cloud
//! object-store backends (S3/GCS/ADLS) remain an explicit scope_cut.

use cave_iceberg::file_io::{FileIo, LocalFileIo};

fn unique_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("cave-iceberg-localio-{}-{}", std::process::id(), name));
    // Start clean.
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[tokio::test]
async fn local_io_round_trip() {
    let dir = unique_dir("round_trip");
    let path = dir.join("a.parquet");
    let path = path.to_str().unwrap();
    let io = LocalFileIo::new();

    assert!(!io.exists(path).await.unwrap());
    io.write(path, b"hello".to_vec()).await.unwrap();
    assert!(io.exists(path).await.unwrap());
    assert_eq!(io.read(path).await.unwrap(), b"hello");
    io.delete(path).await.unwrap();
    assert!(!io.exists(path).await.unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn local_io_read_missing_returns_not_found() {
    let dir = unique_dir("read_missing");
    let path = dir.join("nope").to_str().unwrap().to_string();
    let io = LocalFileIo::new();
    let r = io.read(&path).await;
    assert!(matches!(r, Err(cave_iceberg::Error::NotFound(_))));
}

#[tokio::test]
async fn local_io_delete_missing_returns_not_found() {
    let dir = unique_dir("delete_missing");
    let path = dir.join("nope").to_str().unwrap().to_string();
    let io = LocalFileIo::new();
    let r = io.delete(&path).await;
    assert!(matches!(r, Err(cave_iceberg::Error::NotFound(_))));
}

#[tokio::test]
async fn local_io_write_creates_parent_dirs() {
    let dir = unique_dir("nested");
    let path = dir.join("metadata").join("v1").join("snap.avro");
    let path = path.to_str().unwrap();
    let io = LocalFileIo::new();
    io.write(path, b"x".to_vec()).await.unwrap();
    assert!(io.exists(path).await.unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn local_io_strips_file_scheme() {
    let dir = unique_dir("scheme");
    let bare = dir.join("b.bin");
    let bare = bare.to_str().unwrap().to_string();
    let with_scheme = format!("file://{bare}");
    let io = LocalFileIo::new();

    io.write(&with_scheme, b"data".to_vec()).await.unwrap();
    // Reading via the bare path resolves to the same file.
    assert_eq!(io.read(&bare).await.unwrap(), b"data");
    let _ = std::fs::remove_dir_all(&dir);
}
