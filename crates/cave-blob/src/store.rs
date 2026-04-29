//! In-memory bucket / object store backing both API surfaces.

use std::collections::BTreeMap;

use parking_lot::RwLock;

use crate::error::{BlobError, BlobResult};
use crate::object::{Object, ObjectMeta};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketMeta {
    pub name: String,
    pub tenant: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub versioning_enabled: bool,
    pub immutability_default: Option<chrono::Duration>,
}

#[derive(Debug)]
pub struct Bucket {
    pub meta: BucketMeta,
    /// key → list of versions (latest last). When versioning is disabled the
    /// vec always has exactly one entry (the current object).
    objects: BTreeMap<String, Vec<Object>>,
}

#[derive(Debug, Default)]
pub struct Store {
    buckets: RwLock<BTreeMap<String, Bucket>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_bucket(&self, meta: BucketMeta) -> BlobResult<()> {
        validate_bucket_name(&meta.name)?;
        let mut buckets = self.buckets.write();
        if buckets.contains_key(&meta.name) {
            return Err(BlobError::BucketExists(meta.name.clone()));
        }
        buckets.insert(
            meta.name.clone(),
            Bucket {
                meta,
                objects: BTreeMap::new(),
            },
        );
        Ok(())
    }

    pub fn delete_bucket(&self, name: &str) -> BlobResult<()> {
        let mut buckets = self.buckets.write();
        if buckets.remove(name).is_none() {
            return Err(BlobError::BucketNotFound(name.into()));
        }
        Ok(())
    }

    pub fn list_buckets(&self) -> Vec<BucketMeta> {
        self.buckets
            .read()
            .values()
            .map(|b| b.meta.clone())
            .collect()
    }

    pub fn list_buckets_for_tenant(&self, tenant: &str) -> Vec<BucketMeta> {
        self.buckets
            .read()
            .values()
            .filter(|b| b.meta.tenant == tenant)
            .map(|b| b.meta.clone())
            .collect()
    }

    pub fn put_object(&self, obj: Object) -> BlobResult<ObjectMeta> {
        validate_key(&obj.key())?;
        let mut buckets = self.buckets.write();
        let bucket = buckets
            .get_mut(&obj.meta.bucket)
            .ok_or_else(|| BlobError::BucketNotFound(obj.meta.bucket.clone()))?;
        let key = obj.key().to_string();
        let versioning = bucket.meta.versioning_enabled;
        let entry = bucket.objects.entry(key).or_default();
        if !versioning {
            entry.clear();
        }
        let meta = obj.meta.clone();
        entry.push(obj);
        Ok(meta)
    }

    pub fn get_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> BlobResult<Object> {
        let buckets = self.buckets.read();
        let b = buckets
            .get(bucket)
            .ok_or_else(|| BlobError::BucketNotFound(bucket.into()))?;
        let versions = b
            .objects
            .get(key)
            .ok_or_else(|| BlobError::ObjectNotFound(bucket.into(), key.into()))?;
        let obj = match version_id {
            None => versions.last().cloned(),
            Some(v) => versions
                .iter()
                .find(|o| o.meta.version_id.as_deref() == Some(v))
                .cloned(),
        };
        obj.ok_or_else(|| BlobError::ObjectNotFound(bucket.into(), key.into()))
    }

    pub fn head_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> BlobResult<ObjectMeta> {
        self.get_object(bucket, key, version_id).map(|o| o.meta)
    }

