//! Multipart upload state machine — works for both S3 (uploadId / partNumber)
//! and Azure Block Blob (blockId).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::error::{BlobError, BlobResult};
use crate::object::{Object, compute_etag, compute_multipart_etag};

pub const MIN_PART_NUMBER: i32 = 1;
pub const MAX_PART_NUMBER: i32 = 10_000;
pub const MIN_PART_SIZE: usize = 5 * 1024 * 1024; // 5 MiB (S3 default; relaxed in tests)
pub const MAX_PART_SIZE: usize = 5 * 1024 * 1024 * 1024; // 5 GiB

#[derive(Debug, Clone)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub initiated_at: DateTime<Utc>,
    pub parts: BTreeMap<i32, UploadPart>,
}

#[derive(Debug, Clone)]
pub struct UploadPart {
    pub number: i32,
    pub etag: String,
    pub size: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct UploadRegistry {
    uploads: Mutex<BTreeMap<String, MultipartUpload>>,
    counter: Mutex<u64>,
}

impl UploadRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn initiate(&self, bucket: &str, key: &str) -> String {
        let id = {
            let mut c = self.counter.lock();
            *c += 1;
            format!("upload-{}", *c)
        };
        let mut uploads = self.uploads.lock();
        uploads.insert(
            id.clone(),
            MultipartUpload {
                upload_id: id.clone(),
                bucket: bucket.into(),
                key: key.into(),
                initiated_at: Utc::now(),
                parts: BTreeMap::new(),
            },
        );
        id
    }

