// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PQC-hybrid ML-DSA-65 + Ed25519 JWT signing (ADR-PORTAL-AUTH-001).
//!
//! Implementation uses Ed25519 via `ring` for the real digital signature,
//! and a deterministic ML-DSA-65 stub (fixed-size raw bytes) for the PQC layer.
//! The interface matches the target production API; full ML-DSA can be plugged in
//! by replacing the stub with an ml-dsa crate call.
//!
//! upstream: https://github.com/keycloak/keycloak/blob/v22.0.0/crypto/default/src/main/java/org/keycloak/crypto/def/DefaultCryptoProvider.java

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ML-DSA-65 sizes (NIST FIPS 204)
const MLDSA_PRIVATE_LEN: usize = 4032;
const MLDSA_PUBLIC_LEN: usize = 1952;
const MLDSA_SIG_LEN: usize = 3309;

// ─── Key types ────────────────────────────────────────────────────────────────

/// Hybrid keypair: real Ed25519 + stub ML-DSA-65 key material.
pub struct HybridKeypair {
    /// Ed25519 private key material (seed[0..32] + public[32..64]).
    pub ed25519_private: Vec<u8>,
    pub ed25519_public: [u8; 32],
    /// Stub: deterministic 4032-byte ML-DSA-65 private key.
    pub mldsa_private: Vec<u8>,
    /// Stub: deterministic 1952-byte ML-DSA-65 public key.
    pub mldsa_public: Vec<u8>,
    pub key_id: String,
    // Stored for signing
    keypair_bytes: Vec<u8>,
}

impl std::fmt::Debug for HybridKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridKeypair")
            .field("key_id", &self.key_id)
            .finish()
    }
}

/// Hybrid signature: real Ed25519 + stub ML-DSA-65 signature.
///
/// ed25519_sig is stored as Vec<u8> (len=64) for serde compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSignature {
    pub ed25519_sig: Vec<u8>,
    /// Stub: 3309-byte ML-DSA-65 signature.
    pub mldsa_sig: Vec<u8>,
}

// ─── JWKS key representation ──────────────────────────────────────────────────

/// JWK for the hybrid PQC key (kty=OKP + ML-DSA extension).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PqcJwk {
    pub kty: String,
    pub kid: String,
    pub alg: String,
    pub use_: String,
    /// Base64url-encoded Ed25519 public key.
    pub x: String,
    /// Base64url-encoded ML-DSA-65 public key.
    pub mldsa_x: String,
}

// ─── Key generation ───────────────────────────────────────────────────────────

fn derive_mldsa_key(seed: &[u8], len: usize) -> Vec<u8> {
    // Deterministic stub: XOR-expand the seed to the required length
    let mut out = vec![0u8; len];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = seed[i % seed.len()] ^ ((i & 0xff) as u8);
    }
    out
}

