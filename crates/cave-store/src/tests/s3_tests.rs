// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for the S3/MinIO object store.

use crate::s3::store::ObjectStore;
use crate::s3::types::{VersioningState, StorageClass};
use crate::wal::WalWriter;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn make_store(dir: &TempDir) -> Arc<ObjectStore> {
    let wal = WalWriter::open(dir.path()).unwrap();
    let data_dir = dir.path().join("objects");
    std::fs::create_dir_all(&data_dir).unwrap();
    Arc::new(ObjectStore::new(data_dir, Arc::new(wal)))
}

#[tokio::test]
async fn test_bucket_create_list_delete() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("my-bucket", "us-east-1", "cave").await.unwrap();

    let buckets = store.list_buckets().await;
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].name, "my-bucket");

    // Can't create the same bucket twice
    assert!(store.create_bucket("my-bucket", "us-east-1", "cave").await.is_err());

    store.delete_bucket("my-bucket").await.unwrap();
    let buckets2 = store.list_buckets().await;
    assert!(buckets2.is_empty());
}

#[tokio::test]
async fn test_bucket_name_validation() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    assert!(store.create_bucket("AB", "us-east-1", "cave").await.is_err()); // too short + uppercase
    assert!(store.create_bucket("a", "us-east-1", "cave").await.is_err()); // too short
    assert!(store.create_bucket("-bad", "us-east-1", "cave").await.is_err()); // starts with -
    assert!(store.create_bucket("good-bucket", "us-east-1", "cave").await.is_ok());
}

#[tokio::test]
async fn test_put_get_delete_object() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    let result = store
        .put_object(
            "test",
            "hello.txt",
            b"Hello, World!".to_vec(),
            "text/plain",
            HashMap::new(),
            HashMap::new(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(!result.etag.is_empty());
    assert!(result.version_id.is_none()); // versioning disabled

    // Get it back
    let obj = store
        .get_object("test", "hello.txt", None, None, None)
        .await
        .unwrap();
    assert_eq!(obj.body, b"Hello, World!");
    assert_eq!(obj.content_type, "text/plain");

    // Head object
    let head = store.head_object("test", "hello.txt", None).await.unwrap();
    assert_eq!(head.size, 13);

    // Delete
    store.delete_object("test", "hello.txt", None).await.unwrap();
    assert!(store.get_object("test", "hello.txt", None, None, None).await.is_err());
}

#[tokio::test]
async fn test_object_metadata() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("author".to_string(), "alice".to_string());
    metadata.insert("project".to_string(), "cave".to_string());

    store
        .put_object("test", "obj", b"data".to_vec(), "text/plain", metadata.clone(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    let obj = store.get_object("test", "obj", None, None, None).await.unwrap();
    assert_eq!(obj.metadata.get("author").map(|s| s.as_str()), Some("alice"));
    assert_eq!(obj.metadata.get("project").map(|s| s.as_str()), Some("cave"));
}

#[tokio::test]
async fn test_object_tagging() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store
        .put_object("test", "obj", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "prod".to_string());
    tags.insert("team".to_string(), "platform".to_string());
    store.put_object_tagging("test", "obj", None, tags.clone()).await.unwrap();

    let retrieved = store.get_object_tagging("test", "obj", None).await.unwrap();
    assert_eq!(retrieved.get("env").map(|s| s.as_str()), Some("prod"));
}

#[tokio::test]
async fn test_versioning() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store.set_versioning("test", VersioningState::Enabled).await.unwrap();

    let r1 = store
        .put_object("test", "obj", b"version1".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();
    let r2 = store
        .put_object("test", "obj", b"version2".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    assert!(r1.version_id.is_some());
    assert!(r2.version_id.is_some());
    assert_ne!(r1.version_id, r2.version_id);

    // Latest
    let latest = store.get_object("test", "obj", None, None, None).await.unwrap();
    assert_eq!(latest.body, b"version2");

    // Old version
    let v1 = store
        .get_object("test", "obj", r1.version_id.as_deref(), None, None)
        .await
        .unwrap();
    assert_eq!(v1.body, b"version1");
}

#[tokio::test]
async fn test_delete_marker_versioning() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store.set_versioning("test", VersioningState::Enabled).await.unwrap();

    store
        .put_object("test", "obj", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    // Soft delete — creates a delete marker
    let del = store.delete_object("test", "obj", None).await.unwrap();
    assert!(del.delete_marker);
    assert!(del.version_id.is_some());

    // Can't get latest (delete marker)
    assert!(store.get_object("test", "obj", None, None, None).await.is_err());
}

#[tokio::test]
async fn test_list_objects_v2() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    for key in &["a/1.txt", "a/2.txt", "b/1.txt", "root.txt"] {
        store
            .put_object("test", key, b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
            .await
            .unwrap();
    }

    // List all
    let all = store.list_objects_v2("test", "", None, 100, None).await.unwrap();
    assert_eq!(all.key_count, 4);

    // Prefix filter
    let a_prefix = store.list_objects_v2("test", "a/", None, 100, None).await.unwrap();
    assert_eq!(a_prefix.key_count, 2);

    // Delimiter grouping
    let with_delim = store.list_objects_v2("test", "", Some("/"), 100, None).await.unwrap();
    assert_eq!(with_delim.common_prefixes.len(), 2); // a/ and b/
    assert_eq!(with_delim.key_count, 1); // only root.txt

    // Max keys
    let limited = store.list_objects_v2("test", "", None, 2, None).await.unwrap();
    assert_eq!(limited.key_count, 2);
    assert!(limited.is_truncated);
}

#[tokio::test]
async fn test_copy_object() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("src", "us-east-1", "cave").await.unwrap();
    store.create_bucket("dst", "us-east-1", "cave").await.unwrap();

    store
        .put_object("src", "original.txt", b"original content".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    let copy = store
        .copy_object("src", "original.txt", None, "dst", "copy.txt", "COPY", None)
        .await
        .unwrap();
    assert!(!copy.etag.is_empty());

    let got = store.get_object("dst", "copy.txt", None, None, None).await.unwrap();
    assert_eq!(got.body, b"original content");
}

#[tokio::test]
async fn test_multipart_upload() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    let upload_id = store
        .create_multipart_upload("test", "large.bin", "application/octet-stream", HashMap::new())
        .await
        .unwrap();

    // Upload 3 parts (each >= 5 MB except last)
    let part1_data = vec![b'A'; 5 * 1024 * 1024];
    let part2_data = vec![b'B'; 5 * 1024 * 1024];
    let part3_data = vec![b'C'; 1024]; // last part can be small

    let etag1 = store.upload_part(&upload_id, 1, part1_data.clone()).await.unwrap();
    let etag2 = store.upload_part(&upload_id, 2, part2_data.clone()).await.unwrap();
    let etag3 = store.upload_part(&upload_id, 3, part3_data.clone()).await.unwrap();

    let parts = store.list_parts(&upload_id).await.unwrap();
    assert_eq!(parts.len(), 3);

    let result = store
        .complete_multipart_upload(&upload_id, vec![(1, etag1), (2, etag2), (3, etag3)])
        .await
        .unwrap();
    assert!(!result.etag.is_empty());
    assert!(result.etag.contains('-')); // multipart ETag format

    // Object exists and has correct size
    let head = store.head_object("test", "large.bin", None).await.unwrap();
    let expected_size = part1_data.len() + part2_data.len() + part3_data.len();
    assert_eq!(head.size, expected_size as u64);
}

#[tokio::test]
async fn test_abort_multipart() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    let upload_id = store
        .create_multipart_upload("test", "large.bin", "application/octet-stream", HashMap::new())
        .await
        .unwrap();

    store.upload_part(&upload_id, 1, vec![b'A'; 1024]).await.unwrap();
    store.abort_multipart_upload(&upload_id).await.unwrap();

    // Upload is gone
    assert!(store.list_parts(&upload_id).await.is_err());

    // Object doesn't exist
    assert!(store.head_object("test", "large.bin", None).await.is_err());
}

