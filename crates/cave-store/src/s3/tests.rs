// SPDX-License-Identifier: AGPL-3.0-or-later
#[cfg(test)]
mod bucket_tests {
    use tempfile::TempDir;
    use crate::s3::S3Store;

    fn store(dir: &TempDir) -> S3Store {
        S3Store::new(dir.path(), vec![0u8; 32]).unwrap()
    }

    #[test]
    fn create_and_list_buckets() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.create_bucket("my-bucket".into(), "us-east-1".into()).unwrap();
        let buckets = s.list_buckets();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].name, "my-bucket");
    }

    #[test]
    fn duplicate_bucket_fails() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.create_bucket("bkt".into(), "us-east-1".into()).unwrap();
        assert!(s.create_bucket("bkt".into(), "us-east-1".into()).is_err());
    }

    #[test]
    fn delete_empty_bucket() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.create_bucket("bkt".into(), "us-east-1".into()).unwrap();
        s.delete_bucket("bkt").unwrap();
        assert!(s.list_buckets().is_empty());
    }

    #[test]
    fn delete_non_empty_bucket_fails() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.create_bucket("bkt".into(), "us-east-1".into()).unwrap();
        s.put_object("bkt", "key", bytes::Bytes::from("data"), Default::default(), None, None).unwrap();
        assert!(s.delete_bucket("bkt").is_err());
    }

    #[test]
    fn head_bucket() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        s.create_bucket("bkt".into(), "us-east-1".into()).unwrap();
        let info = s.head_bucket("bkt").unwrap();
        assert_eq!(info.name, "bkt");
        assert!(s.head_bucket("missing-bucket").is_err());
    }

    #[test]
    fn invalid_bucket_name() {
        let dir = TempDir::new().unwrap();
        let s = store(&dir);
        assert!(s.create_bucket("UPPERCASE".into(), "us-east-1".into()).is_err()); // uppercase
        assert!(s.create_bucket("ab".into(), "us-east-1".into()).is_err()); // too short
    }
}

#[cfg(test)]
mod object_tests {
    use std::collections::HashMap;
    use bytes::Bytes;
    use tempfile::TempDir;
    use crate::s3::{S3Store, types::SseConfig, types::SseAlgorithm};

    fn store_with_bucket(dir: &TempDir) -> S3Store {
        let s = S3Store::new(dir.path(), vec![0u8; 32]).unwrap();
        s.create_bucket("test".into(), "us-east-1".into()).unwrap();
        s
    }

    #[test]
    fn put_and_get() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let data = Bytes::from("hello world");
        let meta = s.put_object("test", "foo/bar.txt", data.clone(), HashMap::new(), Some("text/plain".into()), None).unwrap();
        assert_eq!(meta.size, 11);
        assert_eq!(meta.content_type, "text/plain");

