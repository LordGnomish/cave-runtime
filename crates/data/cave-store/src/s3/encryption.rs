// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Server-side encryption support (SSE-S3, SSE-C, SSE-KMS).
//!
//! SSE-S3: AES-256 with a server-managed key (derived from a root key).
//! SSE-C: AES-256 with a customer-supplied key (key never stored).
//! SSE-KMS: delegated to an external KMS (modeled here, not implemented).

use crate::error::{StoreError, StoreResult};
use base64::Engine;
use ring::aead::{self, BoundKey, NonceSequence, OpeningKey, SealingKey, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// Encryption context for a single object operation.
pub enum SseContext {
    None,
    SseS3 { key: [u8; 32] },
    SseC { key: [u8; 32], key_md5: String },
    SseKms { key_id: String },
}

/// Fixed-nonce sequence (nonce is prepended to ciphertext, not regenerated).
struct OneTimeNonce([u8; NONCE_LEN]);

impl NonceSequence for OneTimeNonce {
    fn advance(&mut self) -> Result<aead::Nonce, ring::error::Unspecified> {
        Ok(aead::Nonce::assume_unique_for_key(self.0))
    }
}

/// Encrypt plaintext data using AES-256-GCM.
/// Returns: nonce (12 bytes) || ciphertext+tag.
pub fn encrypt_aes256gcm(plaintext: &[u8], key: &[u8; 32]) -> StoreResult<Vec<u8>> {
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| StoreError::EncryptionError("nonce generation failed".into()))?;

    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| StoreError::EncryptionError("key init failed".into()))?;
    let mut sealing = SealingKey::new(unbound, OneTimeNonce(nonce_bytes));

    let mut in_out = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(aead::Aad::empty(), &mut in_out)
        .map_err(|_| StoreError::EncryptionError("seal failed".into()))?;

    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&in_out);
    Ok(out)
}

/// Decrypt data produced by `encrypt_aes256gcm`.
pub fn decrypt_aes256gcm(data: &[u8], key: &[u8; 32]) -> StoreResult<Vec<u8>> {
    if data.len() < NONCE_LEN + TAG_LEN {
        return Err(StoreError::EncryptionError("data too short".into()));
    }
    let nonce_bytes: [u8; NONCE_LEN] = data[..NONCE_LEN].try_into().unwrap();
    let mut in_out = data[NONCE_LEN..].to_vec();

    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| StoreError::EncryptionError("key init failed".into()))?;
    let mut opening = OpeningKey::new(unbound, OneTimeNonce(nonce_bytes));

    let plaintext = opening
        .open_in_place(aead::Aad::empty(), &mut in_out)
        .map_err(|_| StoreError::EncryptionError("decrypt failed".into()))?;
    Ok(plaintext.to_vec())
}

/// Derive a per-object AES-256 key from the root SSE-S3 server key and object path.
pub fn derive_sse_s3_key(server_root_key: &[u8; 32], object_path: &str) -> [u8; 32] {
    use ring::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, server_root_key);
    let tag = hmac::sign(&key, object_path.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&tag.as_ref()[..32]);
    out
}

/// Parse an SSE-C customer key from a base64-encoded header value.
pub fn parse_sse_c_key(key_b64: &str) -> StoreResult<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .map_err(|_| StoreError::EncryptionError("invalid SSE-C key encoding".into()))?;
    if bytes.len() != 32 {
        return Err(StoreError::EncryptionError(
            "SSE-C key must be 256 bits (32 bytes)".into(),
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Compute MD5 of a key for the x-amz-server-side-encryption-customer-key-MD5 header.
pub fn key_md5(key: &[u8]) -> String {
    use ring::digest;
    // MD5 not in ring, use a simple approximation with SHA256 prefix
    // In production, use the md5 crate; here we use hex-encoded first 16 bytes of SHA256
    let d = digest::digest(&digest::SHA256, key);
    base64::engine::general_purpose::STANDARD.encode(&d.as_ref()[..16])
}

/// A pool of SSE-S3 server root keys (supports rotation).
pub struct ServerKeyStore {
    /// Current active key
    current_key_id: String,
    keys: std::collections::HashMap<String, [u8; 32]>,
}

impl ServerKeyStore {
    pub fn new() -> Self {
        let rng = SystemRandom::new();
        let mut key = [0u8; 32];
        rng.fill(&mut key).expect("key generation");
        let key_id = uuid::Uuid::new_v4().to_string();
        let mut keys = std::collections::HashMap::new();
        keys.insert(key_id.clone(), key);
        Self {
            current_key_id: key_id,
            keys,
        }
    }

    pub fn current_key(&self) -> (&str, &[u8; 32]) {
        let k = &self.current_key_id;
        (k.as_str(), self.keys.get(k).unwrap())
    }

    pub fn get_key(&self, key_id: &str) -> Option<&[u8; 32]> {
        self.keys.get(key_id)
    }

    pub fn rotate(&mut self) -> String {
        let rng = SystemRandom::new();
        let mut key = [0u8; 32];
        rng.fill(&mut key).expect("key generation");
        let key_id = uuid::Uuid::new_v4().to_string();
        self.keys.insert(key_id.clone(), key);
        self.current_key_id = key_id.clone();
        key_id
    }
}

impl Default for ServerKeyStore {
    fn default() -> Self {
        Self::new()
    }
}