#[tokio::test]
async fn test_range_request() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store
        .put_object("test", "data.bin", b"0123456789".to_vec(), "application/octet-stream", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    let obj = store
        .get_object("test", "data.bin", None, Some((2, 5)), None)
        .await
        .unwrap();
    assert_eq!(obj.body, b"2345");
    assert!(obj.content_range.is_some());
}

#[tokio::test]
async fn test_sse_s3_encryption() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();

    let plaintext = b"sensitive data";
    store
        .put_object("test", "secret.txt", plaintext.to_vec(), "text/plain", HashMap::new(), HashMap::new(), Some("AES256"), None, None)
        .await
        .unwrap();

    // Data on disk should be encrypted
    let data_on_disk = {
        let entries = store.list_objects_v2("test", "", None, 100, None).await.unwrap();
        let head = store.head_object("test", "secret.txt", None).await.unwrap();
        assert!(head.encryption.is_some());
        true
    };

    // Get decrypts transparently
    let obj = store.get_object("test", "secret.txt", None, None, None).await.unwrap();
    assert_eq!(obj.body, plaintext);
}

#[tokio::test]
async fn test_bucket_not_empty_delete() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store
        .put_object("test", "obj", b"data".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None)
        .await
        .unwrap();

    // Cannot delete non-empty bucket
    let err = store.delete_bucket("test").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn test_delete_objects_batch() {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    store.create_bucket("test", "us-east-1", "cave").await.unwrap();
    store.put_object("test", "a", b"1".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    store.put_object("test", "b", b"2".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();
    store.put_object("test", "c", b"3".to_vec(), "text/plain", HashMap::new(), HashMap::new(), None, None, None).await.unwrap();

    let entries = vec![
        crate::s3::store::DeleteObjectEntry { key: "a".to_string(), version_id: None },
        crate::s3::store::DeleteObjectEntry { key: "b".to_string(), version_id: None },
        crate::s3::store::DeleteObjectEntry { key: "nonexistent".to_string(), version_id: None },
    ];

    let results = store.delete_objects("test", entries).await.unwrap();
    let errors: Vec<_> = results.iter().filter(|r| r.error.is_some()).collect();
    let deleted: Vec<_> = results.iter().filter(|r| r.error.is_none()).collect();

    assert_eq!(deleted.len(), 2);
    assert_eq!(errors.len(), 1);

    // c still exists
    assert!(store.head_object("test", "c", None).await.is_ok());
}
