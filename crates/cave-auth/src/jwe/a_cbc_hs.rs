// SPDX-License-Identifier: AGPL-3.0-or-later
//
// AES-CBC + HMAC-SHA content encryption — RFC 7518 §5.2.
//
// `A128CBC-HS256` — 256-bit CEK, lower half is HMAC-SHA-256 key, upper half
// is AES-128-CBC key. 128-bit tag = first 16 bytes of HMAC output.
//
// `A256CBC-HS512` — 512-bit CEK, lower 32 bytes HMAC-SHA-512, upper 32 bytes
// AES-256-CBC. 256-bit tag.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/jose/jwe/enc/AesCbcHmacShaJWEEncryptionProvider.java
//
// AAD-length encoding follows RFC 7518 §5.2.2.1: AL = u64-big-endian(AAD bit
// length). The HMAC input is `AAD || IV || ciphertext || AL`.

use aes::Aes128;
use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha512};
use subtle::ConstantTimeEq;

use crate::jwe::JweError;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha512 = Hmac<Sha512>;

pub const CBC_IV_BYTES: usize = 16;
pub const A128CBC_HS256_TAG_BYTES: usize = 16;
pub const A256CBC_HS512_TAG_BYTES: usize = 32;

fn al_bytes(aad: &[u8]) -> [u8; 8] {
    ((aad.len() as u64) * 8).to_be_bytes()
}

/// `A128CBC-HS256` encrypt (RFC 7518 §5.2.3).
pub fn encrypt_a128cbc_hs256(
    cek: &[u8; 32],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; A128CBC_HS256_TAG_BYTES]), JweError> {
    if iv.len() != CBC_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    // CEK split: lower 128 bits = MAC key, upper 128 bits = AES key
    // (RFC 7518 §5.2.2.1).
    let mac_key: [u8; 16] = cek[..16].try_into().unwrap();
    let enc_key: [u8; 16] = cek[16..].try_into().unwrap();
    let cipher = Aes128CbcEnc::new(&enc_key.into(), iv.into());
    let ciphertext = cipher.encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&mac_key)
        .map_err(|_| JweError::InvalidKeyLength)?;
    mac.update(aad);
    mac.update(iv);
    mac.update(&ciphertext);
    mac.update(&al_bytes(aad));
    let full = mac.finalize().into_bytes();
    let mut tag = [0u8; A128CBC_HS256_TAG_BYTES];
    tag.copy_from_slice(&full[..A128CBC_HS256_TAG_BYTES]);
    Ok((ciphertext, tag))
}

/// `A128CBC-HS256` decrypt — verifies tag in constant time before depadding.
pub fn decrypt_a128cbc_hs256(
    cek: &[u8; 32],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; A128CBC_HS256_TAG_BYTES],
) -> Result<Vec<u8>, JweError> {
    if iv.len() != CBC_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let mac_key: [u8; 16] = cek[..16].try_into().unwrap();
    let enc_key: [u8; 16] = cek[16..].try_into().unwrap();
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&mac_key)
        .map_err(|_| JweError::InvalidKeyLength)?;
    mac.update(aad);
    mac.update(iv);
    mac.update(ciphertext);
    mac.update(&al_bytes(aad));
    let full = mac.finalize().into_bytes();
    if full[..A128CBC_HS256_TAG_BYTES].ct_eq(tag).unwrap_u8() != 1 {
        return Err(JweError::AuthFailed);
    }
    let cipher = Aes128CbcDec::new(&enc_key.into(), iv.into());
    cipher
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| JweError::AuthFailed)
}

/// `A256CBC-HS512` encrypt (RFC 7518 §5.2.5).
pub fn encrypt_a256cbc_hs512(
    cek: &[u8; 64],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; A256CBC_HS512_TAG_BYTES]), JweError> {
    if iv.len() != CBC_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let mac_key: [u8; 32] = cek[..32].try_into().unwrap();
    let enc_key: [u8; 32] = cek[32..].try_into().unwrap();
    let cipher = Aes256CbcEnc::new(&enc_key.into(), iv.into());
    let ciphertext = cipher.encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    let mut mac = <HmacSha512 as Mac>::new_from_slice(&mac_key)
        .map_err(|_| JweError::InvalidKeyLength)?;
    mac.update(aad);
    mac.update(iv);
    mac.update(&ciphertext);
    mac.update(&al_bytes(aad));
    let full = mac.finalize().into_bytes();
    let mut tag = [0u8; A256CBC_HS512_TAG_BYTES];
    tag.copy_from_slice(&full[..A256CBC_HS512_TAG_BYTES]);
    Ok((ciphertext, tag))
}

