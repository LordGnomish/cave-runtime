// SPDX-License-Identifier: AGPL-3.0-or-later
//
// JSON Web Encryption (JWE) — RFC 7516.
//
// Upstream parity:
//   - keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38  (v22.0.0)
//     services/src/main/java/org/keycloak/crypto/CekManagementProvider.java
//     services/src/main/java/org/keycloak/jose/jwe/JWE.java
//   - RFC 7516 — JSON Web Encryption
//   - RFC 7518 §4 (alg) + §5 (enc)
//
// Submodules:
//   - [`compact`] — 5-segment compact serialization (BASE64URL(header).key.iv.ct.tag)
//   - [`rsa_oaep`] — `RSA-OAEP` + `RSA-OAEP-256` key encryption
//   - [`aes_kw`]   — `A128KW` + `A256KW` symmetric key wrapping (RFC 3394)
//   - [`a_gcm`]    — `A128GCM` + `A256GCM` content encryption
//   - [`a_cbc_hs`] — `A128CBC-HS256` + `A256CBC-HS512` (RFC 7518 §5.2)
//
// Not in scope (Phase 2, see parity manifest `status="missing"`):
//   - `PBES2-HS256+A128KW` / `PBES2-HS384+A192KW` / `PBES2-HS512+A256KW`
//   - `ECDH-ES` + `ECDH-ES+A128KW` / `ECDH-ES+A192KW` / `ECDH-ES+A256KW`
//   - `A192GCM` / `A192KW` / `A192CBC-HS384` (192-bit family)

pub mod a_cbc_hs;
pub mod a_gcm;
pub mod aes_kw;
pub mod compact;
pub mod rsa_oaep;

use serde::{Deserialize, Serialize};

/// Key-encryption algorithms supported by `cave-auth`.
///
/// Maps onto the `alg` Header Parameter (RFC 7516 §4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyAlg {
    /// RSAES OAEP using default parameters (SHA-1, MGF1-SHA-1) — RFC 7518 §4.3.
    #[serde(rename = "RSA-OAEP")]
    RsaOaep,
    /// RSAES OAEP using SHA-256 and MGF1-SHA-256 — RFC 7518 §4.3.
    #[serde(rename = "RSA-OAEP-256")]
    RsaOaep256,
    /// Direct use of a shared symmetric key as the CEK — RFC 7518 §4.5.
    #[serde(rename = "dir")]
    Dir,
    /// AES Key Wrap with 128-bit key — RFC 7518 §4.4.
    #[serde(rename = "A128KW")]
    A128Kw,
    /// AES Key Wrap with 256-bit key — RFC 7518 §4.4.
    #[serde(rename = "A256KW")]
    A256Kw,
}

impl KeyAlg {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyAlg::RsaOaep => "RSA-OAEP",
            KeyAlg::RsaOaep256 => "RSA-OAEP-256",
            KeyAlg::Dir => "dir",
            KeyAlg::A128Kw => "A128KW",
            KeyAlg::A256Kw => "A256KW",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "RSA-OAEP" => KeyAlg::RsaOaep,
            "RSA-OAEP-256" => KeyAlg::RsaOaep256,
            "dir" => KeyAlg::Dir,
            "A128KW" => KeyAlg::A128Kw,
            "A256KW" => KeyAlg::A256Kw,
            _ => return None,
        })
    }
}

/// Content-encryption algorithms supported by `cave-auth`.
///
/// Maps onto the `enc` Header Parameter (RFC 7516 §4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncAlg {
    /// AES-128 in GCM mode — RFC 7518 §5.3.
    #[serde(rename = "A128GCM")]
    A128Gcm,
    /// AES-256 in GCM mode — RFC 7518 §5.3.
    #[serde(rename = "A256GCM")]
    A256Gcm,
    /// AES-128-CBC + HMAC-SHA256 — RFC 7518 §5.2.3.
    #[serde(rename = "A128CBC-HS256")]
    A128CbcHs256,
    /// AES-256-CBC + HMAC-SHA512 — RFC 7518 §5.2.5.
    #[serde(rename = "A256CBC-HS512")]
    A256CbcHs512,
}

