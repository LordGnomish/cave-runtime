//! KMS envelope encryption — DEK + KEK + key ring rotation.
//!
//! Mirrors the AWS KMS / MinIO KES integration model: each object is
//! encrypted with a per-object DEK (32-byte AES key), and the DEK is wrapped
//! by the bucket-default KEK (referenced by `kms_key_id`).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum KmsError {
    #[error("kek not found: {0}")]
    KekNotFound(String),
    #[error("invalid dek length: {0}")]
    InvalidDekLength(usize),
    #[error("invalid wrapped blob")]
    InvalidWrappedBlob,
    #[error("kek version disabled: {0}/v{1}")]
    VersionDisabled(String, u32),
}

#[derive(Debug, Clone)]
pub struct Kek {
    pub key_id: String,
    pub versions: BTreeMap<u32, [u8; 32]>,
    pub current_version: u32,
    pub min_decrypt_version: u32,
}

impl Kek {
    pub fn new(key_id: &str, initial: [u8; 32]) -> Self {
        let mut versions = BTreeMap::new();
        versions.insert(1, initial);
        Self {
            key_id: key_id.to_string(),
            versions,
            current_version: 1,
            min_decrypt_version: 1,
        }
    }

    pub fn rotate(&mut self, next: [u8; 32]) -> u32 {
        self.current_version += 1;
        let v = self.current_version;
        self.versions.insert(v, next);
        v
    }

    pub fn disable_version(&mut self, v: u32) {
        if v >= self.min_decrypt_version {
            self.min_decrypt_version = v + 1;
        }
        self.versions.remove(&v);
    }
}

#[derive(Debug, Default)]
pub struct KmsRing {
    keks: BTreeMap<String, Kek>,
}

impl KmsRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_kek(&mut self, kek: Kek) {
        self.keks.insert(kek.key_id.clone(), kek);
    }

    pub fn get_kek(&self, key_id: &str) -> Option<&Kek> {
        self.keks.get(key_id)
    }

    pub fn rotate(&mut self, key_id: &str, next: [u8; 32]) -> Result<u32, KmsError> {
        let kek = self
            .keks
            .get_mut(key_id)
            .ok_or_else(|| KmsError::KekNotFound(key_id.into()))?;
        Ok(kek.rotate(next))
    }

    /// Wrap a 32-byte DEK with the KEK's current version.
    /// Wire format: `[ver: u32 BE][kek_id_len: u16][kek_id...][wrapped: 32 bytes]`.
    pub fn wrap_dek(&self, key_id: &str, dek: &[u8; 32]) -> Result<Vec<u8>, KmsError> {
        let kek = self
            .keks
            .get(key_id)
            .ok_or_else(|| KmsError::KekNotFound(key_id.into()))?;
        let v = kek.current_version;
        let kek_bytes = kek
            .versions
            .get(&v)
            .ok_or(KmsError::VersionDisabled(key_id.into(), v))?;
        let wrapped = xor_with_keystream(dek, kek_bytes);
        let mut out = Vec::new();
        out.extend_from_slice(&v.to_be_bytes());
        out.extend_from_slice(&(key_id.len() as u16).to_be_bytes());
        out.extend_from_slice(key_id.as_bytes());
        out.extend_from_slice(&wrapped);
        Ok(out)
    }

    pub fn unwrap_dek(&self, blob: &[u8]) -> Result<[u8; 32], KmsError> {
        if blob.len() < 6 + 32 {
            return Err(KmsError::InvalidWrappedBlob);
        }
        let v = u32::from_be_bytes([blob[0], blob[1], blob[2], blob[3]]);
        let id_len = u16::from_be_bytes([blob[4], blob[5]]) as usize;
        if blob.len() < 6 + id_len + 32 {
            return Err(KmsError::InvalidWrappedBlob);
        }
        let key_id =
            std::str::from_utf8(&blob[6..6 + id_len]).map_err(|_| KmsError::InvalidWrappedBlob)?;
        let kek = self
            .keks
            .get(key_id)
            .ok_or_else(|| KmsError::KekNotFound(key_id.into()))?;
        if v < kek.min_decrypt_version {
            return Err(KmsError::VersionDisabled(key_id.into(), v));
        }
        let kek_bytes = kek
            .versions
            .get(&v)
            .ok_or(KmsError::VersionDisabled(key_id.into(), v))?;
        let ct = &blob[6 + id_len..6 + id_len + 32];
        let mut dek = [0u8; 32];
        dek.copy_from_slice(&xor_with_keystream(ct.try_into().unwrap(), kek_bytes));
        Ok(dek)
    }
}