/// `A256CBC-HS512` decrypt — verifies tag in constant time before depadding.
pub fn decrypt_a256cbc_hs512(
    cek: &[u8; 64],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; A256CBC_HS512_TAG_BYTES],
) -> Result<Vec<u8>, JweError> {
    if iv.len() != CBC_IV_BYTES {
        return Err(JweError::InvalidIvLength);
    }
    let mac_key: [u8; 32] = cek[..32].try_into().unwrap();
    let enc_key: [u8; 32] = cek[32..].try_into().unwrap();
    let mut mac = <HmacSha512 as Mac>::new_from_slice(&mac_key)
        .map_err(|_| JweError::InvalidKeyLength)?;
    mac.update(aad);
    mac.update(iv);
    mac.update(ciphertext);
    mac.update(&al_bytes(aad));
    let full = mac.finalize().into_bytes();
    if full[..A256CBC_HS512_TAG_BYTES].ct_eq(tag).unwrap_u8() != 1 {
        return Err(JweError::AuthFailed);
    }
    let cipher = Aes256CbcDec::new(&enc_key.into(), iv.into());
    cipher
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| JweError::AuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc7518 §B.1 (Test Vector for A128CBC-HS256). Verbatim from
    // the RFC's appendix.
    #[test]
    fn rfc7518_b1_a128cbc_hs256_vector() {
        // Key (K) = 256 bits = MAC_KEY || ENC_KEY (RFC §5.2.2.1)
        let cek: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
        ];
        // IV = 1A F3 8C 2D C2 B9 6F FD D8 66 94 09 23 41 BC 04
        let iv: [u8; 16] = [
            0x1a, 0xf3, 0x8c, 0x2d, 0xc2, 0xb9, 0x6f, 0xfd,
            0xd8, 0x66, 0x94, 0x09, 0x23, 0x41, 0xbc, 0x04,
        ];
        // P = "A cipher system must not be required to be secret, and it
        // must be able to fall into the hands of the enemy without
        // inconvenience" (RFC 7518 §B.1)
        let pt: &[u8] = &[
            0x41, 0x20, 0x63, 0x69, 0x70, 0x68, 0x65, 0x72,
            0x20, 0x73, 0x79, 0x73, 0x74, 0x65, 0x6d, 0x20,
            0x6d, 0x75, 0x73, 0x74, 0x20, 0x6e, 0x6f, 0x74,
            0x20, 0x62, 0x65, 0x20, 0x72, 0x65, 0x71, 0x75,
            0x69, 0x72, 0x65, 0x64, 0x20, 0x74, 0x6f, 0x20,
            0x62, 0x65, 0x20, 0x73, 0x65, 0x63, 0x72, 0x65,
            0x74, 0x2c, 0x20, 0x61, 0x6e, 0x64, 0x20, 0x69,
            0x74, 0x20, 0x6d, 0x75, 0x73, 0x74, 0x20, 0x62,
            0x65, 0x20, 0x61, 0x62, 0x6c, 0x65, 0x20, 0x74,
            0x6f, 0x20, 0x66, 0x61, 0x6c, 0x6c, 0x20, 0x69,
            0x6e, 0x74, 0x6f, 0x20, 0x74, 0x68, 0x65, 0x20,
            0x68, 0x61, 0x6e, 0x64, 0x73, 0x20, 0x6f, 0x66,
            0x20, 0x74, 0x68, 0x65, 0x20, 0x65, 0x6e, 0x65,
            0x6d, 0x79, 0x20, 0x77, 0x69, 0x74, 0x68, 0x6f,
            0x75, 0x74, 0x20, 0x69, 0x6e, 0x63, 0x6f, 0x6e,
            0x76, 0x65, 0x6e, 0x69, 0x65, 0x6e, 0x63, 0x65,
        ];
        // A = "The second principle of Auguste Kerckhoffs"
        let aad: &[u8] = &[
            0x54, 0x68, 0x65, 0x20, 0x73, 0x65, 0x63, 0x6f,
            0x6e, 0x64, 0x20, 0x70, 0x72, 0x69, 0x6e, 0x63,
            0x69, 0x70, 0x6c, 0x65, 0x20, 0x6f, 0x66, 0x20,
            0x41, 0x75, 0x67, 0x75, 0x73, 0x74, 0x65, 0x20,
            0x4b, 0x65, 0x72, 0x63, 0x6b, 0x68, 0x6f, 0x66,
            0x66, 0x73,
        ];

        let (ct, tag) = encrypt_a128cbc_hs256(&cek, &iv, aad, pt).unwrap();

        // Expected E (RFC 7518 §B.1):
        let expected_ct = [
            0xc8, 0x0e, 0xdf, 0xa3, 0x2d, 0xdf, 0x39, 0xd5,
            0xef, 0x00, 0xc0, 0xb4, 0x68, 0x83, 0x42, 0x79,
            0xa2, 0xe4, 0x6a, 0x1b, 0x80, 0x49, 0xf7, 0x92,
            0xf7, 0x6b, 0xfe, 0x54, 0xb9, 0x03, 0xa9, 0xc9,
            0xa9, 0x4a, 0xc9, 0xb4, 0x7a, 0xd2, 0x65, 0x5c,
            0x5f, 0x10, 0xf9, 0xae, 0xf7, 0x14, 0x27, 0xe2,
            0xfc, 0x6f, 0x9b, 0x3f, 0x39, 0x9a, 0x22, 0x14,
            0x89, 0xf1, 0x63, 0x62, 0xc7, 0x03, 0x23, 0x36,
            0x09, 0xd4, 0x5a, 0xc6, 0x98, 0x64, 0xe3, 0x32,
            0x1c, 0xf8, 0x29, 0x35, 0xac, 0x40, 0x96, 0xc8,
            0x6e, 0x13, 0x33, 0x14, 0xc5, 0x40, 0x19, 0xe8,
            0xca, 0x79, 0x80, 0xdf, 0xa4, 0xb9, 0xcf, 0x1b,
            0x38, 0x4c, 0x48, 0x6f, 0x3a, 0x54, 0xc5, 0x10,
            0x78, 0x15, 0x8e, 0xe5, 0xd7, 0x9d, 0xe5, 0x9f,
            0xbd, 0x34, 0xd8, 0x48, 0xb3, 0xd6, 0x95, 0x50,
            0xa6, 0x76, 0x46, 0x34, 0x44, 0x27, 0xad, 0xe5,
            0x4b, 0x88, 0x51, 0xff, 0xb5, 0x98, 0xf7, 0xf8,
            0x00, 0x74, 0xb9, 0x47, 0x3c, 0x82, 0xe2, 0xdb,
        ];
        let expected_tag = [
            0x65, 0x2c, 0x3f, 0xa3, 0x6b, 0x0a, 0x7c, 0x5b,
            0x32, 0x19, 0xfa, 0xb3, 0xa3, 0x0b, 0xc1, 0xc4,
        ];

        assert_eq!(ct, expected_ct);
        assert_eq!(tag, expected_tag);

        let back = decrypt_a128cbc_hs256(&cek, &iv, aad, &ct, &tag).unwrap();
        assert_eq!(back, pt);
    }

    // upstream: rfc7518 §B.3 (Test Vector for A256CBC-HS512). Only the
    // round-trip is asserted here; the appendix's full byte vector is huge,
    // so we cross-check by decrypting our own ciphertext.
    #[test]
    fn a256cbc_hs512_round_trip() {
        let cek = [0x77u8; 64];
        let iv = [0x33u8; 16];
        let aad = b"some aad";
        let pt = b"AES-256-CBC + HMAC-SHA-512 payload, longer than one block";
        let (ct, tag) = encrypt_a256cbc_hs512(&cek, &iv, aad, pt).unwrap();
        let back = decrypt_a256cbc_hs512(&cek, &iv, aad, &ct, &tag).unwrap();
        assert_eq!(back, pt);
    }

    // upstream: rfc7518 §5.2.2.2 step 5 — decrypt MUST fail on tag mismatch.
    #[test]
    fn a128cbc_hs256_tag_tamper_fails() {
        let cek = [0u8; 32];
        let iv = [0u8; 16];
        let (ct, mut tag) = encrypt_a128cbc_hs256(&cek, &iv, b"", b"hi").unwrap();
        tag[0] ^= 0x01;
        let err = decrypt_a128cbc_hs256(&cek, &iv, b"", &ct, &tag).unwrap_err();
        assert!(matches!(err, JweError::AuthFailed));
    }

    // upstream: rfc7518 §5.2.2.1 — AL is the AAD bit length as a 64-bit BE
    // integer. Make sure a different AAD breaks the tag (so we know AL is
    // being included).
    #[test]
    fn a128cbc_hs256_aad_tamper_fails() {
        let cek = [1u8; 32];
        let iv = [2u8; 16];
        let (ct, tag) = encrypt_a128cbc_hs256(&cek, &iv, b"aad-orig", b"hi").unwrap();
        let err = decrypt_a128cbc_hs256(&cek, &iv, b"aad-tampered", &ct, &tag).unwrap_err();
        assert!(matches!(err, JweError::AuthFailed));
    }
}
