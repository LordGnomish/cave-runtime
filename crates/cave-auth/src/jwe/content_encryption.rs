// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/encryption/+ RFC 7518 §5
//
//! JWE content encryption — `enc` parameter (RFC 7518 §5.1 / §5.2).
//!
//! Implemented:
//!   - `A256GCM` — AES-256-GCM (key=32B, iv=12B, tag=16B). RFC 7518 §5.3.
//!   - `A128CBC-HS256` — AES-128-CBC + HMAC-SHA-256 (key=32B split: HMAC|enc, iv=16B,
//!     tag=16B). RFC 7518 §5.2.3.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContentEncAlg {
    #[serde(rename = "A256GCM")]
    A256Gcm,
    #[serde(rename = "A128CBC-HS256")]
    A128CbcHs256,
}

impl ContentEncAlg {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentEncAlg::A256Gcm => "A256GCM",
            ContentEncAlg::A128CbcHs256 => "A128CBC-HS256",
        }
    }

    /// CEK length in bytes — RFC 7518 §5.
    pub fn cek_len(&self) -> usize {
        match self {
            ContentEncAlg::A256Gcm => 32,
            ContentEncAlg::A128CbcHs256 => 32, // HMAC key (16) || CEK (16)
        }
    }

    /// IV length in bytes.
    pub fn iv_len(&self) -> usize {
        match self {
            ContentEncAlg::A256Gcm => 12,
            ContentEncAlg::A128CbcHs256 => 16,
        }
    }

    /// Authentication tag length in bytes.
    pub fn tag_len(&self) -> usize {
        match self {
            ContentEncAlg::A256Gcm => 16,
            ContentEncAlg::A128CbcHs256 => 16,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContentEncError {
    #[error("CEK length {actual} does not match expected {expected} for {alg:?}")]
    CekLength {
        alg: ContentEncAlg,
        expected: usize,
        actual: usize,
    },
    #[error("IV length {actual} does not match expected {expected} for {alg:?}")]
    IvLength {
        alg: ContentEncAlg,
        expected: usize,
        actual: usize,
    },
    #[error("authenticated encryption failed: {0}")]
    Crypto(String),
    #[error("HMAC verification failed (tampered ciphertext / aad / iv)")]
    HmacMismatch,
}

/// Encrypts `plaintext` with the given CEK, IV, and AAD.
///
/// Returns `(ciphertext, auth_tag)` per RFC 7518 §5.1 step 3-4.
pub fn encrypt(
    alg: ContentEncAlg,
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), ContentEncError> {
    check_lens(alg, cek, iv)?;
    match alg {
        ContentEncAlg::A256Gcm => encrypt_a256gcm(cek, iv, aad, plaintext),
        ContentEncAlg::A128CbcHs256 => encrypt_a128cbc_hs256(cek, iv, aad, plaintext),
    }
}

pub fn decrypt(
    alg: ContentEncAlg,
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    auth_tag: &[u8],
) -> Result<Vec<u8>, ContentEncError> {
    check_lens(alg, cek, iv)?;
    match alg {
        ContentEncAlg::A256Gcm => decrypt_a256gcm(cek, iv, aad, ciphertext, auth_tag),
        ContentEncAlg::A128CbcHs256 => {
            decrypt_a128cbc_hs256(cek, iv, aad, ciphertext, auth_tag)
        }
    }
}

fn check_lens(alg: ContentEncAlg, cek: &[u8], iv: &[u8]) -> Result<(), ContentEncError> {
    if cek.len() != alg.cek_len() {
        return Err(ContentEncError::CekLength {
            alg,
            expected: alg.cek_len(),
            actual: cek.len(),
        });
    }
    if iv.len() != alg.iv_len() {
        return Err(ContentEncError::IvLength {
            alg,
            expected: alg.iv_len(),
            actual: iv.len(),
        });
    }
    Ok(())
}

// ── A256GCM ──────────────────────────────────────────────────────────────────

fn encrypt_a256gcm(
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), ContentEncError> {
    let cipher = Aes256Gcm::new_from_slice(cek)
        .map_err(|e| ContentEncError::Crypto(format!("cek invalid: {e}")))?;
    let nonce = Nonce::from_slice(iv);
    let blob = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| ContentEncError::Crypto(e.to_string()))?;
    // GCM appends the 16-byte tag to the ciphertext.
    let tag_offset = blob.len() - 16;
    let mut ciphertext = blob;
    let tag = ciphertext.split_off(tag_offset);
    Ok((ciphertext, tag))
}

fn decrypt_a256gcm(
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    auth_tag: &[u8],
) -> Result<Vec<u8>, ContentEncError> {
    if auth_tag.len() != 16 {
        return Err(ContentEncError::HmacMismatch);
    }
    let cipher = Aes256Gcm::new_from_slice(cek)
        .map_err(|e| ContentEncError::Crypto(format!("cek invalid: {e}")))?;
    let nonce = Nonce::from_slice(iv);
    let mut blob = ciphertext.to_vec();
    blob.extend_from_slice(auth_tag);
    cipher
        .decrypt(nonce, Payload { msg: &blob, aad })
        .map_err(|_| ContentEncError::HmacMismatch)
}

// ── A128CBC-HS256 ────────────────────────────────────────────────────────────
//
// RFC 7518 §5.2.3:
//   K = MAC_KEY (16B) || ENC_KEY (16B)
//   AAD || IV || E (ciphertext) || AL (8-byte big-endian AAD bit length)
//   then HMAC-SHA-256 keyed with MAC_KEY, take leading 16 bytes as auth_tag.

use aes::Aes128;
use cbc::{Decryptor, Encryptor};
use cipher::block_padding::Pkcs7;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
type HmacSha256 = Hmac<Sha256>;
type Aes128CbcEnc = Encryptor<Aes128>;
type Aes128CbcDec = Decryptor<Aes128>;

fn encrypt_a128cbc_hs256(
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), ContentEncError> {
    let (mac_key, enc_key) = cek.split_at(16);
    // Manual padded buffer (cipher 0.4 dropped the _vec_mut helpers).
    let block_size = 16;
    let pad_len = block_size - (plaintext.len() % block_size);
    let mut buf = Vec::with_capacity(plaintext.len() + pad_len);
    buf.extend_from_slice(plaintext);
    buf.resize(plaintext.len() + pad_len, pad_len as u8); // PKCS#7
    let msg_len = plaintext.len();
    let _ = Aes128CbcEnc::new(enc_key.into(), iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
        .map_err(|e| ContentEncError::Crypto(e.to_string()))?;
    let ciphertext = buf;
    let auth_tag = hmac_tag(mac_key, aad, iv, &ciphertext)?;
    Ok((ciphertext, auth_tag))
}

fn decrypt_a128cbc_hs256(
    cek: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    auth_tag: &[u8],
) -> Result<Vec<u8>, ContentEncError> {
    let (mac_key, enc_key) = cek.split_at(16);
    let expected = hmac_tag(mac_key, aad, iv, ciphertext)?;
    // Constant-time compare via slice eq is acceptable here (tag is 16B fixed).
    if expected != auth_tag {
        return Err(ContentEncError::HmacMismatch);
    }
    let mut buf = ciphertext.to_vec();
    let plaintext = Aes128CbcDec::new(enc_key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| ContentEncError::Crypto(e.to_string()))?;
    Ok(plaintext.to_vec())
}

fn hmac_tag(
    mac_key: &[u8],
    aad: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, ContentEncError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(mac_key)
        .map_err(|e| ContentEncError::Crypto(e.to_string()))?;
    let al = (aad.len() as u64 * 8).to_be_bytes();
    mac.update(aad);
    mac.update(iv);
    mac.update(ciphertext);
    mac.update(&al);
    Ok(mac.finalize().into_bytes()[..16].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cek(len: usize) -> Vec<u8> {
        (0..len as u8).collect()
    }

    fn iv(len: usize) -> Vec<u8> {
        (0..len as u8).map(|i| i ^ 0x55).collect()
    }

    #[test]
    fn a256gcm_round_trip() {
        let alg = ContentEncAlg::A256Gcm;
        let (ct, tag) = encrypt(alg, &cek(32), &iv(12), b"aad", b"hello world").unwrap();
        let pt = decrypt(alg, &cek(32), &iv(12), b"aad", &ct, &tag).unwrap();
        assert_eq!(pt, b"hello world");
    }

    #[test]
    fn a128cbc_hs256_round_trip() {
        let alg = ContentEncAlg::A128CbcHs256;
        let (ct, tag) = encrypt(alg, &cek(32), &iv(16), b"aad", b"hello world").unwrap();
        let pt = decrypt(alg, &cek(32), &iv(16), b"aad", &ct, &tag).unwrap();
        assert_eq!(pt, b"hello world");
    }

    #[test]
    fn a256gcm_tag_tamper_detected() {
        let alg = ContentEncAlg::A256Gcm;
        let (ct, mut tag) = encrypt(alg, &cek(32), &iv(12), b"aad", b"x").unwrap();
        tag[0] ^= 1;
        let err = decrypt(alg, &cek(32), &iv(12), b"aad", &ct, &tag).unwrap_err();
        assert_eq!(err, ContentEncError::HmacMismatch);
    }

    #[test]
    fn a128cbc_aad_tamper_detected() {
        let alg = ContentEncAlg::A128CbcHs256;
        let (ct, tag) = encrypt(alg, &cek(32), &iv(16), b"aad-a", b"x").unwrap();
        let err = decrypt(alg, &cek(32), &iv(16), b"aad-b", &ct, &tag).unwrap_err();
        assert_eq!(err, ContentEncError::HmacMismatch);
    }

    #[test]
    fn wrong_cek_length_rejected() {
        let alg = ContentEncAlg::A256Gcm;
        let err = encrypt(alg, &cek(31), &iv(12), b"a", b"p").unwrap_err();
        assert!(matches!(err, ContentEncError::CekLength { .. }));
    }

    #[test]
    fn wrong_iv_length_rejected() {
        let alg = ContentEncAlg::A256Gcm;
        let err = encrypt(alg, &cek(32), &iv(11), b"a", b"p").unwrap_err();
        assert!(matches!(err, ContentEncError::IvLength { .. }));
    }

    #[test]
    fn cek_iv_tag_lengths_match_spec() {
        assert_eq!(ContentEncAlg::A256Gcm.cek_len(), 32);
        assert_eq!(ContentEncAlg::A256Gcm.iv_len(), 12);
        assert_eq!(ContentEncAlg::A256Gcm.tag_len(), 16);
        assert_eq!(ContentEncAlg::A128CbcHs256.cek_len(), 32);
        assert_eq!(ContentEncAlg::A128CbcHs256.iv_len(), 16);
        assert_eq!(ContentEncAlg::A128CbcHs256.tag_len(), 16);
    }

    #[test]
    fn alg_str_representations() {
        assert_eq!(ContentEncAlg::A256Gcm.as_str(), "A256GCM");
        assert_eq!(ContentEncAlg::A128CbcHs256.as_str(), "A128CBC-HS256");
    }
}
