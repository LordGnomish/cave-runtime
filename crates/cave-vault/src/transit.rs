//! Transit Secrets Engine — AES-256-GCM, Ed25519, RSA-2048.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519},
};
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    Oaep, RsaPrivateKey,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;

use crate::error::VaultError;

// ── Key types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TransitKeyType {
    Aes256Gcm,
    Ed25519,
    Rsa2048,
}

/// Raw key material — stored in memory only, never serialised in responses.
#[derive(Clone)]
pub enum KeyMaterial {
    Aes256Gcm(Vec<u8>),  // 32 raw bytes
    Ed25519(Vec<u8>),    // PKCS8 DER
    Rsa2048(Vec<u8>),    // PKCS8 DER
}

impl KeyMaterial {
    fn generate(kt: &TransitKeyType) -> Result<Self, VaultError> {
        let rng = SystemRandom::new();
        match kt {
            TransitKeyType::Aes256Gcm => {
                let mut key = vec![0u8; 32];
                rng.fill(&mut key)
                    .map_err(|_| VaultError::CryptoError("rng fill".into()))?;
                Ok(KeyMaterial::Aes256Gcm(key))
            }
            TransitKeyType::Ed25519 => {
                let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
                    .map_err(|_| VaultError::CryptoError("ed25519 keygen".into()))?;
                Ok(KeyMaterial::Ed25519(pkcs8.as_ref().to_vec()))
            }
            TransitKeyType::Rsa2048 => {
                let mut rng2 = rand::thread_rng();
                let priv_key = RsaPrivateKey::new(&mut rng2, 2048)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let der = priv_key
                    .to_pkcs8_der()
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(KeyMaterial::Rsa2048(der.as_bytes().to_vec()))
            }
        }
    }
}

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
}

// ── TransitKeyEntry (metadata + key material) ────────────────────────────────

pub struct TransitKeyEntry {
    pub meta: TransitKey,
    pub versions: HashMap<u32, KeyMaterial>,
}

impl TransitKeyEntry {
    pub fn create(name: &str, key_type: TransitKeyType) -> Result<Self, VaultError> {
        let material = KeyMaterial::generate(&key_type)?;
        let (enc, dec, sign, deriv) = match key_type {
            TransitKeyType::Aes256Gcm => (true, true, false, true),
            TransitKeyType::Ed25519   => (false, false, true, false),
            TransitKeyType::Rsa2048  => (true, true, true, false),
        };
        let mut versions = HashMap::new();
        versions.insert(1, material);
        Ok(Self {
            meta: TransitKey {
                name: name.to_string(),
                key_type,
                latest_version: 1,
                min_decryption_version: 1,
                deletion_allowed: false,
                exportable: false,
                supports_encryption: enc,
                supports_decryption: dec,
                supports_signing: sign,
                supports_derivation: deriv,
            },
            versions,
        })
    }

    pub fn rotate(&mut self) -> Result<(), VaultError> {
        let next = self.meta.latest_version + 1;
        let material = KeyMaterial::generate(&self.meta.key_type)?;
        self.versions.insert(next, material);
        self.meta.latest_version = next;
        Ok(())
    }

    // ── Encrypt / Decrypt (AES-256-GCM) ──────────────────────────────────────