    pub fn delete_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> BlobResult<()> {
        let mut buckets = self.buckets.write();
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| BlobError::BucketNotFound(bucket.into()))?;
        let versions = b
            .objects
            .get_mut(key)
            .ok_or_else(|| BlobError::ObjectNotFound(bucket.into(), key.into()))?;
        for v in versions.iter() {
            if let Some(until) = v.meta.immutable_until {
                if until > now {
                    return Err(BlobError::Immutable);
                }
            }
        }
        match version_id {
            None => {
                versions.clear();
                b.objects.remove(key);
            }
            Some(vid) => {
                versions.retain(|o| o.meta.version_id.as_deref() != Some(vid));
                if versions.is_empty() {
                    b.objects.remove(key);
                }
            }
        }
        Ok(())
    }

    pub fn list_objects(&self, bucket: &str, prefix: &str) -> BlobResult<Vec<ObjectMeta>> {
        let buckets = self.buckets.read();
        let b = buckets
            .get(bucket)
            .ok_or_else(|| BlobError::BucketNotFound(bucket.into()))?;
        Ok(b.objects
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .filter_map(|(_, vs)| vs.last().map(|o| o.meta.clone()))
            .collect())
    }

    pub fn bucket_meta(&self, name: &str) -> BlobResult<BucketMeta> {
        let buckets = self.buckets.read();
        buckets
            .get(name)
            .map(|b| b.meta.clone())
            .ok_or_else(|| BlobError::BucketNotFound(name.into()))
    }

    /// Toggle versioning on an existing bucket. Returns `BucketNotFound` if
    /// the bucket has not been created.
    pub fn set_versioning(&self, name: &str, enabled: bool) -> BlobResult<()> {
        let mut buckets = self.buckets.write();
        let b = buckets
            .get_mut(name)
            .ok_or_else(|| BlobError::BucketNotFound(name.into()))?;
        b.meta.versioning_enabled = enabled;
        Ok(())
    }
}

impl Object {
    fn key(&self) -> &str {
        &self.meta.key
    }
}

/// S3 bucket-name rules: 3-63 chars, lowercase letters, digits, dots, hyphens.
/// Cannot start/end with hyphen or dot, no consecutive dots.
pub fn validate_bucket_name(name: &str) -> BlobResult<()> {
    if !(3..=63).contains(&name.len()) {
        return Err(BlobError::InvalidBucketName(format!("len {}", name.len())));
    }
    let bytes = name.as_bytes();
    if bytes[0] == b'-' || bytes[0] == b'.' || bytes[bytes.len() - 1] == b'-' || bytes[bytes.len() - 1] == b'.' {
        return Err(BlobError::InvalidBucketName("leading/trailing".into()));
    }
    let mut prev = 0u8;
    for &b in bytes {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.';
        if !ok {
            return Err(BlobError::InvalidBucketName(format!("char {:?}", b as char)));
        }
        if prev == b'.' && b == b'.' {
            return Err(BlobError::InvalidBucketName("consecutive dots".into()));
        }
        prev = b;
    }
    Ok(())
}

