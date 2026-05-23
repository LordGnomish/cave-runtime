// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Low-level signature primitives — ECDSA P-256 + Ed25519.
//!
//! Maps to:
//!   * pkg/cosign/keys.go       → KeyAlgorithm dispatch
//!   * pkg/signature/ecdsa.go   → sign_p256 / verify_p256
//!   * pkg/signature/ed25519.go → sign_ed25519 / verify_ed25519

use crate::error::{Result, SignError};
use crate::models::KeyAlgorithm;
use base64::Engine;
use ed25519_dalek::{Signer as DalekSigner, SigningKey as Ed25519SigningKey,
    VerifyingKey as Ed25519VerifyingKey, SECRET_KEY_LENGTH as ED25519_SK_LEN,
    Signature as Ed25519Signature, Verifier as DalekVerifier};
#[allow(unused_imports)]
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey,
};
use sha2::{Digest, Sha256};

/// Compute the SHA-256 digest used as the signing payload — cosign signs
/// the hash, not the bytes (matches `pkg/cosign/sign.go`).
pub fn sha256_digest(payload: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(payload);
    h.finalize().to_vec()
}

/// "sha256:<hex>" — the canonical OCI digest string.
pub fn sha256_digest_string(payload: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha256_digest(payload)))
}

/// In-memory keypair. Held in process; never written to disk by this crate.
pub struct Keypair {
    pub algorithm: KeyAlgorithm,
    /// Algorithm-specific secret bytes (P-256: 32-byte scalar BE, Ed25519: 32-byte seed).
    sk: Vec<u8>,
    /// Algorithm-specific public bytes (P-256: SEC1 33-byte compressed, Ed25519: 32-byte y).
    pk: Vec<u8>,
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keypair")
            .field("algorithm", &self.algorithm)
            .field("sk", &"<redacted>")
            .field("pk_b64", &base64::engine::general_purpose::STANDARD.encode(&self.pk))
            .finish()
    }
}

impl Keypair {
    /// Generate a fresh keypair using OS randomness.
    pub fn generate(algorithm: KeyAlgorithm) -> Result<Self> {
        use rand_compat::random_bytes;
        match algorithm {
            KeyAlgorithm::EcdsaP256 => {
                let mut seed = [0u8; 32];
                random_bytes(&mut seed)?;
                let mut sk = P256SigningKey::from_slice(&seed)
                    .map_err(|e| SignError::Key(format!("p256 from seed: {}", e)))?;
                // Retry until non-zero — cryptographically negligible probability.
                let mut tries = 0;
                while sk.to_bytes().iter().all(|b| *b == 0) {
                    random_bytes(&mut seed)?;
                    sk = P256SigningKey::from_slice(&seed)
                        .map_err(|e| SignError::Key(format!("p256 from seed: {}", e)))?;
                    tries += 1;
                    if tries > 4 {
                        return Err(SignError::Key("rng returned zeros".into()));
                    }
                }
                let pk = sk.verifying_key().to_sec1_bytes().to_vec();
                Ok(Self {
                    algorithm,
                    sk: sk.to_bytes().to_vec(),
                    pk,
                })
            }
            KeyAlgorithm::Ed25519 => {
                let mut seed = [0u8; ED25519_SK_LEN];
                random_bytes(&mut seed)?;
                let sk = Ed25519SigningKey::from_bytes(&seed);
                let pk = sk.verifying_key().to_bytes().to_vec();
                Ok(Self {
                    algorithm,
                    sk: seed.to_vec(),
                    pk,
                })
            }
        }
    }

    /// Deterministic constructor from seed — used in tests + key-import.
    pub fn from_seed(algorithm: KeyAlgorithm, seed: &[u8; 32]) -> Result<Self> {
        match algorithm {
            KeyAlgorithm::EcdsaP256 => {
                let sk = P256SigningKey::from_slice(seed)
                    .map_err(|e| SignError::Key(format!("p256 from seed: {}", e)))?;
                if sk.to_bytes().iter().all(|b| *b == 0) {
                    return Err(SignError::Key("zero scalar".into()));
                }
                let pk = sk.verifying_key().to_sec1_bytes().to_vec();
                Ok(Self {
                    algorithm,
                    sk: sk.to_bytes().to_vec(),
                    pk,
                })
            }
            KeyAlgorithm::Ed25519 => {
                let sk = Ed25519SigningKey::from_bytes(seed);
                let pk = sk.verifying_key().to_bytes().to_vec();
                Ok(Self {
                    algorithm,
                    sk: seed.to_vec(),
                    pk,
                })
            }
        }
    }

    pub fn public_key_bytes(&self) -> &[u8] {
        &self.pk
    }

    /// Sign the SHA-256 of `payload`, returning raw signature bytes (DER for
    /// P-256, 64-byte concatenation for Ed25519).
    pub fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let digest = sha256_digest(payload);
        match self.algorithm {
            KeyAlgorithm::EcdsaP256 => {
                let sk = P256SigningKey::from_slice(&self.sk)
                    .map_err(|e| SignError::Key(format!("reload p256: {}", e)))?;
                let sig: P256Signature = sk.sign(&digest);
                Ok(sig.to_der().as_bytes().to_vec())
            }
            KeyAlgorithm::Ed25519 => {
                let mut seed = [0u8; ED25519_SK_LEN];
                if self.sk.len() != ED25519_SK_LEN {
                    return Err(SignError::Key("ed25519 sk wrong length".into()));
                }
                seed.copy_from_slice(&self.sk);
                let sk = Ed25519SigningKey::from_bytes(&seed);
                let sig: Ed25519Signature = sk.sign(&digest);
                Ok(sig.to_bytes().to_vec())
            }
        }
    }
}

