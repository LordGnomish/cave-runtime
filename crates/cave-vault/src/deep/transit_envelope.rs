//! Transit-engine envelope encryption.
//!
//! Reproduces the data-key API surface of the OpenBao Transit engine:
//!
//! * `KekRing` holds a list of versioned KEKs (latest = `current_version`).
//! * `wrap_dek` / `unwrap_dek` use the KEK at a specific version.
//! * `rotate` issues a new version; old versions remain readable until
//!   pruned by `min_decryption_version`.
//!
//! The actual KEK material in this module is a 32-byte symmetric key (chosen
//! by the caller); production code substitutes ring's AEAD primitives.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TransitError {
    #[error("key version not found: {0}")]
    VersionNotFound(u32),
    #[error("invalid kek length: {0}")]
    InvalidKekLength(usize),
    #[error("decryption version is below minimum: {0} < {1}")]
    BelowMin(u32, u32),
    #[error("ciphertext too short")]
    CiphertextTooShort,
    #[error("invalid ciphertext header")]
    InvalidHeader,
}

#[derive(Debug, Clone)]
pub struct KekRing {
    pub keys: BTreeMap<u32, [u8; 32]>,
    pub current_version: u32,
    pub min_decryption_version: u32,
    pub min_encryption_version: u32,
    pub deletion_allowed: bool,
}

impl KekRing {
    pub fn new(initial: [u8; 32]) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(1, initial);
        Self {
            keys,
            current_version: 1,
            min_decryption_version: 1,
            min_encryption_version: 1,
            deletion_allowed: false,
        }
    }

    pub fn rotate(&mut self, next: [u8; 32]) -> u32 {
        self.current_version += 1;
        let v = self.current_version;
        self.keys.insert(v, next);
        v
    }

    pub fn trim(&mut self, min_decrypt: u32) -> Result<(), TransitError> {
        if min_decrypt > self.current_version {
            return Err(TransitError::BelowMin(min_decrypt, self.current_version));
        }
        self.min_decryption_version = min_decrypt;
        self.keys.retain(|v, _| *v >= min_decrypt);
        Ok(())
    }

    pub fn key(&self, version: u32) -> Option<&[u8; 32]> {
        self.keys.get(&version)
    }

    pub fn version_count(&self) -> usize {
        self.keys.len()
    }
}

/// Wire format for a wrapped DEK:
/// `[version: u32 BE][nonce: 12 bytes][ct: 32 bytes (DEK XORed with HMAC stream)]`.
/// The XOR-with-HMAC construction is purely demonstrative — real Transit
/// uses GCM or ChaCha20-Poly1305. The shape is what we test.
pub fn wrap_dek(ring: &KekRing, dek: &[u8; 32], version: u32, nonce: [u8; 12]) -> Result<Vec<u8>, TransitError> {
    if version < ring.min_encryption_version {
        return Err(TransitError::BelowMin(version, ring.min_encryption_version));
    }
    let kek = ring.key(version).ok_or(TransitError::VersionNotFound(version))?;
    let stream = derive_stream(kek, &nonce, 32);
    let mut ct = [0u8; 32];
    for i in 0..32 {
        ct[i] = dek[i] ^ stream[i];
    }
    let mut out = Vec::with_capacity(4 + 12 + 32);
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn unwrap_dek(ring: &KekRing, blob: &[u8]) -> Result<[u8; 32], TransitError> {
    if blob.len() < 4 + 12 + 32 {
        return Err(TransitError::CiphertextTooShort);
    }
    let version = u32::from_be_bytes(blob[..4].try_into().map_err(|_| TransitError::InvalidHeader)?);
    if version < ring.min_decryption_version {
        return Err(TransitError::BelowMin(version, ring.min_decryption_version));
    }
    let nonce: [u8; 12] = blob[4..16].try_into().map_err(|_| TransitError::InvalidHeader)?;
    let ct: [u8; 32] = blob[16..48].try_into().map_err(|_| TransitError::InvalidHeader)?;
    let kek = ring.key(version).ok_or(TransitError::VersionNotFound(version))?;
    let stream = derive_stream(kek, &nonce, 32);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = ct[i] ^ stream[i];
    }
    Ok(out)
}