        let (got_meta, got_data) = s.get_object("test", "foo/bar.txt", None, None, None).unwrap();
        assert_eq!(got_data, data);
        assert_eq!(got_meta.size, 11);
    }

    #[test]
    fn head_object() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_object("test", "k", Bytes::from("v"), HashMap::new(), None, None).unwrap();
        let meta = s.head_object("test", "k", None).unwrap();
        assert_eq!(meta.key, "k");
        assert_eq!(meta.size, 1);
    }

    #[test]
    fn delete_object() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_object("test", "k", Bytes::from("v"), HashMap::new(), None, None).unwrap();
        s.delete_object("test", "k", None).unwrap();
        assert!(s.head_object("test", "k", None).is_err());
    }

    #[test]
    fn get_nonexistent_object() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        assert!(s.get_object("test", "missing", None, None, None).is_err());
    }

    #[test]
    fn range_get() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let data = Bytes::from("0123456789");
        s.put_object("test", "k", data, HashMap::new(), None, None).unwrap();
        let (_, slice) = s.get_object("test", "k", None, Some((2, 5)), None).unwrap();
        assert_eq!(slice.as_ref(), b"2345");
    }

    #[test]
    fn sse_s3_roundtrip() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let plaintext = Bytes::from("secret data");
        let sse = Some(SseConfig { algorithm: SseAlgorithm::AwsS3, customer_key: None });
        let meta = s.put_object("test", "encrypted", plaintext.clone(), HashMap::new(), None, sse).unwrap();
        assert_eq!(meta.sse_algorithm, Some(SseAlgorithm::AwsS3));

        let (_, decrypted) = s.get_object("test", "encrypted", None, None, None).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn sse_c_roundtrip() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let key = vec![42u8; 32];
        let plaintext = Bytes::from("customer-encrypted");
        let sse = Some(SseConfig { algorithm: SseAlgorithm::Customer, customer_key: Some(key.clone()) });
        s.put_object("test", "ssec", plaintext.clone(), HashMap::new(), None, sse).unwrap();

        let (_, decrypted) = s.get_object("test", "ssec", None, None, Some(&key)).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn copy_object() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.create_bucket("dst".into(), "us-east-1".into()).unwrap();
        let data = Bytes::from("copy me");
        s.put_object("test", "src", data.clone(), HashMap::new(), None, None).unwrap();
        s.copy_object("test", "src", "dst", "dst-key").unwrap();
        let (_, got) = s.get_object("dst", "dst-key", None, None, None).unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn list_objects_prefix_filter() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_object("test", "logs/2024/01.log", Bytes::from("a"), HashMap::new(), None, None).unwrap();
        s.put_object("test", "logs/2024/02.log", Bytes::from("b"), HashMap::new(), None, None).unwrap();
        s.put_object("test", "data/file.bin", Bytes::from("c"), HashMap::new(), None, None).unwrap();

        let result = s.list_objects_v2("test", "logs/", "", None, 1000).unwrap();
        assert_eq!(result.key_count, 2);
        assert!(result.contents.iter().all(|o| o.key.starts_with("logs/")));
    }

    #[test]
    fn list_objects_with_delimiter() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_object("test", "a/1.txt", Bytes::from("x"), HashMap::new(), None, None).unwrap();
        s.put_object("test", "a/2.txt", Bytes::from("x"), HashMap::new(), None, None).unwrap();
        s.put_object("test", "b/1.txt", Bytes::from("x"), HashMap::new(), None, None).unwrap();

        let result = s.list_objects_v2("test", "", "/", None, 1000).unwrap();
        assert_eq!(result.common_prefixes.len(), 2);
        assert!(result.common_prefixes.contains(&"a/".to_string()));
        assert!(result.common_prefixes.contains(&"b/".to_string()));
    }

    #[test]
    fn list_objects_pagination() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        for i in 0..5 {
            s.put_object("test", &format!("key-{i}"), Bytes::from("v"), HashMap::new(), None, None).unwrap();
        }

        let page1 = s.list_objects_v2("test", "", "", None, 3).unwrap();
        assert_eq!(page1.key_count, 3);
        assert!(page1.truncated);
        let token = page1.next_continuation_token.unwrap();

        let page2 = s.list_objects_v2("test", "", "", Some(&token), 3).unwrap();
        assert_eq!(page2.key_count, 2);
        assert!(!page2.truncated);
    }
}

#[cfg(test)]
mod multipart_tests {
    use std::collections::HashMap;
    use bytes::Bytes;
    use tempfile::TempDir;
    use crate::s3::S3Store;

    fn store_with_bucket(dir: &TempDir) -> S3Store {
        let s = S3Store::new(dir.path(), vec![0u8; 32]).unwrap();
        s.create_bucket("test".into(), "us-east-1".into()).unwrap();
        s
    }

    #[test]
    fn full_multipart_flow() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);

        let upload_id = s.create_multipart_upload("test", "big-file", HashMap::new(), None).unwrap();
        let etag1 = s.upload_part("test", "big-file", &upload_id, 1, Bytes::from("part-one-data")).unwrap();
        let etag2 = s.upload_part("test", "big-file", &upload_id, 2, Bytes::from("part-two-data")).unwrap();

        let parts = vec![(1u32, etag1), (2u32, etag2)];
        let meta = s.complete_multipart_upload("test", "big-file", &upload_id, parts).unwrap();
        assert_eq!(meta.size, "part-one-data".len() as u64 + "part-two-data".len() as u64);

        let (_, data) = s.get_object("test", "big-file", None, None, None).unwrap();
        assert_eq!(data.as_ref(), b"part-one-datapart-two-data");
    }

    #[test]
    fn abort_multipart() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);

        let upload_id = s.create_multipart_upload("test", "k", HashMap::new(), None).unwrap();
        s.upload_part("test", "k", &upload_id, 1, Bytes::from("data")).unwrap();
        s.abort_multipart_upload("test", "k", &upload_id).unwrap();

        assert!(s.list_multipart_uploads("test").unwrap().is_empty());
    }

    #[test]
    fn list_multipart_uploads() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);

        s.create_multipart_upload("test", "k1", HashMap::new(), None).unwrap();
        s.create_multipart_upload("test", "k2", HashMap::new(), None).unwrap();

        let uploads = s.list_multipart_uploads("test").unwrap();
        assert_eq!(uploads.len(), 2);
    }
}

