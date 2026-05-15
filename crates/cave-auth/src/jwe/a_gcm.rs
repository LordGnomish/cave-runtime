// SPDX-License-Identifier: AGPL-3.0-or-later
//
// AES-GCM content encryption (`A128GCM` / `A256GCM`) — RFC 7518 §5.3.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/jose/jwe/enc/AesGcmJWEEncryptionProvider.java
//
// `A192GCM` is intentionally out-of-scope (see jwe/mod.rs).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};

use crate::jwe::JweError;

/// JWE A*GCM uses a 96-bit (12-byte) IV (RFC 7518 §5.3).
pub const GCM_IV_BYTES: usize = 12;
/// JWE A*GCM uses a 128-bit (16-byte) authentication tag (RFC 7518 §5.3).
pub const GCM_TAG_BYTES: usize = 16;

/// `A128GCM` encrypt — returns `(ciphertext, tag)`. The tag is the trailing
/// 16 bytes of the AEAD output split off so the caller can place it in the
/// fifth JWE segment.
pub fn encrypt_a128gcm(
    cek: &[u8; 16],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; GCM_TAG_BYTES]), JweError> {
    if iv.len() != GCM_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let cipher = Aes128Gcm::new_from_slice(cek).map_err(|_| JweError::InvalidKeyLength)?;
    let nonce = Nonce::from_slice(iv);
    let mut out = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| JweError::AuthFailed)?;
    let tag_start = out.len() - GCM_TAG_BYTES;
    let mut tag = [0u8; GCM_TAG_BYTES];
    tag.copy_from_slice(&out[tag_start..]);
    out.truncate(tag_start);
    Ok((out, tag))
}

/// `A128GCM` decrypt — verifies the tag and returns the plaintext.
pub fn decrypt_a128gcm(
    cek: &[u8; 16],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; GCM_TAG_BYTES],
) -> Result<Vec<u8>, JweError> {
    if iv.len() != GCM_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let cipher = Aes128Gcm::new_from_slice(cek).map_err(|_| JweError::InvalidKeyLength)?;
    let nonce = Nonce::from_slice(iv);
    let mut combined = Vec::with_capacity(ciphertext.len() + GCM_TAG_BYTES);
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);
    cipher
        .decrypt(nonce, Payload { msg: &combined, aad })
        .map_err(|_| JweError::AuthFailed)
}

/// `A256GCM` encrypt — returns `(ciphertext, tag)`.
pub fn encrypt_a256gcm(
    cek: &[u8; 32],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; GCM_TAG_BYTES]), JweError> {
    if iv.len() != GCM_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let cipher = Aes256Gcm::new_from_slice(cek).map_err(|_| JweError::InvalidKeyLength)?;
    let nonce = Nonce::from_slice(iv);
    let mut out = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| JweError::AuthFailed)?;
    let tag_start = out.len() - GCM_TAG_BYTES;
    let mut tag = [0u8; GCM_TAG_BYTES];
    tag.copy_from_slice(&out[tag_start..]);
    out.truncate(tag_start);
    Ok((out, tag))
}

/// `A256GCM` decrypt — verifies the tag and returns the plaintext.
pub fn decrypt_a256gcm(
    cek: &[u8; 32],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; GCM_TAG_BYTES],
) -> Result<Vec<u8>, JweError> {
    if iv.len() != GCM_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let cipher = Aes256Gcm::new_from_slice(cek).map_err(|_| JweError::InvalidKeyLength)?;
    let nonce = Nonce::from_slice(iv);
    let mut combined = Vec::with_capacity(ciphertext.len() + GCM_TAG_BYTES);
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);
    cipher
        .decrypt(nonce, Payload { msg: &combined, aad })
        .map_err(|_| JweError::AuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc7518 §5.3 — A128GCM/A256GCM are AEAD with 96-bit IV and
    // 128-bit tag. Round-trip with caller-supplied AAD.
    #[test]
    fn a128gcm_round_trip() {
        let cek = [0x42u8; 16];
        let iv = [0x11u8; 12];
        let aad = b"associated";
        let pt = b"hello jwe";
        let (ct, tag) = encrypt_a128gcm(&cek, &iv, aad, pt).unwrap();
        let back = decrypt_a128gcm(&cek, &iv, aad, &ct, &tag).unwrap();
        assert_eq!(back, pt);
    }

    // upstream: rfc7518 §5.3 — A256GCM round-trip.
    #[test]
    fn a256gcm_round_trip() {
        let cek = [0x33u8; 32];
        let iv = [0x55u8; 12];
        let aad = b"a";
        let pt = b"longer payload for AES-256-GCM JWE content encryption";
        let (ct, tag) = encrypt_a256gcm(&cek, &iv, aad, pt).unwrap();
        let back = decrypt_a256gcm(&cek, &iv, aad, &ct, &tag).unwrap();
        assert_eq!(back, pt);
    }

    // upstream: rfc7518 §5.3 — flipping a bit in AAD MUST cause auth failure
    // (GCM authenticates AAD).
    #[test]
    fn a128gcm_aad_tamper_fails() {
        let cek = [0x42u8; 16];
        let iv = [0x11u8; 12];
        let (ct, tag) = encrypt_a128gcm(&cek, &iv, b"aad1", b"data").unwrap();
        let err = decrypt_a128gcm(&cek, &iv, b"aad2", &ct, &tag).unwrap_err();
        assert!(matches!(err, JweError::AuthFailed));
    }

    // upstream: rfc7518 §5.3 — flipping the tag MUST cause auth failure.
    #[test]
    fn a256gcm_tag_tamper_fails() {
        let cek = [0x33u8; 32];
        let iv = [0x55u8; 12];
        let (ct, mut tag) = encrypt_a256gcm(&cek, &iv, b"", b"data").unwrap();
        tag[0] ^= 0xff;
        let err = decrypt_a256gcm(&cek, &iv, b"", &ct, &tag).unwrap_err();
        assert!(matches!(err, JweError::AuthFailed));
    }

    // upstream: rfc7518 §5.3 — IV is exactly 96 bits (12 bytes). Wrong
    // length must be rejected before crypto.
    #[test]
    fn a128gcm_wrong_iv_length_errors() {
        let cek = [0u8; 16];
        let err = encrypt_a128gcm(&cek, &[0u8; 8], b"", b"x").unwrap_err();
        assert!(matches!(err, JweError::InvalidIvLength));
    }
}