pub fn validate_key(key: &str) -> BlobResult<()> {
    if key.is_empty() {
        return Err(BlobError::InvalidKey("empty".into()));
    }
    if key.len() > 1024 {
        return Err(BlobError::InvalidKey("> 1024 bytes".into()));
    }
    if key.contains('\0') {
        return Err(BlobError::InvalidKey("nul byte".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn bm(name: &str, tenant: &str) -> BucketMeta {
        BucketMeta {
            name: name.into(),
            tenant: tenant.into(),
            created_at: Utc::now(),
            versioning_enabled: false,
            immutability_default: None,
        }
    }

    #[test]
    fn validate_bucket_name_simple_ok() {
        assert!(validate_bucket_name("my-bucket").is_ok());
    }
    #[test]
    fn validate_bucket_name_too_short_errors() {
        assert!(validate_bucket_name("ab").is_err());
    }
    #[test]
    fn validate_bucket_name_too_long_errors() {
        assert!(validate_bucket_name(&"a".repeat(64)).is_err());
    }
    #[test]
    fn validate_bucket_name_uppercase_errors() {
        assert!(validate_bucket_name("MyBucket").is_err());
    }
    #[test]
    fn validate_bucket_name_consecutive_dots_errors() {
        assert!(validate_bucket_name("a..b").is_err());
    }
    #[test]
    fn validate_bucket_name_leading_dot_errors() {
        assert!(validate_bucket_name(".abc").is_err());
    }

    #[test]
    fn create_bucket_then_list() {
        let s = Store::new();
        s.create_bucket(bm("a-b", "acme")).unwrap();
        s.create_bucket(bm("c-d", "acme")).unwrap();
        assert_eq!(s.list_buckets().len(), 2);
    }
    #[test]
    fn create_bucket_twice_errors() {
        let s = Store::new();
        s.create_bucket(bm("a-b", "acme")).unwrap();
        assert!(matches!(
            s.create_bucket(bm("a-b", "acme")),
            Err(BlobError::BucketExists(_))
        ));
    }
    #[test]
    fn delete_bucket_removes() {
        let s = Store::new();
        s.create_bucket(bm("x-y", "acme")).unwrap();
        s.delete_bucket("x-y").unwrap();
        assert!(s.list_buckets().is_empty());
    }
    #[test]
    fn list_buckets_for_tenant_filters() {
        let s = Store::new();
        s.create_bucket(bm("a-b", "acme")).unwrap();
        s.create_bucket(bm("c-d", "wile")).unwrap();
        assert_eq!(s.list_buckets_for_tenant("acme").len(), 1);
    }

    #[test]
    fn put_then_get_object() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        let o = Object::new("bucket1", "k1", b"hi".to_vec(), "text/plain");
        s.put_object(o.clone()).unwrap();
        let got = s.get_object("bucket1", "k1", None).unwrap();
        assert_eq!(got.body, b"hi");
    }
    #[test]
    fn put_overwrites_when_versioning_off() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        s.put_object(Object::new("bucket1", "k", b"old".to_vec(), "t"))
            .unwrap();
        s.put_object(Object::new("bucket1", "k", b"new".to_vec(), "t"))
            .unwrap();
        assert_eq!(s.get_object("bucket1", "k", None).unwrap().body, b"new");
    }
    #[test]
    fn put_keeps_versions_when_enabled() {
        let s = Store::new();
        let mut bm = bm("bucket1", "acme");
        bm.versioning_enabled = true;
        s.create_bucket(bm).unwrap();
        let mut o1 = Object::new("bucket1", "k", b"v1".to_vec(), "t");
        o1.meta.version_id = Some("v1".into());
        let mut o2 = Object::new("bucket1", "k", b"v2".to_vec(), "t");
        o2.meta.version_id = Some("v2".into());
        s.put_object(o1).unwrap();
        s.put_object(o2).unwrap();
        assert_eq!(s.get_object("bucket1", "k", Some("v1")).unwrap().body, b"v1");
        assert_eq!(s.get_object("bucket1", "k", Some("v2")).unwrap().body, b"v2");
    }
    #[test]
    fn delete_object_removes() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        s.put_object(Object::new("bucket1", "k", b"x".to_vec(), "t"))
            .unwrap();
        s.delete_object("bucket1", "k", None, Utc::now()).unwrap();
        assert!(matches!(
            s.get_object("bucket1", "k", None),
            Err(BlobError::ObjectNotFound(_, _))
        ));
    }
    #[test]
    fn delete_object_immutable_blocked() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        let mut o = Object::new("bucket1", "k", b"x".to_vec(), "t");
        o.meta.immutable_until = Some(Utc::now() + chrono::Duration::days(1));
        s.put_object(o).unwrap();
        let err = s.delete_object("bucket1", "k", None, Utc::now()).unwrap_err();
        assert_eq!(err, BlobError::Immutable);
    }
    #[test]
    fn list_objects_with_prefix() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        s.put_object(Object::new("bucket1", "logs/a.txt", vec![], "t"))
            .unwrap();
        s.put_object(Object::new("bucket1", "logs/b.txt", vec![], "t"))
            .unwrap();
        s.put_object(Object::new("bucket1", "data/c.txt", vec![], "t"))
            .unwrap();
        let logs = s.list_objects("bucket1", "logs/").unwrap();
        assert_eq!(logs.len(), 2);
    }
    #[test]
    fn head_object_returns_meta_only() {
        let s = Store::new();
        s.create_bucket(bm("bucket1", "acme")).unwrap();
        s.put_object(Object::new("bucket1", "k", b"hi".to_vec(), "t"))
            .unwrap();
        let m = s.head_object("bucket1", "k", None).unwrap();
        assert_eq!(m.size, 2);
    }

    #[test]
    fn invalid_key_empty() {
        assert!(validate_key("").is_err());
    }
    #[test]
    fn invalid_key_too_long() {
        assert!(validate_key(&"k".repeat(1025)).is_err());
    }
    #[test]
    fn invalid_key_with_nul() {
        assert!(validate_key("a\0b").is_err());
    }
    #[test]
    fn valid_key_with_slashes() {
        assert!(validate_key("a/b/c.txt").is_ok());
    }
}