impl EncAlg {
    pub fn as_str(&self) -> &'static str {
        match self {
            EncAlg::A128Gcm => "A128GCM",
            EncAlg::A256Gcm => "A256GCM",
            EncAlg::A128CbcHs256 => "A128CBC-HS256",
            EncAlg::A256CbcHs512 => "A256CBC-HS512",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "A128GCM" => EncAlg::A128Gcm,
            "A256GCM" => EncAlg::A256Gcm,
            "A128CBC-HS256" => EncAlg::A128CbcHs256,
            "A256CBC-HS512" => EncAlg::A256CbcHs512,
            _ => return None,
        })
    }

    /// CEK bit-length (RFC 7518 §5.3 + §5.2).
    pub fn cek_bits(&self) -> usize {
        match self {
            EncAlg::A128Gcm => 128,
            EncAlg::A256Gcm => 256,
            EncAlg::A128CbcHs256 => 256, // 128-bit AES + 128-bit HMAC key
            EncAlg::A256CbcHs512 => 512, // 256-bit AES + 256-bit HMAC key
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JweError {
    #[error("malformed JWE: {0}")]
    Malformed(&'static str),
    #[error("base64 decode failed: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("unsupported alg: {0}")]
    UnsupportedAlg(String),
    #[error("unsupported enc: {0}")]
    UnsupportedEnc(String),
    #[error("decryption failed (authentication tag mismatch)")]
    AuthFailed,
    #[error("RSA error: {0}")]
    Rsa(String),
    #[error("AES key-wrap error: {0}")]
    AesKw(String),
    #[error("invalid key length")]
    InvalidKeyLength,
    #[error("invalid IV length")]
    InvalidIvLength,
    #[error("JSON header parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("utf8 error in header: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

/// Protected header — fields are flat-mapped (RFC 7516 §4.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedHeader {
    pub alg: KeyAlg,
    pub enc: EncAlg,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typ: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cty: Option<String>,
}

impl ProtectedHeader {
    pub fn new(alg: KeyAlg, enc: EncAlg) -> Self {
        Self { alg, enc, kid: None, typ: None, cty: None }
    }
}

/// Key material used at encryption / decryption time. For asymmetric algorithms
/// only the matching half is meaningful.
#[derive(Clone)]
pub enum KeyInput {
    /// Symmetric direct-CEK or AES-KW key.
    Symmetric(Vec<u8>),
    /// RSA public key for encryption.
    RsaPublic(rsa::RsaPublicKey),
    /// RSA private key for decryption.
    RsaPrivate(rsa::RsaPrivateKey),
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc7516 §4.1.1 — alg names are case-sensitive strings.
    #[test]
    fn keyalg_round_trips_through_string() {
        for k in [KeyAlg::RsaOaep, KeyAlg::RsaOaep256, KeyAlg::Dir, KeyAlg::A128Kw, KeyAlg::A256Kw] {
            assert_eq!(KeyAlg::from_str(k.as_str()).unwrap(), k);
        }
    }

    // upstream: rfc7516 §4.1.2 — enc names are case-sensitive strings.
    #[test]
    fn encalg_round_trips_through_string() {
        for e in [EncAlg::A128Gcm, EncAlg::A256Gcm, EncAlg::A128CbcHs256, EncAlg::A256CbcHs512] {
            assert_eq!(EncAlg::from_str(e.as_str()).unwrap(), e);
        }
    }

    // upstream: rfc7518 §5.3 — A128GCM uses a 128-bit CEK.
    // upstream: rfc7518 §5.2.3 — A128CBC-HS256 uses a 256-bit CEK (128 AES + 128 MAC).
    #[test]
    fn enc_cek_bits_matches_rfc7518() {
        assert_eq!(EncAlg::A128Gcm.cek_bits(), 128);
        assert_eq!(EncAlg::A256Gcm.cek_bits(), 256);
        assert_eq!(EncAlg::A128CbcHs256.cek_bits(), 256);
        assert_eq!(EncAlg::A256CbcHs512.cek_bits(), 512);
    }

    // upstream: rfc7516 §4.1.1 — unsupported alg values must round-trip None.
    #[test]
    fn keyalg_unknown_returns_none() {
        assert!(KeyAlg::from_str("RSA1_5").is_none());
        assert!(KeyAlg::from_str("ECDH-ES").is_none());
        assert!(EncAlg::from_str("A192GCM").is_none());
    }
}