    pub fn encrypt(&self, plaintext: &[u8], context: Option<&[u8]>) -> Result<String, VaultError> {
        if !self.meta.supports_encryption {
            return Err(VaultError::InvalidRequest("key does not support encryption".into()));
        }
        let version = self.meta.latest_version;
        match self.versions.get(&version) {
            Some(KeyMaterial::Aes256Gcm(key_bytes)) => {
                let rng = SystemRandom::new();
                let mut nonce_buf = [0u8; 12];
                rng.fill(&mut nonce_buf)
                    .map_err(|_| VaultError::CryptoError("nonce".into()))?;

                let uk = UnboundKey::new(&AES_256_GCM, key_bytes)
                    .map_err(|_| VaultError::CryptoError("aes key".into()))?;
                let key = LessSafeKey::new(uk);
                let nonce = Nonce::assume_unique_for_key(nonce_buf);
                let empty: &[u8] = b"";
                let aad = context.map(Aad::from).unwrap_or_else(|| Aad::from(empty));

                let mut buf = plaintext.to_vec();
                key.seal_in_place_append_tag(nonce, aad, &mut buf)
                    .map_err(|_| VaultError::CryptoError("seal".into()))?;

                let mut combined = nonce_buf.to_vec();
                combined.extend_from_slice(&buf);
                Ok(format!("vault:v{}:{}", version, B64.encode(&combined)))
            }
            Some(KeyMaterial::Rsa2048(der)) => {
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let pub_key = priv_key.to_public_key();
                let mut rng2 = rand::thread_rng();
                let ct = pub_key
                    .encrypt(&mut rng2, Oaep::new::<Sha256>(), plaintext)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(format!("vault:v{}:{}", version, B64.encode(&ct)))
            }
            _ => Err(VaultError::InvalidRequest("wrong key type for encrypt".into())),
        }
    }

    pub fn decrypt(&self, ciphertext: &str, context: Option<&[u8]>) -> Result<Vec<u8>, VaultError> {
        if !self.meta.supports_decryption {
            return Err(VaultError::InvalidRequest("key does not support decryption".into()));
        }
        let (version, b64_data) = Self::parse_vault_token(ciphertext)?;
        if version < self.meta.min_decryption_version {
            return Err(VaultError::PermissionDenied(format!(
                "version {version} below min_decryption_version"
            )));
        }

        match self.versions.get(&version) {
            Some(KeyMaterial::Aes256Gcm(key_bytes)) => {
                let combined = B64.decode(b64_data)
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                if combined.len() < 12 {
                    return Err(VaultError::InvalidRequest("ciphertext too short".into()));
                }
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
                    .open_in_place(nonce, aad, &mut buf)
                    .map_err(|_| VaultError::CryptoError("open failed".into()))?;
                Ok(pt.to_vec())
            }
            Some(KeyMaterial::Rsa2048(der)) => {
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let ct = B64.decode(b64_data)
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                let pt = priv_key
                    .decrypt(Oaep::new::<Sha256>(), &ct)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                Ok(pt)
            }
            _ => Err(VaultError::InvalidRequest("wrong key type for decrypt".into())),
        }
    }

    pub fn rewrap(&self, ciphertext: &str, context: Option<&[u8]>) -> Result<String, VaultError> {
        let pt = self.decrypt(ciphertext, context)?;
        self.encrypt(&pt, context)
    }

    // ── Sign / Verify (Ed25519, RSA-PKCS1v15) ────────────────────────────────

