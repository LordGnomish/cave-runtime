// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transit secrets engine — encryption as a service.
//!
//! Provides AES-256-GCM encryption/decryption and Ed25519 sign/verify
//! without exposing key material. Ciphertext format: `vault:v{N}:{base64}`.

use crate::models::{TransitKey, TransitKeyType};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::Utc;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransitError {
    #[error("Key not found: {0}")]
    KeyNotFound(String),
    #[error("Invalid ciphertext format")]
    InvalidCiphertext,
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed")]
    DecryptionFailed,
    #[error("Key version not found: {0}")]
    VersionNotFound(u32),
    #[error("Key type does not support this operation")]
    UnsupportedOperation,
}

/// Raw key material for one version
#[derive(Debug, Clone)]
pub enum TransitKeyMaterial {
    /// AES-256 key bytes (32 bytes)
    Aes256Gcm(Vec<u8>),
    /// Ed25519 PKCS8 DER key pair
    Ed25519(Vec<u8>),
}

/// All versions of a named transit key
#[derive(Debug, Clone)]
pub struct TransitKeyEntry {
    pub meta: TransitKey,
    pub versions: HashMap<u32, TransitKeyMaterial>,
}

/// Create and register a new transit key.
pub fn create_key(
    store: &mut HashMap<String, TransitKeyEntry>,
    name: &str,
    key_type: TransitKeyType,
) -> Result<TransitKey, TransitError> {
    let rng = SystemRandom::new();
    let now = Utc::now();

    let material = generate_material(&rng, &key_type)?;

    let supports_enc = matches!(
        key_type,
        TransitKeyType::Aes256Gcm96 | TransitKeyType::ChaCha20Poly1305
    );
    let supports_sign = matches!(
        key_type,
        TransitKeyType::Ed25519 | TransitKeyType::EcdsaP256 | TransitKeyType::Rsa2048
    );

    let meta = TransitKey {
        name: name.to_string(),
        key_type,
        latest_version: 1,
        min_decryption_version: 1,
        min_encryption_version: 0,
        supports_encryption: supports_enc,
        supports_decryption: supports_enc,
        supports_signing: supports_sign,
        supports_derivation: false,
        deletion_allowed: false,
        exportable: false,
        allow_plaintext_backup: false,
        created_at: now,
        updated_at: now,
    };

    let mut versions = HashMap::new();
    versions.insert(1u32, material);
    store.insert(name.to_string(), TransitKeyEntry { meta: meta.clone(), versions });
    Ok(meta)
}

/// Encrypt plaintext with the latest key version.
/// Returns `vault:v{N}:{base64(nonce || ciphertext || tag)}`.
pub fn encrypt(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    plaintext: &[u8],
    context: Option<&[u8]>,
) -> Result<String, TransitError> {
    let entry = store
        .get(key_name)
        .ok_or_else(|| TransitError::KeyNotFound(key_name.to_string()))?;
    let ver = entry.meta.latest_version;
    let material = entry.versions.get(&ver).ok_or(TransitError::VersionNotFound(ver))?;

    match material {
        TransitKeyMaterial::Aes256Gcm(key_bytes) => {
            aes_encrypt(key_bytes, ver, plaintext, context)
        }
        TransitKeyMaterial::Ed25519(_) => Err(TransitError::UnsupportedOperation),
    }
}

/// Decrypt a `vault:v{N}:{base64}` ciphertext.
pub fn decrypt(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    ciphertext_str: &str,
    context: Option<&[u8]>,
) -> Result<Vec<u8>, TransitError> {
    let entry = store
        .get(key_name)
        .ok_or_else(|| TransitError::KeyNotFound(key_name.to_string()))?;

    let (ver, encoded) = parse_vault_ciphertext(ciphertext_str)?;

    if ver < entry.meta.min_decryption_version {
        return Err(TransitError::VersionNotFound(ver));
    }
    let material = entry.versions.get(&ver).ok_or(TransitError::VersionNotFound(ver))?;

    match material {
        TransitKeyMaterial::Aes256Gcm(key_bytes) => {
            aes_decrypt(key_bytes, &encoded, context)
        }
        TransitKeyMaterial::Ed25519(_) => Err(TransitError::UnsupportedOperation),
    }
}

/// Sign data with an Ed25519 key. Returns `vault:v{N}:{base64(signature)}`.
pub fn sign(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    data: &[u8],
) -> Result<String, TransitError> {
    let entry = store
        .get(key_name)
        .ok_or_else(|| TransitError::KeyNotFound(key_name.to_string()))?;
    let ver = entry.meta.latest_version;
    let material = entry.versions.get(&ver).ok_or(TransitError::VersionNotFound(ver))?;

    match material {
        TransitKeyMaterial::Ed25519(pkcs8) => {
            let kp = Ed25519KeyPair::from_pkcs8(pkcs8)
                .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;
            let sig = kp.sign(data);
            Ok(format!("vault:v{ver}:{}", B64.encode(sig.as_ref())))
        }
        TransitKeyMaterial::Aes256Gcm(_) => Err(TransitError::UnsupportedOperation),
    }
}