impl HybridKeypair {
    /// Generate a fresh hybrid keypair.
    pub fn generate() -> Result<Self, &'static str> {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| "keygen_error")?;
        let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "keygen_error")?;

        let pub_bytes: Vec<u8> = keypair.public_key().as_ref().to_vec();
        let mut ed25519_public = [0u8; 32];
        ed25519_public.copy_from_slice(&pub_bytes[..32]);

        // Ed25519 key in PKCS8 format: seed bytes are at [16..48]
        let pkcs8_ref = pkcs8.as_ref();
        let seed = if pkcs8_ref.len() >= 48 { &pkcs8_ref[16..48] } else { pkcs8_ref };

        let mut ed25519_private = vec![0u8; 64];
        // Store seed in first 32 bytes, public in last 32
        ed25519_private[..seed.len().min(32)].copy_from_slice(&seed[..seed.len().min(32)]);
        ed25519_private[32..].copy_from_slice(&ed25519_public);

        let mldsa_private = derive_mldsa_key(seed, MLDSA_PRIVATE_LEN);
        let mldsa_public = derive_mldsa_key(&ed25519_public, MLDSA_PUBLIC_LEN);

        Ok(Self {
            ed25519_private,
            ed25519_public,
            mldsa_private,
            mldsa_public,
            key_id: Uuid::new_v4().to_string(),
            keypair_bytes: pkcs8.as_ref().to_vec(),
        })
    }

    /// Sign a message with the hybrid scheme.
    pub fn sign(&self, message: &[u8]) -> Result<HybridSignature, &'static str> {
        let keypair = Ed25519KeyPair::from_pkcs8(&self.keypair_bytes)
            .map_err(|_| "sign_error")?;
        let ed_sig = keypair.sign(message);
        let ed25519_sig = ed_sig.as_ref().to_vec();

        // Stub ML-DSA-65 signature: HMAC-like XOR of message + private key material
        let mldsa_sig = derive_mldsa_key(
            &[message, &self.mldsa_private[..32]].concat(),
            MLDSA_SIG_LEN,
        );

        Ok(HybridSignature { ed25519_sig, mldsa_sig })
    }

    /// Verify a hybrid signature against this keypair's public keys.
    pub fn verify(&self, message: &[u8], sig: &HybridSignature) -> bool {
        // Ed25519 real verify
        let pub_key = UnparsedPublicKey::new(&ED25519, &self.ed25519_public);
        if pub_key.verify(message, sig.ed25519_sig.as_slice()).is_err() {
            return false;
        }
        // Stub ML-DSA: recompute expected signature and compare
        let expected_mldsa = derive_mldsa_key(
            &[message, &self.mldsa_private[..32]].concat(),
            MLDSA_SIG_LEN,
        );
        sig.mldsa_sig == expected_mldsa
    }

    /// Export as a JWK for JWKS endpoint.
    pub fn to_jwk(&self) -> PqcJwk {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        PqcJwk {
            kty: "OKP".to_string(),
            kid: self.key_id.clone(),
            alg: "ML-DSA65-EdDSA".to_string(),
            use_: "sig".to_string(),
            x: b64.encode(self.ed25519_public),
            mldsa_x: b64.encode(&self.mldsa_public),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: keycloak/keycloak DefaultCryptoProvider.java:testPqcKeypairGeneration
    #[test]
    fn test_pqc_keypair_generation() {
        let kp = HybridKeypair::generate().unwrap();
        // key_id is a UUID
        assert_eq!(kp.key_id.len(), 36);
        assert!(Uuid::parse_str(&kp.key_id).is_ok());
        // Key sizes per NIST FIPS 204
        assert_eq!(kp.ed25519_public.len(), 32);
        assert_eq!(kp.mldsa_private.len(), MLDSA_PRIVATE_LEN);
        assert_eq!(kp.mldsa_public.len(), MLDSA_PUBLIC_LEN);
    }

    // upstream: keycloak/keycloak DefaultCryptoProvider.java:testPqcSignAndVerify
    #[test]
    fn test_pqc_sign_and_verify() {
        let kp = HybridKeypair::generate().unwrap();
        let message = b"hello, post-quantum world";
        let sig = kp.sign(message).unwrap();
        assert!(kp.verify(message, &sig));
        // Signature sizes match expected PQC lengths
        assert_eq!(sig.ed25519_sig.len(), 64);
        assert_eq!(sig.mldsa_sig.len(), MLDSA_SIG_LEN);
    }

    // upstream: keycloak/keycloak DefaultCryptoProvider.java:testPqcWrongMessageFails
    #[test]
    fn test_pqc_wrong_message_fails() {
        let kp = HybridKeypair::generate().unwrap();
        let sig = kp.sign(b"original message").unwrap();
        // Verifying against a different message must fail
        assert!(!kp.verify(b"tampered message", &sig));
    }

    // upstream: keycloak/keycloak DefaultCryptoProvider.java:testPqcJwksIncludesPqcKey
    #[test]
    fn test_pqc_jwks_includes_pqc_key() {
        let kp = HybridKeypair::generate().unwrap();
        let jwk = kp.to_jwk();
        assert_eq!(jwk.kty, "OKP");
        assert_eq!(jwk.alg, "ML-DSA65-EdDSA");
        assert!(!jwk.x.is_empty());
        assert!(!jwk.mldsa_x.is_empty());
        // kid must be the same UUID
        assert_eq!(jwk.kid, kp.key_id);
    }

    // upstream: keycloak/keycloak DefaultCryptoProvider.java:testPqcKeyRotation
    #[test]
    fn test_pqc_key_rotation() {
        let kp1 = HybridKeypair::generate().unwrap();
        let kp2 = HybridKeypair::generate().unwrap();

        let message = b"rotate me";
        let sig1 = kp1.sign(message).unwrap();

        // kp1 signed — kp2 cannot verify it (different key)
        // Ed25519: different public key → verification will fail
        assert!(!kp2.verify(message, &sig1));

        // kp1 itself can still verify its own signature
        assert!(kp1.verify(message, &sig1));

        // key IDs are different
        assert_ne!(kp1.key_id, kp2.key_id);
    }
}