    pub fn sign(&self, data: &[u8]) -> Result<String, VaultError> {
        if !self.meta.supports_signing {
            return Err(VaultError::InvalidRequest("key does not support signing".into()));
        }
        let version = self.meta.latest_version;
        match self.versions.get(&version) {
            Some(KeyMaterial::Ed25519(pkcs8)) => {
                let kp = Ed25519KeyPair::from_pkcs8(pkcs8)
                    .map_err(|_| VaultError::CryptoError("ed25519 key parse".into()))?;
                let sig = kp.sign(data);
                Ok(format!("vault:v{}:{}", version, B64.encode(sig.as_ref())))
            }
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
            }
            _ => Err(VaultError::InvalidRequest("wrong key type for signing".into())),
        }
    }

    pub fn verify(&self, data: &[u8], sig_str: &str) -> Result<bool, VaultError> {
        let (version, b64_sig) = Self::parse_vault_token(sig_str)?;
        let sig_bytes = B64.decode(b64_sig)
            .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;

        match self.versions.get(&version) {
            Some(KeyMaterial::Ed25519(pkcs8)) => {
                let kp = Ed25519KeyPair::from_pkcs8(pkcs8)
                    .map_err(|_| VaultError::CryptoError("ed25519 key".into()))?;
                let pub_key = UnparsedPublicKey::new(&ED25519, kp.public_key().as_ref());
                Ok(pub_key.verify(data, &sig_bytes).is_ok())
            }
            Some(KeyMaterial::Rsa2048(der)) => {
                use rsa::pkcs1v15::{Signature, VerifyingKey};
                use rsa::signature::Verifier;
                let priv_key = RsaPrivateKey::from_pkcs8_der(der)
                    .map_err(|e| VaultError::CryptoError(e.to_string()))?;
                let vk = VerifyingKey::<Sha256>::from(priv_key.to_public_key());
                let sig = Signature::try_from(sig_bytes.as_slice())
                    .map_err(|e| VaultError::InvalidRequest(e.to_string()))?;
                Ok(vk.verify(data, &sig).is_ok())
            }
            _ => Err(VaultError::InvalidRequest("wrong key type for verify".into())),
        }
    }

    // ── Data key generation ───────────────────────────────────────────────────

    /// Returns (plaintext_key, encrypted_key).
    pub fn generate_data_key(&self, bits: u32) -> Result<(Vec<u8>, String), VaultError> {
        if !self.meta.supports_encryption {
            return Err(VaultError::InvalidRequest("key does not support encryption".into()));
        }
        let rng = SystemRandom::new();
        let len = (bits / 8) as usize;
        let mut pt = vec![0u8; len];
        rng.fill(&mut pt)
            .map_err(|_| VaultError::CryptoError("datakey rng".into()))?;
        let ct = self.encrypt(&pt, None)?;
        Ok((pt, ct))
    }

    // ── Helper ────────────────────────────────────────────────────────────────

    fn parse_vault_token(s: &str) -> Result<(u32, &str), VaultError> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() != 3 || parts[0] != "vault" {
            return Err(VaultError::InvalidRequest("invalid vault token format".into()));
        }
        let ver: u32 = parts[1]
            .strip_prefix('v')
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| VaultError::InvalidRequest("invalid version".into()))?;
        Ok((ver, parts[2]))
    }
}

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
    }

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
    }

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
    }

    #[test]
    fn test_transit_ed25519_sign_verify() {
        let key = TransitKeyEntry::create("signing", TransitKeyType::Ed25519).unwrap();
        let data = b"message to sign";
        let sig = key.sign(data).unwrap();
        assert!(sig.starts_with("vault:v1:"));
        assert!(key.verify(data, &sig).unwrap());
        // Tampered message fails
        assert!(!key.verify(b"other message", &sig).unwrap());
    }

    #[test]
    fn test_transit_rsa_encrypt_decrypt() {
        let key = TransitKeyEntry::create("rsa", TransitKeyType::Rsa2048).unwrap();
        let pt = b"rsa-secret";
        let ct = key.encrypt(pt, None).unwrap();
        let dec = key.decrypt(&ct, None).unwrap();
        assert_eq!(dec, pt);
    }

    #[test]
    fn test_transit_key_rotation() {
        let mut key = TransitKeyEntry::create("rot", TransitKeyType::Aes256Gcm).unwrap();
        key.rotate().unwrap();
        key.rotate().unwrap();
        assert_eq!(key.meta.latest_version, 3);
        assert_eq!(key.versions.len(), 3);
    }

    #[test]
    fn test_transit_datakey() {
        let key = TransitKeyEntry::create("dk", TransitKeyType::Aes256Gcm).unwrap();
        let (pt, ct) = key.generate_data_key(256).unwrap();
        assert_eq!(pt.len(), 32);
        let dec = key.decrypt(&ct, None).unwrap();
        assert_eq!(dec, pt);
    }

    #[test]
    fn test_transit_min_decrypt_version_enforced() {
        let mut key = TransitKeyEntry::create("mdv", TransitKeyType::Aes256Gcm).unwrap();
        let ct_v1 = key.encrypt(b"old", None).unwrap();
        key.rotate().unwrap();
        key.meta.min_decryption_version = 2;
        assert!(key.decrypt(&ct_v1, None).is_err());
    }
}
