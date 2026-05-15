// SPDX-License-Identifier: AGPL-3.0-or-later
//
// RSA-OAEP key encryption — RFC 7518 §4.3.
//
// `RSA-OAEP`     uses SHA-1   + MGF1-SHA-1.
// `RSA-OAEP-256` uses SHA-256 + MGF1-SHA-256.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/jose/jwe/alg/RsaKeyEncryptionJWEAlgorithmProvider.java

use rsa::Oaep;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha1::Sha1;
use sha2::Sha256;

use crate::jwe::JweError;

/// `RSA-OAEP` encrypt — RFC 8017 §7.1.1 with SHA-1.
pub fn encrypt_rsa_oaep(pk: &RsaPublicKey, cek: &[u8]) -> Result<Vec<u8>, JweError> {
    let mut rng = rand::thread_rng();
    pk.encrypt(&mut rng, Oaep::new::<Sha1>(), cek)
        .map_err(|e| JweError::Rsa(e.to_string()))
}

/// `RSA-OAEP` decrypt — RFC 8017 §7.1.2 with SHA-1.
pub fn decrypt_rsa_oaep(sk: &RsaPrivateKey, encrypted_cek: &[u8]) -> Result<Vec<u8>, JweError> {
    sk.decrypt(Oaep::new::<Sha1>(), encrypted_cek)
        .map_err(|e| JweError::Rsa(e.to_string()))
}

/// `RSA-OAEP-256` encrypt — RFC 8017 §7.1.1 with SHA-256.
pub fn encrypt_rsa_oaep_256(pk: &RsaPublicKey, cek: &[u8]) -> Result<Vec<u8>, JweError> {
    let mut rng = rand::thread_rng();
    pk.encrypt(&mut rng, Oaep::new::<Sha256>(), cek)
        .map_err(|e| JweError::Rsa(e.to_string()))
}

/// `RSA-OAEP-256` decrypt — RFC 8017 §7.1.2 with SHA-256.
pub fn decrypt_rsa_oaep_256(sk: &RsaPrivateKey, encrypted_cek: &[u8]) -> Result<Vec<u8>, JweError> {
    sk.decrypt(Oaep::new::<Sha256>(), encrypted_cek)
        .map_err(|e| JweError::Rsa(e.to_string()))
}

// Add a minimal sha1 dep via re-export so we don't need a workspace entry.
// (rsa already pulls sha1 transitively when used via Oaep; we just re-import
// the type.)
use sha1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::RsaPrivateKey;

    fn test_key() -> RsaPrivateKey {
        let mut rng = rand::thread_rng();
        // 2048 bit key — RFC 7518 §4.3 says SHOULD support 2048.
        RsaPrivateKey::new(&mut rng, 2048).expect("key gen")
    }

    // upstream: rfc7518 §4.3 — RSA-OAEP-256 wraps a CEK and the receiver
    // recovers exactly the same CEK.
    #[test]
    fn rsa_oaep_256_round_trip() {
        let sk = test_key();
        let pk = RsaPublicKey::from(&sk);
        let cek: Vec<u8> = (0..32).collect();
        let wrapped = encrypt_rsa_oaep_256(&pk, &cek).unwrap();
        let back = decrypt_rsa_oaep_256(&sk, &wrapped).unwrap();
        assert_eq!(back, cek);
    }

    // upstream: rfc7518 §4.3 — RSA-OAEP (SHA-1) round-trip.
    #[test]
    fn rsa_oaep_sha1_round_trip() {
        let sk = test_key();
        let pk = RsaPublicKey::from(&sk);
        let cek = b"a-128-bit-cek-16";
        let wrapped = encrypt_rsa_oaep(&pk, cek).unwrap();
        let back = decrypt_rsa_oaep(&sk, &wrapped).unwrap();
        assert_eq!(back, cek);
    }

    // upstream: rfc7518 §4.3 — wrapping the same CEK twice MUST produce
    // different ciphertexts (OAEP randomness).
    #[test]
    fn rsa_oaep_256_is_non_deterministic() {
        let sk = test_key();
        let pk = RsaPublicKey::from(&sk);
        let cek: Vec<u8> = (0..32).collect();
        let a = encrypt_rsa_oaep_256(&pk, &cek).unwrap();
        let b = encrypt_rsa_oaep_256(&pk, &cek).unwrap();
        assert_ne!(a, b);
    }

    // upstream: rfc7518 §4.3 — decrypt with a different private key fails.
    #[test]
    fn rsa_oaep_256_wrong_key_fails() {
        let sk = test_key();
        let pk = RsaPublicKey::from(&sk);
        let other = test_key();
        let cek: Vec<u8> = (0..32).collect();
        let wrapped = encrypt_rsa_oaep_256(&pk, &cek).unwrap();
        let err = decrypt_rsa_oaep_256(&other, &wrapped).unwrap_err();
        assert!(matches!(err, JweError::Rsa(_)));
    }
}