fn xor_with_keystream(input: &[u8; 32], key: &[u8; 32]) -> [u8; 32] {
    // Derive 32 bytes from SHA-256(key || "kms-stream").
    let mut h = Sha256::new();
    h.update(key);
    h.update(b"cave-blob-kms-stream");
    let stream = h.finalize();
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = input[i] ^ stream[i];
    }
    out
}

pub fn random_dek() -> [u8; 32] {
    let mut dek = [0u8; 32];
    use rand::Rng;
    rand::thread_rng().fill(&mut dek[..]);
    dek
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_kek_bytes() -> [u8; 32] {
        [0xAA; 32]
    }

    #[test]
    fn add_then_get_kek() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        assert!(r.get_kek("k1").is_some());
    }

    #[test]
    fn rotate_increments_version() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        let v = r.rotate("k1", [0xBB; 32]).unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        let dek = [0x11; 32];
        let blob = r.wrap_dek("k1", &dek).unwrap();
        let recovered = r.unwrap_dek(&blob).unwrap();
        assert_eq!(recovered, dek);
    }

    #[test]
    fn wrap_unknown_key_errors() {
        let r = KmsRing::new();
        let err = r.wrap_dek("missing", &[0; 32]).unwrap_err();
        assert!(matches!(err, KmsError::KekNotFound(_)));
    }

    #[test]
    fn unwrap_truncated_blob_errors() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        let err = r.unwrap_dek(&[0; 10]).unwrap_err();
        assert_eq!(err, KmsError::InvalidWrappedBlob);
    }

    #[test]
    fn unwrap_unknown_key_errors() {
        let r = KmsRing::new();
        let mut blob = Vec::new();
        blob.extend_from_slice(&1u32.to_be_bytes());
        blob.extend_from_slice(&5u16.to_be_bytes());
        blob.extend_from_slice(b"ghost");
        blob.extend_from_slice(&[0; 32]);
        let err = r.unwrap_dek(&blob).unwrap_err();
        assert!(matches!(err, KmsError::KekNotFound(_)));
    }

    #[test]
    fn unwrap_after_disable_version_errors() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        let dek = [0x11; 32];
        let blob = r.wrap_dek("k1", &dek).unwrap();
        // Rotate then disable the original version.
        r.rotate("k1", [0xBB; 32]).unwrap();
        r.keks.get_mut("k1").unwrap().disable_version(1);
        let err = r.unwrap_dek(&blob).unwrap_err();
        assert!(matches!(err, KmsError::VersionDisabled(_, _)));
    }

    #[test]
    fn random_dek_changes_each_call() {
        let a = random_dek();
        let b = random_dek();
        assert_ne!(a, b);
    }

    #[test]
    fn wrap_after_rotation_uses_new_kek() {
        let mut r = KmsRing::new();
        r.add_kek(Kek::new("k1", fixed_kek_bytes()));
        let dek = [0x11; 32];
        let blob_v1 = r.wrap_dek("k1", &dek).unwrap();
        r.rotate("k1", [0xBB; 32]).unwrap();
        let blob_v2 = r.wrap_dek("k1", &dek).unwrap();
        assert_ne!(blob_v1, blob_v2);
        assert!(blob_v2.starts_with(&2u32.to_be_bytes()));
    }

    #[test]
    fn xor_keystream_deterministic() {
        let a = xor_with_keystream(&[0; 32], &fixed_kek_bytes());
        let b = xor_with_keystream(&[0; 32], &fixed_kek_bytes());
        assert_eq!(a, b);
    }

    #[test]
    fn xor_keystream_changes_with_key() {
        let a = xor_with_keystream(&[0; 32], &[0xAA; 32]);
        let b = xor_with_keystream(&[0; 32], &[0xBB; 32]);
        assert_ne!(a, b);
    }
}