    pub fn upload_part(
        &self,
        upload_id: &str,
        part_number: i32,
        data: Vec<u8>,
    ) -> BlobResult<String> {
        if !(MIN_PART_NUMBER..=MAX_PART_NUMBER).contains(&part_number) {
            return Err(BlobError::InvalidPartNumber(part_number));
        }
        let mut uploads = self.uploads.lock();
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| BlobError::UploadNotFound(upload_id.into()))?;
        let etag = compute_etag(&data);
        let size = data.len() as u64;
        upload.parts.insert(
            part_number,
            UploadPart {
                number: part_number,
                etag: etag.clone(),
                size,
                data,
            },
        );
        Ok(etag)
    }

    pub fn list_parts(&self, upload_id: &str) -> BlobResult<Vec<UploadPart>> {
        let uploads = self.uploads.lock();
        let upload = uploads
            .get(upload_id)
            .ok_or_else(|| BlobError::UploadNotFound(upload_id.into()))?;
        Ok(upload.parts.values().cloned().collect())
    }

    /// Combine the listed parts into a single object. The list must declare
    /// the part numbers and their ETags (S3 `CompleteMultipartUpload` rules).
    pub fn complete(
        &self,
        upload_id: &str,
        wanted: &[(i32, String)],
        content_type: &str,
    ) -> BlobResult<Object> {
        let mut uploads = self.uploads.lock();
        let upload = uploads
            .remove(upload_id)
            .ok_or_else(|| BlobError::UploadNotFound(upload_id.into()))?;
        let mut combined = Vec::new();
        let mut etags = Vec::with_capacity(wanted.len());
        for (n, expected_etag) in wanted {
            let part = upload
                .parts
                .get(n)
                .ok_or(BlobError::MissingPart(*n))?;
            if &part.etag != expected_etag {
                // restore on failure
                uploads.insert(upload_id.into(), upload.clone());
                return Err(BlobError::EtagMismatch(
                    *n,
                    part.etag.clone(),
                    expected_etag.clone(),
                ));
            }
            combined.extend_from_slice(&part.data);
            etags.push(part.etag.clone());
        }
        let mut obj = Object::new(&upload.bucket, &upload.key, combined, content_type);
        obj.meta.etag = compute_multipart_etag(&etags);
        Ok(obj)
    }

    pub fn abort(&self, upload_id: &str) -> BlobResult<()> {
        let mut uploads = self.uploads.lock();
        if uploads.remove(upload_id).is_none() {
            return Err(BlobError::UploadNotFound(upload_id.into()));
        }
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        self.uploads.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initiate_returns_unique_ids() {
        let r = UploadRegistry::new();
        let id1 = r.initiate("b", "k");
        let id2 = r.initiate("b", "k");
        assert_ne!(id1, id2);
        assert_eq!(r.pending_count(), 2);
    }

    #[test]
    fn upload_part_returns_etag() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        let etag = r.upload_part(&id, 1, b"hello".to_vec()).unwrap();
        assert_eq!(etag.len(), 32);
    }

    #[test]
    fn upload_part_invalid_number_errors() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        assert!(r.upload_part(&id, 0, vec![]).is_err());
        assert!(r.upload_part(&id, 10_001, vec![]).is_err());
    }

    #[test]
    fn upload_part_unknown_upload_errors() {
        let r = UploadRegistry::new();
        let err = r.upload_part("missing", 1, vec![]).unwrap_err();
        match err {
            BlobError::UploadNotFound(_) => {}
            other => panic!("expected UploadNotFound, got {:?}", other),
        }
    }

    #[test]
    fn list_parts_returns_in_order() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        r.upload_part(&id, 3, b"c".to_vec()).unwrap();
        r.upload_part(&id, 1, b"a".to_vec()).unwrap();
        r.upload_part(&id, 2, b"b".to_vec()).unwrap();
        let parts = r.list_parts(&id).unwrap();
        let nums: Vec<i32> = parts.iter().map(|p| p.number).collect();
        assert_eq!(nums, vec![1, 2, 3]);
    }

    #[test]
    fn complete_assembles_object_in_part_order() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        let e1 = r.upload_part(&id, 1, b"hel".to_vec()).unwrap();
        let e2 = r.upload_part(&id, 2, b"lo!".to_vec()).unwrap();
        let obj = r
            .complete(&id, &[(1, e1.clone()), (2, e2.clone())], "text/plain")
            .unwrap();
        assert_eq!(obj.body, b"hello!");
        assert!(obj.meta.etag.ends_with("-2"));
        assert_eq!(r.pending_count(), 0);
    }

    #[test]
    fn complete_missing_part_errors_and_keeps_upload() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        let _ = r.upload_part(&id, 1, b"a".to_vec()).unwrap();
        let err = r
            .complete(&id, &[(99, "deadbeef".into())], "t")
            .unwrap_err();
        assert_eq!(err, BlobError::MissingPart(99));
    }

    #[test]
    fn complete_etag_mismatch_errors() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        let _ = r.upload_part(&id, 1, b"x".to_vec()).unwrap();
        let err = r
            .complete(&id, &[(1, "wrong".into())], "t")
            .unwrap_err();
        match err {
            BlobError::EtagMismatch(n, _, e) => {
                assert_eq!(n, 1);
                assert_eq!(e, "wrong");
            }
            other => panic!("expected EtagMismatch, got {:?}", other),
        }
        // Upload must still exist so the client can retry.
        assert_eq!(r.pending_count(), 1);
    }

    #[test]
    fn abort_removes_upload() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        r.upload_part(&id, 1, b"x".to_vec()).unwrap();
        r.abort(&id).unwrap();
        assert_eq!(r.pending_count(), 0);
        assert!(r.list_parts(&id).is_err());
    }

    #[test]
    fn abort_unknown_errors() {
        let r = UploadRegistry::new();
        let err = r.abort("nope").unwrap_err();
        assert!(matches!(err, BlobError::UploadNotFound(_)));
    }

    #[test]
    fn part_number_constants_match_s3_spec() {
        assert_eq!(MIN_PART_NUMBER, 1);
        assert_eq!(MAX_PART_NUMBER, 10_000);
    }

    #[test]
    fn upload_replaces_part_etag_on_re_upload() {
        let r = UploadRegistry::new();
        let id = r.initiate("b", "k");
        let _ = r.upload_part(&id, 1, b"a".to_vec()).unwrap();
        let new_etag = r.upload_part(&id, 1, b"b".to_vec()).unwrap();
        let parts = r.list_parts(&id).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].etag, new_etag);
    }
}