/// Verify a raw signature against `payload` using the encoded public key
/// bytes returned by `Keypair::public_key_bytes`.
pub fn verify(
    algorithm: KeyAlgorithm,
    public_key: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> Result<()> {
    let digest = sha256_digest(payload);
    match algorithm {
        KeyAlgorithm::EcdsaP256 => {
            let vk = P256VerifyingKey::from_sec1_bytes(public_key)
                .map_err(|e| SignError::Key(format!("p256 vk parse: {}", e)))?;
            let sig = P256Signature::from_der(signature)
                .map_err(|e| SignError::InvalidSignature(format!("p256 sig der: {}", e)))?;
            vk.verify(&digest, &sig)
                .map_err(|_| SignError::Verify("p256 signature mismatch".into()))?;
        }
        KeyAlgorithm::Ed25519 => {
            if public_key.len() != 32 {
                return Err(SignError::Key("ed25519 pk wrong length".into()));
            }
            let mut pk = [0u8; 32];
            pk.copy_from_slice(public_key);
            let vk = Ed25519VerifyingKey::from_bytes(&pk)
                .map_err(|e| SignError::Key(format!("ed25519 vk parse: {}", e)))?;
            if signature.len() != 64 {
                return Err(SignError::InvalidSignature(
                    "ed25519 sig must be 64 bytes".into(),
                ));
            }
            let mut sb = [0u8; 64];
            sb.copy_from_slice(signature);
            let sig = Ed25519Signature::from_bytes(&sb);
            vk.verify(&digest, &sig)
                .map_err(|_| SignError::Verify("ed25519 signature mismatch".into()))?;
        }
    }
    Ok(())
}

/// Internal accessor used by `keypair::encode_private_pem` to read the
/// raw secret seed. Not exported outside the crate.
#[doc(hidden)]
pub(crate) fn __internal_secret_bytes(kp: &Keypair) -> Vec<u8> {
    kp.sk.clone()
}

mod rand_compat {
    use crate::error::{Result, SignError};
    use ring::rand::{SecureRandom, SystemRandom};

    pub fn random_bytes(buf: &mut [u8]) -> Result<()> {
        let rng = SystemRandom::new();
        rng.fill(buf)
            .map_err(|_| SignError::Key("system rng".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p256_roundtrip() {
        let seed = [9u8; 32];
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &seed).unwrap();
        let sig = kp.sign(b"hello cave").unwrap();
        verify(KeyAlgorithm::EcdsaP256, kp.public_key_bytes(), b"hello cave", &sig).unwrap();
    }

    #[test]
    fn p256_tamper_detected() {
        let seed = [7u8; 32];
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &seed).unwrap();
        let sig = kp.sign(b"hello").unwrap();
        let err = verify(KeyAlgorithm::EcdsaP256, kp.public_key_bytes(), b"helLO", &sig)
            .expect_err("tamper must be detected");
        assert!(matches!(err, SignError::Verify(_)));
    }

    #[test]
    fn ed25519_roundtrip() {
        let seed = [3u8; 32];
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &seed).unwrap();
        let sig = kp.sign(b"hello cave").unwrap();
        assert_eq!(sig.len(), 64);
        verify(KeyAlgorithm::Ed25519, kp.public_key_bytes(), b"hello cave", &sig).unwrap();
    }

    #[test]
    fn ed25519_tamper_detected() {
        let seed = [5u8; 32];
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &seed).unwrap();
        let mut sig = kp.sign(b"hello").unwrap();
        sig[0] ^= 0x01;
        let err = verify(KeyAlgorithm::Ed25519, kp.public_key_bytes(), b"hello", &sig)
            .expect_err("must be invalid");
        assert!(matches!(err, SignError::Verify(_)));
    }

    #[test]
    fn cross_algorithm_rejects() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[1u8; 32]).unwrap();
        let sig = kp.sign(b"x").unwrap();
        let p256 = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        // Wrong public key + wrong-length sig must yield error, not panic.
        let err = verify(KeyAlgorithm::Ed25519, p256.public_key_bytes(), b"x", &sig);
        assert!(err.is_err());
    }

    #[test]
    fn sha256_digest_known_vector() {
        // Empty input → known SHA-256.
        let d = sha256_digest_string(b"");
        assert_eq!(
            d,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn generate_yields_nonzero_pk() {
        let kp = Keypair::generate(KeyAlgorithm::EcdsaP256).unwrap();
        assert!(kp.public_key_bytes().iter().any(|b| *b != 0));
        let kp2 = Keypair::generate(KeyAlgorithm::Ed25519).unwrap();
        assert!(kp2.public_key_bytes().iter().any(|b| *b != 0));
    }

    #[test]
    fn deterministic_from_seed_p256() {
        let a = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[42u8; 32]).unwrap();
        let b = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[42u8; 32]).unwrap();
        assert_eq!(a.public_key_bytes(), b.public_key_bytes());
    }

    #[test]
    fn debug_redacts_secret() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[8u8; 32]).unwrap();
        let s = format!("{:?}", kp);
        assert!(s.contains("redacted"));
        assert!(!s.contains("0x08"));
    }
}