/// Verify an Ed25519 signature produced by `sign`.
pub fn verify(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    data: &[u8],
    signature_str: &str,
) -> Result<bool, TransitError> {
    let entry = store
        .get(key_name)
        .ok_or_else(|| TransitError::KeyNotFound(key_name.to_string()))?;

    let (ver, sig_bytes_encoded) = parse_vault_ciphertext(signature_str)?;
    let sig_bytes = sig_bytes_encoded;

    let material = entry.versions.get(&ver).ok_or(TransitError::VersionNotFound(ver))?;

    match material {
        TransitKeyMaterial::Ed25519(pkcs8) => {
            let kp = Ed25519KeyPair::from_pkcs8(pkcs8)
                .map_err(|_| TransitError::DecryptionFailed)?;
            let pub_key_bytes = kp.public_key().as_ref().to_vec();
            let pub_key = UnparsedPublicKey::new(&ED25519, &pub_key_bytes);
            Ok(pub_key.verify(data, &sig_bytes).is_ok())
        }
        TransitKeyMaterial::Aes256Gcm(_) => Err(TransitError::UnsupportedOperation),
    }
}

/// Generate a data key: returns `(plaintext_key_bytes, wrapped_ciphertext)`.
/// The plaintext is used once for encryption; the ciphertext is stored.
pub fn generate_data_key(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    bits: u32,
) -> Result<(Vec<u8>, String), TransitError> {
    let rng = SystemRandom::new();
    let key_len = (bits / 8) as usize;
    let mut plaintext = vec![0u8; key_len];
    rng.fill(&mut plaintext)
        .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;
    let ciphertext = encrypt(store, key_name, &plaintext, None)?;
    Ok((plaintext, ciphertext))
}

/// Rotate a key: add a new version, bump latest_version.
pub fn rotate_key(
    store: &mut HashMap<String, TransitKeyEntry>,
    key_name: &str,
) -> Result<TransitKey, TransitError> {
    let rng = SystemRandom::new();
    let entry = store
        .get_mut(key_name)
        .ok_or_else(|| TransitError::KeyNotFound(key_name.to_string()))?;

    let new_ver = entry.meta.latest_version + 1;
    let material = generate_material(&rng, &entry.meta.key_type)?;
    entry.versions.insert(new_ver, material);
    entry.meta.latest_version = new_ver;
    entry.meta.updated_at = Utc::now();
    Ok(entry.meta.clone())
}

/// Re-encrypt old ciphertext with the current key version.
pub fn rewrap(
    store: &HashMap<String, TransitKeyEntry>,
    key_name: &str,
    old_ciphertext: &str,
) -> Result<String, TransitError> {
    let plaintext = decrypt(store, key_name, old_ciphertext, None)?;
    encrypt(store, key_name, &plaintext, None)
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn generate_material(
    rng: &SystemRandom,
    key_type: &TransitKeyType,
) -> Result<TransitKeyMaterial, TransitError> {
    match key_type {
        TransitKeyType::Aes256Gcm96
        | TransitKeyType::ChaCha20Poly1305
        | TransitKeyType::EcdsaP256
        | TransitKeyType::Rsa2048 => {
            let mut bytes = vec![0u8; 32];
            rng.fill(&mut bytes)
                .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;
            Ok(TransitKeyMaterial::Aes256Gcm(bytes))
        }
        TransitKeyType::Ed25519 => {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(rng)
                .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;
            Ok(TransitKeyMaterial::Ed25519(pkcs8.as_ref().to_vec()))
        }
    }
}

fn aes_encrypt(
    key_bytes: &[u8],
    version: u32,
    plaintext: &[u8],
    context: Option<&[u8]>,
) -> Result<String, TransitError> {
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes)
        .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|e| TransitError::EncryptionFailed(e.to_string()))?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let aad = Aad::from(context.unwrap_or(&[]));

    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, aad, &mut buf)
        .map_err(|_| TransitError::EncryptionFailed("seal failed".into()))?;

    let mut output = nonce_bytes.to_vec();
    output.extend_from_slice(&buf);

    Ok(format!("vault:v{version}:{}", B64.encode(&output)))
}

fn aes_decrypt(
    key_bytes: &[u8],
    encoded: &[u8],
    context: Option<&[u8]>,
) -> Result<Vec<u8>, TransitError> {
    if encoded.len() < 12 {
        return Err(TransitError::InvalidCiphertext);
    }
    let nonce_bytes: [u8; 12] = encoded[..12]
        .try_into()
        .map_err(|_| TransitError::InvalidCiphertext)?;
    let mut buf = encoded[12..].to_vec();

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|_| TransitError::DecryptionFailed)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let aad = Aad::from(context.unwrap_or(&[]));

    let plaintext = key
        .open_in_place(nonce, aad, &mut buf)
        .map_err(|_| TransitError::DecryptionFailed)?;
    Ok(plaintext.to_vec())
}

/// Parse `vault:v{N}:{base64}` — returns `(version, decoded_bytes)`.
fn parse_vault_ciphertext(s: &str) -> Result<(u32, Vec<u8>), TransitError> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() != 3 || parts[0] != "vault" {
        return Err(TransitError::InvalidCiphertext);
    }
    let ver: u32 = parts[1]
        .strip_prefix('v')
        .and_then(|v| v.parse().ok())
        .ok_or(TransitError::InvalidCiphertext)?;
    let decoded = B64.decode(parts[2]).map_err(|_| TransitError::InvalidCiphertext)?;
    Ok((ver, decoded))
}
