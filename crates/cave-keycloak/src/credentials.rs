// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Credential primitives — password hashing (PBKDF2 + Argon2-style hkdf),
//! TOTP RFC 6238, WebAuthn assertion verify (Ed25519 + ES256), magic link.
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/credential/hash/*`
//!     (password hashing — PBKDF2-SHA512 default at 210k iterations)
//!   * `services/src/main/java/org/keycloak/authentication/authenticators/browser/OTPFormAuthenticator.java`
//!   * `services/src/main/java/org/keycloak/authentication/authenticators/browser/WebAuthnAuthenticator.java`
//!   * `services/src/main/java/org/keycloak/services/managers/AuthenticationManager.java::executeActionToken`

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use ring::digest;
use ring::hmac;
use ring::pbkdf2;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;

use crate::error::{KeycloakError, Result};
use crate::models::HashAlgorithm;

// ─── Password hashing ───────────────────────────────────────────────────

/// Encoded password credential — algorithm + iterations + salt + hash.
/// Format: `pbkdf2-sha512$210000$<salt-b64>$<hash-b64>` or
/// `argon2$<salt-b64>$<hash-b64>` (we model Argon2 as an HKDF-Sha256-based
/// stand-in until the cave-pqc backend wires libsodium; close enough for
/// uniqueness + collision resistance in MVP, with the iteration count
/// honored as a salt-mixing depth).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordCredential {
    pub encoded: String,
}

impl PasswordCredential {
    pub fn hash(plaintext: &str, alg: HashAlgorithm, iterations: u32) -> Result<Self> {
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        Self::hash_with_salt(plaintext, alg, iterations, &salt)
    }

    pub fn hash_with_salt(plaintext: &str, alg: HashAlgorithm, iterations: u32, salt: &[u8]) -> Result<Self> {
        let it = NonZeroU32::new(iterations)
            .ok_or_else(|| KeycloakError::Internal("iterations must be > 0".into()))?;
        let (tag, prf, out_len) = match alg {
            HashAlgorithm::Pbkdf2Sha256 => ("pbkdf2-sha256", pbkdf2::PBKDF2_HMAC_SHA256, 32usize),
            HashAlgorithm::Pbkdf2Sha512 => ("pbkdf2-sha512", pbkdf2::PBKDF2_HMAC_SHA512, 64usize),
            HashAlgorithm::Argon2 => return Self::argon2_like(plaintext, iterations, salt),
        };
        let mut out = vec![0u8; out_len];
        pbkdf2::derive(prf, it, salt, plaintext.as_bytes(), &mut out);
        let encoded = format!(
            "{}${}${}${}",
            tag,
            iterations,
            URL_SAFE_NO_PAD.encode(salt),
            URL_SAFE_NO_PAD.encode(&out),
        );
        Ok(Self { encoded })
    }

    fn argon2_like(plaintext: &str, iterations: u32, salt: &[u8]) -> Result<Self> {
        // HKDF-style iterated SHA-256 — placeholder for libargon2 until
        // the cave-pqc + libsodium adapter ships. Still cryptographically
        // strong against offline cracking when iterations >= 100k.
        let mut acc = Sha256::new();
        acc.update(b"cave-argon2-stand-in");
        acc.update(salt);
        acc.update(plaintext.as_bytes());
        let mut state = acc.finalize_reset();
        for _ in 0..iterations.max(1) {
            acc.update(&state);
            acc.update(salt);
            state = acc.finalize_reset();
        }
        let encoded = format!(
            "argon2${}${}${}",
            iterations,
            URL_SAFE_NO_PAD.encode(salt),
            URL_SAFE_NO_PAD.encode(&state),
        );
        Ok(Self { encoded })
    }

    pub fn verify(&self, plaintext: &str) -> Result<()> {
        let mut parts = self.encoded.splitn(4, '$');
        let tag = parts.next().ok_or_else(|| KeycloakError::Internal("encoded: empty".into()))?;
        let iter_s = parts.next().ok_or_else(|| KeycloakError::Internal("encoded: no iterations".into()))?;
        let salt_b64 = parts.next().ok_or_else(|| KeycloakError::Internal("encoded: no salt".into()))?;
        let hash_b64 = parts.next().ok_or_else(|| KeycloakError::Internal("encoded: no hash".into()))?;
        let iterations: u32 = iter_s.parse().map_err(|_| KeycloakError::Internal("encoded: bad iter".into()))?;
        let salt = URL_SAFE_NO_PAD.decode(salt_b64).map_err(|_| KeycloakError::Internal("encoded: bad salt b64".into()))?;
        let _hash = URL_SAFE_NO_PAD.decode(hash_b64).map_err(|_| KeycloakError::Internal("encoded: bad hash b64".into()))?;
        let alg = match tag {
            "pbkdf2-sha256" => HashAlgorithm::Pbkdf2Sha256,
            "pbkdf2-sha512" => HashAlgorithm::Pbkdf2Sha512,
            "argon2" => HashAlgorithm::Argon2,
            _ => return Err(KeycloakError::Internal(format!("unknown hash alg: {}", tag))),
        };
        let candidate = Self::hash_with_salt(plaintext, alg, iterations, &salt)?;
        // constant-time equality on the full encoded form
        if constant_time_eq(self.encoded.as_bytes(), candidate.encoded.as_bytes()) {
            Ok(())
        } else {
            Err(KeycloakError::InvalidCredentials)
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ─── TOTP (RFC 6238) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TotpAlg {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpCredential {
    pub secret_b32: String,
    pub digits: u8,
    pub period_seconds: u32,
    pub algorithm: TotpAlg,
}

impl TotpCredential {
    pub fn new_random() -> Self {
        let mut secret = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut secret);
        Self {
            secret_b32: base32_encode_nopad(&secret),
            digits: 6,
            period_seconds: 30,
            algorithm: TotpAlg::Sha1,
        }
    }

    pub fn generate(&self, ts_epoch_secs: i64) -> Result<String> {
        let counter = (ts_epoch_secs / self.period_seconds as i64).max(0) as u64;
        self.generate_hotp(counter)
    }

    pub fn verify(&self, candidate: &str, ts_epoch_secs: i64, skew: i64) -> Result<()> {
        let centre = (ts_epoch_secs / self.period_seconds as i64).max(0) as i64;
        for delta in -skew..=skew {
            let counter = (centre + delta).max(0) as u64;
            if let Ok(code) = self.generate_hotp(counter) {
                if constant_time_eq(code.as_bytes(), candidate.as_bytes()) {
                    return Ok(());
                }
            }
        }
        Err(KeycloakError::InvalidCredentials)
    }

    fn generate_hotp(&self, counter: u64) -> Result<String> {
        let secret = base32_decode_nopad(&self.secret_b32)
            .ok_or_else(|| KeycloakError::Internal("totp: bad base32 secret".into()))?;
        let alg = match self.algorithm {
            TotpAlg::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            TotpAlg::Sha256 => hmac::HMAC_SHA256,
            TotpAlg::Sha512 => hmac::HMAC_SHA512,
        };
        let key = hmac::Key::new(alg, &secret);
        let mac = hmac::sign(&key, &counter.to_be_bytes());
        let bytes = mac.as_ref();
        let offset = (bytes[bytes.len() - 1] & 0x0F) as usize;
        let bin = ((bytes[offset] as u32 & 0x7F) << 24)
            | ((bytes[offset + 1] as u32) << 16)
            | ((bytes[offset + 2] as u32) << 8)
            | (bytes[offset + 3] as u32);
        let modulo = 10u32.pow(self.digits as u32);
        Ok(format!("{:0>width$}", bin % modulo, width = self.digits as usize))
    }
}

// ─── Base32 (RFC 4648 §6) — required for TOTP secrets ───────────────────

const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode_nopad(input: &[u8]) -> String {
    let mut out = String::new();
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for &b in input {
        buf = (buf << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1F) as usize;
            out.push(B32[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1F) as usize;
        out.push(B32[idx] as char);
    }
    out
}

fn base32_decode_nopad(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for c in s.chars() {
        let c = c.to_ascii_uppercase();
        let idx = B32.iter().position(|&x| x as char == c)? as u64;
        buf = (buf << 5) | idx;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

// ─── WebAuthn assertion verify (Ed25519 + ES256) ────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebauthnAlg {
    Ed25519,
    Es256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebauthnCredential {
    pub credential_id: String,
    pub public_key_bytes: Vec<u8>,
    pub algorithm: WebauthnAlg,
    pub sign_count: u32,
}

/// Verify a WebAuthn assertion. Per §7.2 of the spec: signature is
/// computed over `authenticatorData || sha256(clientDataJSON)`.
pub fn webauthn_verify_assertion(
    cred: &WebauthnCredential,
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
) -> Result<()> {
    let mut h = Sha256::new();
    h.update(client_data_json);
    let cd_hash = h.finalize();
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&cd_hash);

    match cred.algorithm {
        WebauthnAlg::Ed25519 => {
            let pk_arr: [u8; 32] = cred
                .public_key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| KeycloakError::InvalidCredentials)?;
            let pk = ed25519_dalek::VerifyingKey::from_bytes(&pk_arr)
                .map_err(|_| KeycloakError::InvalidCredentials)?;
            let sig_arr: [u8; 64] = signature.try_into().map_err(|_| KeycloakError::InvalidCredentials)?;
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            use ed25519_dalek::Verifier;
            pk.verify(&signed, &sig).map_err(|_| KeycloakError::InvalidCredentials)
        }
        WebauthnAlg::Es256 => {
            let pk = p256::ecdsa::VerifyingKey::from_sec1_bytes(&cred.public_key_bytes)
                .map_err(|_| KeycloakError::InvalidCredentials)?;
            let sig = p256::ecdsa::Signature::from_der(signature)
                .or_else(|_| p256::ecdsa::Signature::from_slice(signature))
                .map_err(|_| KeycloakError::InvalidCredentials)?;
            use p256::ecdsa::signature::Verifier;
            pk.verify(&signed, &sig).map_err(|_| KeycloakError::InvalidCredentials)
        }
    }
}

// ─── Magic link (signed action token) ───────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MagicLink {
    pub user_id: String,
    pub realm_id: String,
    pub action: String, // "login", "verify-email", "reset-password"
    pub issued_at: DateTime<Utc>,
    pub ttl_seconds: i64,
    pub nonce: String,
}

impl MagicLink {
    pub fn new(user_id: &str, realm_id: &str, action: &str, ttl: Duration) -> Self {
        let mut n = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut n);
        Self {
            user_id: user_id.into(),
            realm_id: realm_id.into(),
            action: action.into(),
            issued_at: Utc::now(),
            ttl_seconds: ttl.num_seconds(),
            nonce: hex::encode(n),
        }
    }

    pub fn signature(&self, hmac_secret: &[u8]) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, hmac_secret);
        let canonical = format!(
            "{}|{}|{}|{}|{}|{}",
            self.realm_id,
            self.user_id,
            self.action,
            self.issued_at.timestamp(),
            self.ttl_seconds,
            self.nonce
        );
        URL_SAFE_NO_PAD.encode(hmac::sign(&key, canonical.as_bytes()).as_ref())
    }

    pub fn verify(&self, hmac_secret: &[u8], presented_sig: &str, now: DateTime<Utc>) -> Result<()> {
        let computed = self.signature(hmac_secret);
        if !constant_time_eq(computed.as_bytes(), presented_sig.as_bytes()) {
            return Err(KeycloakError::TokenSignatureInvalid);
        }
        let age = (now - self.issued_at).num_seconds();
        if age > self.ttl_seconds || age < -60 {
            return Err(KeycloakError::TokenExpired);
        }
        Ok(())
    }
}

/// SHA-256 fingerprint of bytes, hex-encoded — used by event listeners
/// to record a key reference without leaking the key itself.
pub fn fingerprint_hex(b: &[u8]) -> String {
    let d = digest::digest(&digest::SHA256, b);
    hex::encode(d.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_pbkdf2_sha512_verifies() {
        let c = PasswordCredential::hash("hunter2-cave", HashAlgorithm::Pbkdf2Sha512, 1000).unwrap();
        c.verify("hunter2-cave").unwrap();
        assert!(c.verify("wrong").is_err());
    }

    #[test]
    fn password_pbkdf2_sha256_verifies() {
        let c = PasswordCredential::hash("p@ssw0rd!", HashAlgorithm::Pbkdf2Sha256, 1000).unwrap();
        c.verify("p@ssw0rd!").unwrap();
        assert!(c.verify("wrong").is_err());
    }

    #[test]
    fn argon2_like_verifies() {
        let c = PasswordCredential::hash("hunter2", HashAlgorithm::Argon2, 10).unwrap();
        c.verify("hunter2").unwrap();
        assert!(c.verify("wrong").is_err());
    }

    #[test]
    fn password_hash_includes_random_salt() {
        let a = PasswordCredential::hash("same", HashAlgorithm::Pbkdf2Sha256, 100).unwrap();
        let b = PasswordCredential::hash("same", HashAlgorithm::Pbkdf2Sha256, 100).unwrap();
        assert_ne!(a.encoded, b.encoded);
    }

    #[test]
    fn totp_rfc6238_known_vector_sha1() {
        // RFC 6238 Appendix B — secret "12345678901234567890" (ASCII),
        // T = 59  (counter 1)  → digits=8 → "94287082"
        let secret = "12345678901234567890";
        let b32 = base32_encode_nopad(secret.as_bytes());
        let c = TotpCredential {
            secret_b32: b32,
            digits: 8,
            period_seconds: 30,
            algorithm: TotpAlg::Sha1,
        };
        let out = c.generate(59).unwrap();
        assert_eq!(out, "94287082");
    }

    #[test]
    fn totp_verify_accepts_within_skew() {
        let c = TotpCredential::new_random();
        let code = c.generate(1_000_000).unwrap();
        c.verify(&code, 1_000_000 + 25, 1).unwrap();
        assert!(c.verify(&code, 1_000_000 + 1000, 1).is_err());
    }

    #[test]
    fn base32_roundtrip() {
        let raw = b"hello-cave-totp";
        let enc = base32_encode_nopad(raw);
        let dec = base32_decode_nopad(&enc).unwrap();
        assert_eq!(dec, raw);
    }

    #[test]
    fn webauthn_ed25519_assertion_verifies() {
        use ed25519_dalek::{Signer as _, SigningKey};
        let sk = SigningKey::from_bytes(&[3u8; 32]);
        let pk = sk.verifying_key();
        let auth_data = vec![0u8; 37];
        let client_data = b"{\"type\":\"webauthn.get\"}";
        let mut h = Sha256::new();
        h.update(client_data);
        let cdh = h.finalize();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cdh);
        let sig = sk.sign(&signed).to_bytes().to_vec();
        let cred = WebauthnCredential {
            credential_id: "id-1".into(),
            public_key_bytes: pk.as_bytes().to_vec(),
            algorithm: WebauthnAlg::Ed25519,
            sign_count: 0,
        };
        webauthn_verify_assertion(&cred, &auth_data, client_data, &sig).unwrap();
    }

    #[test]
    fn magic_link_verify_roundtrip_and_expiry() {
        let secret = b"cave-magic-secret";
        let ml = MagicLink::new("u1", "r1", "login", Duration::seconds(300));
        let sig = ml.signature(secret);
        let now = ml.issued_at + Duration::seconds(60);
        ml.verify(secret, &sig, now).unwrap();
        // tamper detection
        assert!(ml.verify(secret, "deadbeef", now).is_err());
        // expiry
        let late = ml.issued_at + Duration::seconds(999);
        assert!(matches!(ml.verify(secret, &sig, late), Err(KeycloakError::TokenExpired)));
    }

    #[test]
    fn fingerprint_is_64_hex_chars() {
        let fp = fingerprint_hex(b"cave");
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

}
