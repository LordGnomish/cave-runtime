// SPDX-License-Identifier: AGPL-3.0-or-later
//
// AES Key Wrap (`A128KW` / `A256KW`) — RFC 7518 §4.4 (which delegates to
// RFC 3394 §2).
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/jose/jwe/alg/AesKeyWrapAlgorithmProvider.java
//
// `A192KW` is intentionally out-of-scope (see jwe/mod.rs).

use aes_kw::{KekAes128, KekAes256};

use crate::jwe::JweError;

/// AES-128 key wrap (RFC 3394).
pub fn wrap_a128kw(kek: &[u8; 16], cek: &[u8]) -> Result<Vec<u8>, JweError> {
    if cek.len() % 8 != 0 || cek.is_empty() {
        return Err(JweError::AesKw("CEK length must be a non-zero multiple of 8".into()));
    }
    let kek = KekAes128::from(*kek);
    let mut out = vec![0u8; cek.len() + 8];
    kek.wrap(cek, &mut out)
        .map_err(|e| JweError::AesKw(e.to_string()))?;
    Ok(out)
}

/// AES-128 key unwrap (RFC 3394).
pub fn unwrap_a128kw(kek: &[u8; 16], wrapped: &[u8]) -> Result<Vec<u8>, JweError> {
    if wrapped.len() < 16 || wrapped.len() % 8 != 0 {
        return Err(JweError::AesKw("wrapped key length must be a multiple of 8 and >= 16".into()));
    }
    let kek = KekAes128::from(*kek);
    let mut out = vec![0u8; wrapped.len() - 8];
    kek.unwrap(wrapped, &mut out)
        .map_err(|_| JweError::AuthFailed)?;
    Ok(out)
}

/// AES-256 key wrap (RFC 3394).
pub fn wrap_a256kw(kek: &[u8; 32], cek: &[u8]) -> Result<Vec<u8>, JweError> {
    if cek.len() % 8 != 0 || cek.is_empty() {
        return Err(JweError::AesKw("CEK length must be a non-zero multiple of 8".into()));
    }
    let kek = KekAes256::from(*kek);
    let mut out = vec![0u8; cek.len() + 8];
    kek.wrap(cek, &mut out)
        .map_err(|e| JweError::AesKw(e.to_string()))?;
    Ok(out)
}

/// AES-256 key unwrap (RFC 3394).
pub fn unwrap_a256kw(kek: &[u8; 32], wrapped: &[u8]) -> Result<Vec<u8>, JweError> {
    if wrapped.len() < 16 || wrapped.len() % 8 != 0 {
        return Err(JweError::AesKw("wrapped key length must be a multiple of 8 and >= 16".into()));
    }
    let kek = KekAes256::from(*kek);
    let mut out = vec![0u8; wrapped.len() - 8];
    kek.unwrap(wrapped, &mut out)
        .map_err(|_| JweError::AuthFailed)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc3394 §4.1 vector "Wrap 128 bits of Key Data with a
    // 128-bit KEK". KEK = 000102030405060708090A0B0C0D0E0F,
    // KeyData = 00112233445566778899AABBCCDDEEFF,
    // Ciphertext = 1FA68B0A8112B447 AEF34BD8FB5A7B82 9D3E862371D2CFE5.
    #[test]
    fn rfc3394_a128kw_vector() {
        let kek = hex_to_arr16("000102030405060708090A0B0C0D0E0F");
        let cek = hex_to_vec("00112233445566778899AABBCCDDEEFF");
        let expected = hex_to_vec("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5");
        let wrapped = wrap_a128kw(&kek, &cek).unwrap();
        assert_eq!(wrapped, expected);
        let back = unwrap_a128kw(&kek, &wrapped).unwrap();
        assert_eq!(back, cek);
    }

    // upstream: rfc3394 §4.6 vector "Wrap 256 bits of Key Data with a
    // 256-bit KEK".
    #[test]
    fn rfc3394_a256kw_vector() {
        let kek = hex_to_arr32(
            "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        );
        let cek = hex_to_vec(
            "00112233445566778899AABBCCDDEEFF000102030405060708090A0B0C0D0E0F",
        );
        let expected = hex_to_vec(
            "28C9F404C4B810F4CBCCB35CFB87F8263F5786E2D80ED326CBC7F0E71A99F43BFB988B9B7A02DD21",
        );
        let wrapped = wrap_a256kw(&kek, &cek).unwrap();
        assert_eq!(wrapped, expected);
        let back = unwrap_a256kw(&kek, &wrapped).unwrap();
        assert_eq!(back, cek);
    }

    // upstream: rfc3394 §2.2.2 — unwrap MUST verify the integrity check
    // value. Tampering with a single byte causes an unwrap failure.
    #[test]
    fn a128kw_tampered_wrapped_key_fails_unwrap() {
        let kek = hex_to_arr16("000102030405060708090A0B0C0D0E0F");
        let cek = hex_to_vec("00112233445566778899AABBCCDDEEFF");
        let mut wrapped = wrap_a128kw(&kek, &cek).unwrap();
        wrapped[0] ^= 0x01;
        let err = unwrap_a128kw(&kek, &wrapped).unwrap_err();
        assert!(matches!(err, JweError::AuthFailed));
    }

    // upstream: rfc3394 §2 — wrapped output length is plaintext + 8 bytes.
    #[test]
    fn wrap_extends_input_by_eight_bytes() {
        let kek = [0u8; 16];
        let cek = [0u8; 16];
        let wrapped = wrap_a128kw(&kek, &cek).unwrap();
        assert_eq!(wrapped.len(), cek.len() + 8);
    }

    fn hex_to_vec(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn hex_to_arr16(s: &str) -> [u8; 16] {
        let v = hex_to_vec(s);
        let mut a = [0u8; 16];
        a.copy_from_slice(&v);
        a
    }

    fn hex_to_arr32(s: &str) -> [u8; 32] {
        let v = hex_to_vec(s);
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
}
