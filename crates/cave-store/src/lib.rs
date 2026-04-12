//! cave-store — MinIO replacement for object storage management.
//!
//! Replaces: MinIO, AWS S3 (dev/platform use)
//! Features: bucket CRUD, put/get/delete objects, multipart upload,
//!           versioning, lifecycle rules, access policies, replication rules.

pub mod models;
pub mod routes;
pub mod storage;

use axum::Router;
use std::sync::{Arc, Mutex};

/// Shared state for the object store module.
pub struct StoreState {
    pub inner: Mutex<storage::ObjectStore>,
}

impl StoreState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(storage::ObjectStore::new()),
        }
    }
}

impl Default for StoreState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<StoreState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "store";
//! cave-store — Object storage, S3/MinIO replacement.
pub mod encryption;
pub mod error;
pub mod lifecycle;
pub mod multipart;
pub mod notification;
pub mod policy;
pub mod presigned;
pub mod routes;
pub mod store;
pub mod types;
pub mod versioning;
pub use error::{StoreError, StoreResult};
pub use store::ObjectStore;
pub const MODULE_NAME: &str = "store";
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use chrono::Duration;
    use super::encryption::EncryptionEngine;
    use super::lifecycle::{LifecycleAction, LifecycleManager};
    use super::notification::NotificationDispatcher;
    use super::policy::{PolicyEffect, PolicyEvaluator};
    use super::presigned::PresignConfig;
    use super::store::ObjectStore;
    use super::types::{
        BucketPolicy, CannedAcl, LifecycleRule, NotificationConfig, PolicyStatement,
        QueueConfig, VersioningState,
    };
    async fn fresh_store() -> ObjectStore {
        ObjectStore::new()
    }
    // ── Test 1: create_bucket ───────────────────────────────────────────────
    #[tokio::test]
    async fn test_create_bucket() {
        let store = fresh_store().await;
        store.create_bucket("my-bucket", "us-east-1").await.unwrap();
        let info = store.head_bucket("my-bucket").await.unwrap();
        assert_eq!(info.name, "my-bucket");
        assert_eq!(info.region, "us-east-1");
    }
    // ── Test 2: delete_bucket ───────────────────────────────────────────────
    #[tokio::test]
    async fn test_delete_bucket() {
        let store = fresh_store().await;
        store.create_bucket("del-bucket", "us-east-1").await.unwrap();
        store.delete_bucket("del-bucket").await.unwrap();
        let result = store.head_bucket("del-bucket").await;
        assert!(result.is_err());
    }
    // ── Test 3: list_buckets ────────────────────────────────────────────────
    #[tokio::test]
    async fn test_list_buckets() {
        let store = fresh_store().await;
        store.create_bucket("bucket-a", "us-east-1").await.unwrap();
        store.create_bucket("bucket-b", "eu-west-1").await.unwrap();
        let buckets = store.list_buckets().await;
        assert_eq!(buckets.len(), 2);
    }
    // ── Test 4: put_object + get_object ────────────────────────────────────
    #[tokio::test]
    async fn test_put_get_object() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        store.put_object("bkt", "key1", b"hello world".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        let (version, data) = store.get_object("bkt", "key1", None).await.unwrap();
        assert_eq!(data, b"hello world".to_vec());
        assert_eq!(version.content_type, "text/plain");
        assert_eq!(version.size, 11);
    }
    // ── Test 5: delete_object ───────────────────────────────────────────────
    #[tokio::test]
    async fn test_delete_object() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        store.put_object("bkt", "k", b"data".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        store.delete_object("bkt", "k", None).await.unwrap();
        let result = store.get_object("bkt", "k", None).await;
        assert!(result.is_err());
    }
    // ── Test 6: head_object ─────────────────────────────────────────────────
    #[tokio::test]
    async fn test_head_object() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        store.put_object("bkt", "k", b"data".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        let info = store.head_object("bkt", "k").await.unwrap();
        assert_eq!(info.size, 4);
        assert_eq!(info.content_type, "text/plain");
    }
    // ── Test 7: copy_object ─────────────────────────────────────────────────
    #[tokio::test]
    async fn test_copy_object() {
        let store = fresh_store().await;
        store.create_bucket("src", "us-east-1").await.unwrap();
        store.create_bucket("dst", "us-east-1").await.unwrap();
        store.put_object("src", "orig", b"content".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        store.copy_object("src", "orig", "dst", "copy").await.unwrap();
        let (_, data) = store.get_object("dst", "copy", None).await.unwrap();
        assert_eq!(data, b"content".to_vec());
    }
    // ── Test 8: list_objects_v2 with prefix ─────────────────────────────────
    #[tokio::test]
    async fn test_list_objects_prefix() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        store.put_object("bkt", "docs/a.txt", b"a".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        store.put_object("bkt", "docs/b.txt", b"b".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        store.put_object("bkt", "images/c.png", b"c".to_vec(), "image/png", HashMap::new(), None).await.unwrap();
        let result = store.list_objects_v2("bkt", Some("docs/"), None, None, None).await.unwrap();
        assert_eq!(result.objects.len(), 2);
        assert!(result.objects.iter().all(|o| o.key.starts_with("docs/")));
    }
    // ── Test 9: object versioning ───────────────────────────────────────────
    #[tokio::test]
    async fn test_versioning() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        store.set_bucket_versioning("bkt", VersioningState::Enabled).await.unwrap();
        let info1 = store.put_object("bkt", "k", b"v1".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        let info2 = store.put_object("bkt", "k", b"v2".to_vec(), "text/plain", HashMap::new(), None).await.unwrap();
        let vid1 = info1.version_id.unwrap();
        let _vid2 = info2.version_id.unwrap();
        // Latest version
        let (_, latest) = store.get_object("bkt", "k", None).await.unwrap();
        assert_eq!(latest, b"v2".to_vec());
        // Get by version_id
        let (_, v1) = store.get_object("bkt", "k", Some(&vid1)).await.unwrap();
        assert_eq!(v1, b"v1".to_vec());
    }
    // ── Test 10: multipart upload ───────────────────────────────────────────
    #[tokio::test]
    async fn test_multipart_upload() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        let upload_id = store.create_multipart_upload("bkt", "big-file", HashMap::new()).await.unwrap();
        let etag1 = store.upload_part(&upload_id, 1, b"part1-data".to_vec()).await.unwrap();
        let etag2 = store.upload_part(&upload_id, 2, b"part2-data".to_vec()).await.unwrap();
        let etag3 = store.upload_part(&upload_id, 3, b"part3-data".to_vec()).await.unwrap();
        let parts: Vec<(u32, String)> = vec![(1, etag1), (2, etag2), (3, etag3)];
        let info = store.complete_multipart_upload(&upload_id, parts).await.unwrap();
        let (_, data) = store.get_object("bkt", "big-file", None).await.unwrap();
        assert_eq!(data, b"part1-datapart2-datapart3-data".to_vec());
        assert_eq!(info.size, 30);
    }
    // ── Test 11: abort_multipart_upload ─────────────────────────────────────
    #[tokio::test]
    async fn test_abort_multipart() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        let upload_id = store.create_multipart_upload("bkt", "k", HashMap::new()).await.unwrap();
        store.upload_part(&upload_id, 1, b"data".to_vec()).await.unwrap();
        store.abort_multipart_upload(&upload_id).await.unwrap();
        let result = store.abort_multipart_upload(&upload_id).await;
        assert!(result.is_err()); // already removed
    }
    // ── Test 12: presigned URL generation + verify ──────────────────────────
    #[test]
    fn test_presigned_url() {
        let config = PresignConfig::new(
            "AKID123",
            b"supersecretkey",
            "us-east-1",
            "http://localhost:9000",
        );
        let url = config.presign_get("my-bucket", "my-key", Duration::hours(1));
        assert!(url.url.contains("my-bucket"));
        assert!(url.url.contains("my-key"));
        assert!(url.url.contains("X-Signature="));
        assert!(config.verify(&url.url));
    }
    // ── Test 13: lifecycle should_expire ────────────────────────────────────
    #[test]
    fn test_lifecycle_should_expire() {
        let rule = LifecycleRule {
            id: "r1".to_string(),
            prefix: "".to_string(),
            enabled: true,
            expiration_days: Some(30),
            transition_days: None,
            transition_storage_class: None,
            noncurrent_expiration_days: None,
        };
        assert!(LifecycleManager::should_expire(&rule, 30));
        assert!(LifecycleManager::should_expire(&rule, 31));
        assert!(!LifecycleManager::should_expire(&rule, 29));
    }
    // ── Test 14: lifecycle evaluate_rules ───────────────────────────────────
    #[test]
    fn test_lifecycle_evaluate_rules() {
        let rules = vec![
            LifecycleRule {
                id: "expire-old".to_string(),
                prefix: "logs/".to_string(),
                enabled: true,
                expiration_days: Some(90),
                transition_days: None,
                transition_storage_class: None,
                noncurrent_expiration_days: None,
            },
        ];
        let objects = vec![
            ("logs/old.log".to_string(), 100u32),
            ("logs/new.log".to_string(), 10u32),
            ("data/file.dat".to_string(), 200u32),
        ];
        let actions = LifecycleManager::evaluate_rules(&rules, &objects);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].0, "logs/old.log");
        assert!(matches!(actions[0].1, LifecycleAction::Expire));
    }
    // ── Test 15: policy evaluation (allow/deny) ─────────────────────────────
    #[test]
    fn test_policy_evaluation() {
        let policy = BucketPolicy {
            version: "2012-10-17".to_string(),
            statements: vec![
                PolicyStatement {
                    effect: "Allow".to_string(),
                    principal: vec!["user123".to_string()],
                    action: vec!["s3:GetObject".to_string()],
                    resource: vec!["arn:aws:s3:::my-bucket/*".to_string()],
                },
                PolicyStatement {
                    effect: "Deny".to_string(),
                    principal: vec!["*".to_string()],
                    action: vec!["s3:DeleteObject".to_string()],
                    resource: vec!["*".to_string()],
                },
            ],
        };
        let effect = PolicyEvaluator::evaluate(
            &policy,
            "s3:GetObject",
            "arn:aws:s3:::my-bucket/file.txt",
            "user123",
        );
        assert!(matches!(effect, PolicyEffect::Allow));
        let effect = PolicyEvaluator::evaluate(
            &policy,
            "s3:DeleteObject",
            "arn:aws:s3:::my-bucket/file.txt",
            "anyone",
        );
        assert!(matches!(effect, PolicyEffect::Deny));
        let effect = PolicyEvaluator::evaluate(
            &policy,
            "s3:PutObject",
            "arn:aws:s3:::my-bucket/file.txt",
            "user123",
        );
        assert!(matches!(effect, PolicyEffect::NoMatch));
    }
    // ── Test 16: encryption SSE-S3 roundtrip ────────────────────────────────
    #[test]
    fn test_encryption_sse_s3_roundtrip() {
        let plaintext = b"Hello, secret world!";
        let encrypted = EncryptionEngine::encrypt_sse_s3(plaintext, "key-id-abc");
        assert_ne!(encrypted, plaintext);
        let decrypted = EncryptionEngine::decrypt_sse_s3(&encrypted, "key-id-abc");
        assert_eq!(decrypted, plaintext);
    }
    // ── Test 17: notification should_notify ─────────────────────────────────
    #[test]
    fn test_notification_should_notify() {
        let config = NotificationConfig {
            queue_configurations: vec![QueueConfig {
                id: "q1".to_string(),
                queue_arn: "arn:aws:sqs:us-east-1:123456789:my-queue".to_string(),
                events: vec!["s3:ObjectCreated:*".to_string()],
                prefix_filter: Some("uploads/".to_string()),
            }],
        };
        assert!(NotificationDispatcher::should_notify(
            &config, "s3:ObjectCreated:Put", "uploads/file.txt"
        ));
        assert!(!NotificationDispatcher::should_notify(
            &config, "s3:ObjectCreated:Put", "other/file.txt"
        ));
        assert!(!NotificationDispatcher::should_notify(
            &config, "s3:ObjectRemoved:Delete", "uploads/file.txt"
        ));
    }
    // ── Test 18: bucket policy + acl set/get ────────────────────────────────
    #[tokio::test]
    async fn test_bucket_policy_acl() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        let policy = BucketPolicy {
            version: "2012-10-17".to_string(),
            statements: vec![PolicyStatement {
                effect: "Allow".to_string(),
                principal: vec!["*".to_string()],
                action: vec!["s3:GetObject".to_string()],
                resource: vec!["*".to_string()],
            }],
        };
        store.put_bucket_policy("bkt", policy).await.unwrap();
        let fetched = store.get_bucket_policy("bkt").await.unwrap();
        assert_eq!(fetched.statements.len(), 1);
        store.put_bucket_acl("bkt", CannedAcl::PublicRead).await.unwrap();
    }
    // ── Bonus: versioning state set/get ─────────────────────────────────────
    #[tokio::test]
    async fn test_versioning_state() {
        let store = fresh_store().await;
        store.create_bucket("bkt", "us-east-1").await.unwrap();
        assert_eq!(store.get_bucket_versioning("bkt").await.unwrap(), VersioningState::Disabled);
        store.set_bucket_versioning("bkt", VersioningState::Enabled).await.unwrap();
        assert_eq!(store.get_bucket_versioning("bkt").await.unwrap(), VersioningState::Enabled);
    }
    // ── Bonus: SSE-C encryption roundtrip ───────────────────────────────────
    #[test]
    fn test_encryption_sse_c_roundtrip() {
        let plaintext = b"Customer secret data";
        let customer_key = b"my-32-byte-customer-key-12345678";
        let encrypted = EncryptionEngine::encrypt_sse_c(plaintext, customer_key);
        assert_ne!(encrypted, plaintext.as_slice());
        let decrypted = EncryptionEngine::decrypt_sse_c(&encrypted, customer_key);
        assert_eq!(decrypted, plaintext);
    }
}
