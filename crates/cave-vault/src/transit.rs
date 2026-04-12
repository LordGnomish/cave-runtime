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
//! Transit Secrets Engine — AES-256-GCM, Ed25519, RSA-2048.
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519},
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    Oaep, RsaPrivateKey,
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use crate::error::VaultError;
// ── Key types ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TransitKeyType {
    Aes256Gcm,
    Ed25519,
    Rsa2048,
/// Raw key material — stored in memory only, never serialised in responses.
#[derive(Clone)]
pub enum KeyMaterial {
    Aes256Gcm(Vec<u8>),  // 32 raw bytes
    Ed25519(Vec<u8>),    // PKCS8 DER
    Rsa2048(Vec<u8>),    // PKCS8 DER
impl KeyMaterial {
    fn generate(kt: &TransitKeyType) -> Result<Self, VaultError> {
        match kt {
            TransitKeyType::Aes256Gcm => {
                let mut key = vec![0u8; 32];
                rng.fill(&mut key)
                    .map_err(|_| VaultError::CryptoError("rng fill".into()))?;
                Ok(KeyMaterial::Aes256Gcm(key))
                let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
                    .map_err(|_| VaultError::CryptoError("ed25519 keygen".into()))?;
                Ok(KeyMaterial::Ed25519(pkcs8.as_ref().to_vec()))
            TransitKeyType::Rsa2048 => {
                let mut rng2 = rand::thread_rng();
                let priv_key = RsaPrivateKey::new(&mut rng2, 2048)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let der = priv_key
                    .to_pkcs8_der()
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(KeyMaterial::Rsa2048(der.as_bytes().to_vec()))
// ── TransitKey (public metadata) ─────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitKey {
    pub name: String,
    pub key_type: TransitKeyType,
    pub latest_version: u32,
    pub min_decryption_version: u32,
    pub deletion_allowed: bool,
    pub exportable: bool,
    pub supports_encryption: bool,
    pub supports_decryption: bool,
    pub supports_signing: bool,
    pub supports_derivation: bool,
// ── TransitKeyEntry (metadata + key material) ────────────────────────────────
    pub versions: HashMap<u32, KeyMaterial>,
impl TransitKeyEntry {
    pub fn create(name: &str, key_type: TransitKeyType) -> Result<Self, VaultError> {
        let material = KeyMaterial::generate(&key_type)?;
        let (enc, dec, sign, deriv) = match key_type {
            TransitKeyType::Aes256Gcm => (true, true, false, true),
            TransitKeyType::Ed25519   => (false, false, true, false),
            TransitKeyType::Rsa2048  => (true, true, true, false),
        versions.insert(1, material);
        Ok(Self {
            meta: TransitKey {
                supports_encryption: enc,
                supports_decryption: dec,
                supports_signing: sign,
                supports_derivation: deriv,
            },
            versions,
        })
    pub fn rotate(&mut self) -> Result<(), VaultError> {
        let next = self.meta.latest_version + 1;
        let material = KeyMaterial::generate(&self.meta.key_type)?;
        self.versions.insert(next, material);
        self.meta.latest_version = next;
        Ok(())
    // ── Encrypt / Decrypt (AES-256-GCM) ──────────────────────────────────────
    pub fn encrypt(&self, plaintext: &[u8], context: Option<&[u8]>) -> Result<String, VaultError> {
        if !self.meta.supports_encryption {
            return Err(VaultError::InvalidRequest("key does not support encryption".into()));
        let version = self.meta.latest_version;
        match self.versions.get(&version) {
            Some(KeyMaterial::Aes256Gcm(key_bytes)) => {
                let mut nonce_buf = [0u8; 12];
                rng.fill(&mut nonce_buf)
                    .map_err(|_| VaultError::CryptoError("nonce".into()))?;
                let uk = UnboundKey::new(&AES_256_GCM, key_bytes)
                    .map_err(|_| VaultError::CryptoError("aes key".into()))?;
                let key = LessSafeKey::new(uk);
                let nonce = Nonce::assume_unique_for_key(nonce_buf);
                let empty: &[u8] = b"";
                let aad = context.map(Aad::from).unwrap_or_else(|| Aad::from(empty));
                    .map_err(|_| VaultError::CryptoError("seal".into()))?;
                let mut combined = nonce_buf.to_vec();
                combined.extend_from_slice(&buf);
                Ok(format!("vault:v{}:{}", version, B64.encode(&combined)))
            Some(KeyMaterial::Rsa2048(der)) => {
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let pub_key = priv_key.to_public_key();
                let mut rng2 = rand::thread_rng();
                let ct = pub_key
                    .encrypt(&mut rng2, Oaep::new::<Sha256>(), plaintext)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(format!("vault:v{}:{}", version, B64.encode(&ct)))
            _ => Err(VaultError::InvalidRequest("wrong key type for encrypt".into())),
    pub fn decrypt(&self, ciphertext: &str, context: Option<&[u8]>) -> Result<Vec<u8>, VaultError> {
        if !self.meta.supports_decryption {
            return Err(VaultError::InvalidRequest("key does not support decryption".into()));
        let (version, b64_data) = Self::parse_vault_token(ciphertext)?;
        if version < self.meta.min_decryption_version {
            return Err(VaultError::PermissionDenied(format!(
                "version {version} below min_decryption_version"
            )));
        match self.versions.get(&version) {
            Some(KeyMaterial::Aes256Gcm(key_bytes)) => {
                let combined = B64.decode(b64_data)
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                if combined.len() < 12 {
                    return Err(VaultError::InvalidRequest("ciphertext too short".into()));
                let (nonce_bytes, ct) = combined.split_at(12);
                let nonce_arr: [u8; 12] = nonce_bytes.try_into()
                    .map_err(|_| VaultError::CryptoError("nonce size".into()))?;
                let uk = UnboundKey::new(&AES_256_GCM, key_bytes)
                    .map_err(|_| VaultError::CryptoError("aes key".into()))?;
                let key = LessSafeKey::new(uk);
                let nonce = Nonce::assume_unique_for_key(nonce_arr);
                let empty: &[u8] = b"";
                let aad = context.map(Aad::from).unwrap_or_else(|| Aad::from(empty));
                let mut buf = ct.to_vec();
                let pt = key
                    .map_err(|_| VaultError::CryptoError("open failed".into()))?;
                Ok(pt.to_vec())
            Some(KeyMaterial::Rsa2048(der)) => {
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let ct = B64.decode(b64_data)
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                let pt = priv_key
                    .decrypt(Oaep::new::<Sha256>(), &ct)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(pt)
            _ => Err(VaultError::InvalidRequest("wrong key type for decrypt".into())),
    pub fn rewrap(&self, ciphertext: &str, context: Option<&[u8]>) -> Result<String, VaultError> {
        let pt = self.decrypt(ciphertext, context)?;
        self.encrypt(&pt, context)
    // ── Sign / Verify (Ed25519, RSA-PKCS1v15) ────────────────────────────────
    pub fn sign(&self, data: &[u8]) -> Result<String, VaultError> {
        if !self.meta.supports_signing {
            return Err(VaultError::InvalidRequest("key does not support signing".into()));
        let version = self.meta.latest_version;
        match self.versions.get(&version) {
            Some(KeyMaterial::Ed25519(pkcs8)) => {
                    .map_err(|_| VaultError::CryptoError("ed25519 key parse".into()))?;
                Ok(format!("vault:v{}:{}", version, B64.encode(sig.as_ref())))
            Some(KeyMaterial::Rsa2048(der)) => {
                use rsa::pkcs1v15::SigningKey;
                use rsa::signature::RandomizedSigner;
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let signing_key = SigningKey::<Sha256>::new(priv_key);
                let mut rng2 = rand::thread_rng();
                let sig = signing_key.sign_with_rng(&mut rng2, data);
                use rsa::signature::SignatureEncoding;
                Ok(format!("vault:v{}:{}", version, B64.encode(sig.to_bytes())))
            _ => Err(VaultError::InvalidRequest("wrong key type for signing".into())),
    pub fn verify(&self, data: &[u8], sig_str: &str) -> Result<bool, VaultError> {
        let (version, b64_sig) = Self::parse_vault_token(sig_str)?;
        let sig_bytes = B64.decode(b64_sig)
            .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
        match self.versions.get(&version) {
            Some(KeyMaterial::Ed25519(pkcs8)) => {
                    .map_err(|_| VaultError::CryptoError("ed25519 key".into()))?;
                let pub_key = UnparsedPublicKey::new(&ED25519, kp.public_key().as_ref());
            Some(KeyMaterial::Rsa2048(der)) => {
                use rsa::pkcs1v15::{Signature, VerifyingKey};
                use rsa::signature::Verifier;
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let vk = VerifyingKey::<Sha256>::from(priv_key.to_public_key());
                let sig = Signature::try_from(sig_bytes.as_slice())
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                Ok(vk.verify(data, &sig).is_ok())
            _ => Err(VaultError::InvalidRequest("wrong key type for verify".into())),
    // ── Data key generation ───────────────────────────────────────────────────
    /// Returns (plaintext_key, encrypted_key).
    pub fn generate_data_key(&self, bits: u32) -> Result<(Vec<u8>, String), VaultError> {
        if !self.meta.supports_encryption {
            return Err(VaultError::InvalidRequest("key does not support encryption".into()));
        let len = (bits / 8) as usize;
        let mut pt = vec![0u8; len];
        rng.fill(&mut pt)
            .map_err(|_| VaultError::CryptoError("datakey rng".into()))?;
        let ct = self.encrypt(&pt, None)?;
        Ok((pt, ct))
    // ── Helper ────────────────────────────────────────────────────────────────
    fn parse_vault_token(s: &str) -> Result<(u32, &str), VaultError> {
            return Err(VaultError::InvalidRequest("invalid vault token format".into()));
            .ok_or_else(|| VaultError::InvalidRequest("invalid version".into()))?;
        Ok((ver, parts[2]))
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_transit_aes_encrypt_decrypt() {
        let key = TransitKeyEntry::create("mykey", TransitKeyType::Aes256Gcm).unwrap();
        let pt = b"hello world secret";
        let ct = key.encrypt(pt, None).unwrap();
        assert!(ct.starts_with("vault:v1:"));
        let dec = key.decrypt(&ct, None).unwrap();
        assert_eq!(dec, pt);
    #[test]
    fn test_transit_aes_with_context() {
        let key = TransitKeyEntry::create("ctx", TransitKeyType::Aes256Gcm).unwrap();
        let pt = b"data";
        let ctx = b"context-label";
        let ct = key.encrypt(pt, Some(ctx)).unwrap();
        let dec = key.decrypt(&ct, Some(ctx)).unwrap();
        assert_eq!(dec, pt);
        // Wrong context must fail
        assert!(key.decrypt(&ct, Some(b"wrong")).is_err());
    #[test]
    fn test_transit_aes_rewrap() {
        let mut key = TransitKeyEntry::create("rw", TransitKeyType::Aes256Gcm).unwrap();
        let pt = b"sensitive";
        let ct_v1 = key.encrypt(pt, None).unwrap();
        assert!(ct_v1.contains(":v1:"));
        key.rotate().unwrap();
        assert_eq!(key.meta.latest_version, 2);
        let ct_v2 = key.rewrap(&ct_v1, None).unwrap();
        assert!(ct_v2.contains(":v2:"));
        let dec = key.decrypt(&ct_v2, None).unwrap();
        assert_eq!(dec, pt);
    #[test]
    fn test_transit_ed25519_sign_verify() {
        let key = TransitKeyEntry::create("signing", TransitKeyType::Ed25519).unwrap();
        let data = b"message to sign";
        let sig = key.sign(data).unwrap();
        assert!(sig.starts_with("vault:v1:"));
        assert!(key.verify(data, &sig).unwrap());
        // Tampered message fails
        assert!(!key.verify(b"other message", &sig).unwrap());
    #[test]
    fn test_transit_rsa_encrypt_decrypt() {
        let key = TransitKeyEntry::create("rsa", TransitKeyType::Rsa2048).unwrap();
        let pt = b"rsa-secret";
        let ct = key.encrypt(pt, None).unwrap();
        let dec = key.decrypt(&ct, None).unwrap();
        assert_eq!(dec, pt);
    #[test]
    fn test_transit_key_rotation() {
        let mut key = TransitKeyEntry::create("rot", TransitKeyType::Aes256Gcm).unwrap();
        key.rotate().unwrap();
        key.rotate().unwrap();
        assert_eq!(key.meta.latest_version, 3);
        assert_eq!(key.versions.len(), 3);
    #[test]
    fn test_transit_datakey() {
        let key = TransitKeyEntry::create("dk", TransitKeyType::Aes256Gcm).unwrap();
        let (pt, ct) = key.generate_data_key(256).unwrap();
        assert_eq!(pt.len(), 32);
        let dec = key.decrypt(&ct, None).unwrap();
        assert_eq!(dec, pt);
    #[test]
    fn test_transit_min_decrypt_version_enforced() {
        let mut key = TransitKeyEntry::create("mdv", TransitKeyType::Aes256Gcm).unwrap();
        let ct_v1 = key.encrypt(b"old", None).unwrap();
        key.rotate().unwrap();
        key.meta.min_decryption_version = 2;
        assert!(key.decrypt(&ct_v1, None).is_err());
}
