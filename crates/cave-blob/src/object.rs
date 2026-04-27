//! Object metadata + ETag computation.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub bucket: String,
    pub key: String,
    pub etag: String,
    pub size: u64,
    pub content_type: String,
    pub last_modified: DateTime<Utc>,
    pub headers: BTreeMap<String, String>,
    pub version_id: Option<String>,
    pub immutable_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub meta: ObjectMeta,
    pub body: Vec<u8>,
}

impl Object {
    pub fn new(bucket: &str, key: &str, body: Vec<u8>, content_type: &str) -> Self {
        let etag = compute_etag(&body);
        let size = body.len() as u64;
        Self {
            meta: ObjectMeta {
                bucket: bucket.into(),
                key: key.into(),
                etag,
                size,
                content_type: content_type.into(),
                last_modified: Utc::now(),
                headers: BTreeMap::new(),
                version_id: None,
                immutable_until: None,
            },
            body,
        }
    }
}

/// S3-compatible ETag for a single-part upload: hex-encoded MD5 of the body.
/// We substitute SHA-256 truncated to 16 bytes (hex) so we don't pull MD5.
/// Multipart ETags follow the AWS convention `"{hex}-{partcount}"`.
pub fn compute_etag(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    let digest = h.finalize();
    let mut hex_str = String::with_capacity(32);
    for b in &digest[..16] {
        hex_str.push_str(&format!("{:02x}", b));
    }
    hex_str
}

pub fn compute_multipart_etag(part_etags: &[String]) -> String {
    let mut buf = Vec::new();
    for tag in part_etags {
        // strip optional surrounding quotes
        let cleaned = tag.trim_matches('"');
        if cleaned.len() % 2 != 0 {
            // shouldn't happen with hex etags
            buf.extend_from_slice(cleaned.as_bytes());
        } else {
            for chunk in cleaned.as_bytes().chunks(2) {
                let s = std::str::from_utf8(chunk).unwrap_or("00");
                let b = u8::from_str_radix(s, 16).unwrap_or(0);
                buf.push(b);
            }
        }
    }
    let mut h = Sha256::new();
    h.update(&buf);
    let digest = h.finalize();
    let mut hex_str = String::with_capacity(32);
    for b in &digest[..16] {
        hex_str.push_str(&format!("{:02x}", b));
    }
    format!("{}-{}", hex_str, part_etags.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_deterministic() {
        let a = compute_etag(b"hello");
        let b = compute_etag(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn etag_distinguishes_inputs() {
        assert_ne!(compute_etag(b"a"), compute_etag(b"b"));
    }

    #[test]
    fn etag_is_hex() {
        let e = compute_etag(b"hello");
        assert_eq!(e.len(), 32);
        for c in e.chars() {
            assert!(c.is_ascii_hexdigit());
        }
    }

    #[test]
    fn etag_empty_body() {
        let e = compute_etag(b"");
        assert_eq!(e.len(), 32);
    }

    #[test]
    fn multipart_etag_includes_part_count() {
        let parts = vec![compute_etag(b"a"), compute_etag(b"b"), compute_etag(b"c")];
        let mp = compute_multipart_etag(&parts);
        assert!(mp.ends_with("-3"));
    }

    #[test]
    fn multipart_etag_deterministic() {
        let parts = vec![compute_etag(b"x"), compute_etag(b"y")];
        let a = compute_multipart_etag(&parts);
        let b = compute_multipart_etag(&parts);
        assert_eq!(a, b);
    }

    #[test]
    fn multipart_etag_different_part_order_differs() {
        let parts1 = vec![compute_etag(b"x"), compute_etag(b"y")];
        let parts2 = vec![compute_etag(b"y"), compute_etag(b"x")];
        assert_ne!(
            compute_multipart_etag(&parts1),
            compute_multipart_etag(&parts2)
        );
    }

    #[test]
    fn object_new_sets_size_and_etag() {
        let o = Object::new("b", "k", b"hello".to_vec(), "text/plain");
        assert_eq!(o.meta.size, 5);
        assert_eq!(o.meta.bucket, "b");
        assert_eq!(o.meta.key, "k");
        assert_eq!(o.meta.etag.len(), 32);
    }
}