#[cfg(test)]
mod versioning_tests {
    use std::collections::HashMap;
    use bytes::Bytes;
    use tempfile::TempDir;
    use crate::s3::{S3Store, types::{BucketVersioning, VersioningStatus}};

    fn store_with_bucket(dir: &TempDir) -> S3Store {
        let s = S3Store::new(dir.path(), vec![0u8; 32]).unwrap();
        s.create_bucket("test".into(), "us-east-1".into()).unwrap();
        s
    }

    #[test]
    fn versioning_default_off() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let v = s.get_bucket_versioning("test").unwrap();
        assert_eq!(v.status, VersioningStatus::Off);
    }

    #[test]
    fn enable_versioning() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_bucket_versioning("test", BucketVersioning { status: VersioningStatus::Enabled }).unwrap();
        let v = s.get_bucket_versioning("test").unwrap();
        assert_eq!(v.status, VersioningStatus::Enabled);
    }

    #[test]
    fn versioned_puts_create_versions() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        s.put_bucket_versioning("test", BucketVersioning { status: VersioningStatus::Enabled }).unwrap();

        let m1 = s.put_object("test", "k", Bytes::from("v1"), HashMap::new(), None, None).unwrap();
        let m2 = s.put_object("test", "k", Bytes::from("v2"), HashMap::new(), None, None).unwrap();

        // Both version IDs should be set and different
        assert!(m1.version_id.is_some());
        assert!(m2.version_id.is_some());
        assert_ne!(m1.version_id, m2.version_id);

        // Get by version ID
        let (_, d1) = s.get_object("test", "k", m1.version_id.as_deref(), None, None).unwrap();
        assert_eq!(d1.as_ref(), b"v1");
    }
}

#[cfg(test)]
mod lifecycle_policy_tests {
    use tempfile::TempDir;
    use crate::s3::{S3Store, types::{LifecycleRule, BucketPolicy}};

    fn store_with_bucket(dir: &TempDir) -> S3Store {
        let s = S3Store::new(dir.path(), vec![0u8; 32]).unwrap();
        s.create_bucket("test".into(), "us-east-1".into()).unwrap();
        s
    }

    #[test]
    fn lifecycle_roundtrip() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let rules = vec![LifecycleRule {
            id: "expire-old".into(),
            prefix: "logs/".into(),
            expiration_days: Some(30),
            noncurrent_version_expiration_days: None,
            enabled: true,
        }];
        s.put_bucket_lifecycle("test", rules.clone()).unwrap();
        let got = s.get_bucket_lifecycle("test").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "expire-old");
        assert_eq!(got[0].expiration_days, Some(30));
    }

    #[test]
    fn policy_roundtrip() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let policy = BucketPolicy {
            version: "2012-10-17".into(),
            statements: vec![],
        };
        s.put_bucket_policy("test", policy).unwrap();
        let got = s.get_bucket_policy("test").unwrap();
        assert_eq!(got.version, "2012-10-17");

        s.delete_bucket_policy("test").unwrap();
        assert!(s.get_bucket_policy("test").is_err());
    }

    #[test]
    fn presign_url() {
        let dir = TempDir::new().unwrap();
        let s = store_with_bucket(&dir);
        let url = s.presign_url("test", "mykey", "GET", 3600, "http://localhost:9000").unwrap();
        assert!(url.contains("test"));
        assert!(url.contains("mykey"));
        assert!(url.contains("X-Amz-Signature="));
    }
}