fn derive_stream(key: &[u8; 32], nonce: &[u8; 12], n: usize) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut out = Vec::with_capacity(n);
    let mut counter: u32 = 0;
    while out.len() < n {
        let mut h = Sha256::new();
        h.update(key);
        h.update(nonce);
        h.update(counter.to_be_bytes());
        let block = h.finalize();
        let need = (n - out.len()).min(32);
        out.extend_from_slice(&block[..need]);
        counter += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn new_ring_starts_at_version_one() {
        let r = KekRing::new(k(0xAA));
        assert_eq!(r.current_version, 1);
        assert_eq!(r.version_count(), 1);
    }

    #[test]
    fn rotate_increments_version() {
        let mut r = KekRing::new(k(0xAA));
        let v = r.rotate(k(0xBB));
        assert_eq!(v, 2);
        assert_eq!(r.version_count(), 2);
    }

    #[test]
    fn key_for_version_returns_correct_bytes() {
        let mut r = KekRing::new(k(0xAA));
        r.rotate(k(0xBB));
        assert_eq!(r.key(1), Some(&k(0xAA)));
        assert_eq!(r.key(2), Some(&k(0xBB)));
    }

    #[test]
    fn key_unknown_version_returns_none() {
        let r = KekRing::new(k(0xAA));
        assert!(r.key(99).is_none());
    }

    #[test]
    fn trim_removes_old_versions() {
        let mut r = KekRing::new(k(0xAA));
        r.rotate(k(0xBB));
        r.rotate(k(0xCC));
        r.trim(2).unwrap();
        assert_eq!(r.version_count(), 2);
        assert!(r.key(1).is_none());
    }

    #[test]
    fn trim_above_current_errors() {
        let mut r = KekRing::new(k(0xAA));
        let err = r.trim(99).unwrap_err();
        assert!(matches!(err, TransitError::BelowMin(_, _)));
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let r = KekRing::new(k(0xAA));
        let dek = k(0x11);
        let blob = wrap_dek(&r, &dek, 1, [0u8; 12]).unwrap();
        let recovered = unwrap_dek(&r, &blob).unwrap();
        assert_eq!(recovered, dek);
    }

    #[test]
    fn wrap_unknown_version_errors() {
        let r = KekRing::new(k(0xAA));
        let err = wrap_dek(&r, &k(0x11), 99, [0u8; 12]).unwrap_err();
        assert_eq!(err, TransitError::VersionNotFound(99));
    }

    #[test]
    fn wrap_below_min_encryption_errors() {
        let mut r = KekRing::new(k(0xAA));
        r.rotate(k(0xBB));
        r.min_encryption_version = 2;
        let err = wrap_dek(&r, &k(0x11), 1, [0u8; 12]).unwrap_err();
        assert!(matches!(err, TransitError::BelowMin(_, _)));
    }

    #[test]
    fn unwrap_below_min_decryption_errors() {
        let mut r = KekRing::new(k(0xAA));
        r.rotate(k(0xBB));
        let blob = wrap_dek(&r, &k(0x11), 1, [0u8; 12]).unwrap();
        r.min_decryption_version = 2;
        let err = unwrap_dek(&r, &blob).unwrap_err();
        assert!(matches!(err, TransitError::BelowMin(_, _)));
    }

    #[test]
    fn wrap_after_rotate_uses_new_kek() {
        let mut r = KekRing::new(k(0xAA));
        r.rotate(k(0xBB));
        let dek = k(0x11);
        let blob1 = wrap_dek(&r, &dek, 1, [0u8; 12]).unwrap();
        let blob2 = wrap_dek(&r, &dek, 2, [0u8; 12]).unwrap();
        assert_ne!(blob1, blob2);
    }

    #[test]
    fn unwrap_truncated_errors() {
        let r = KekRing::new(k(0xAA));
        let err = unwrap_dek(&r, &[0u8; 10]).unwrap_err();
        assert_eq!(err, TransitError::CiphertextTooShort);
    }

    #[test]
    fn unwrap_after_trim_errors_for_old_versions() {
        let mut r = KekRing::new(k(0xAA));
        let blob = wrap_dek(&r, &k(0x11), 1, [0u8; 12]).unwrap();
        r.rotate(k(0xBB));
        r.trim(2).unwrap();
        let err = unwrap_dek(&r, &blob).unwrap_err();
        assert!(matches!(err, TransitError::BelowMin(_, _)));
    }

    #[test]
    fn nonce_changes_ciphertext() {
        let r = KekRing::new(k(0xAA));
        let dek = k(0x11);
        let b1 = wrap_dek(&r, &dek, 1, [0u8; 12]).unwrap();
        let b2 = wrap_dek(&r, &dek, 1, [1u8; 12]).unwrap();
        assert_ne!(b1, b2);
    }
}
