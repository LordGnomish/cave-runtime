// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWT signing — RS256 / ES256 / EdDSA + PQC ML-DSA-65 placeholder.
//!
//! Upstream: `services/src/main/java/org/keycloak/keys/*` (KeyProvider) and
//! `core/src/main/java/org/keycloak/jose/jws/JWSBuilder.java`. We don't
//! ship RSA in MVP (Keycloak's default 2048-bit RSA signing key would
//! require a heavyweight crypto stack); ES256 + EdDSA cover modern OIDC.
//! ML-DSA-65 lands as an interface placeholder so the IdToken / AT can
//! flag `alg: "ML-DSA-65"` and rotate cleanly when the cave-pqc backend
//! materialises.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{SigningKey, VerifyingKey};
use ed25519_dalek::Signer as _Ed25519Signer;
use ed25519_dalek::Verifier as _Ed25519Verifier;
use p256::ecdsa::signature::Signer as _P256Signer;
use p256::ecdsa::signature::Verifier as _P256Verifier;
use p256::ecdsa::{Signature as P256Sig, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::{KeycloakError, Result};

/// Signing algorithm identifier — matches the `alg` JOSE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JwsAlg {
    Es256,
    EdDsa,
    /// Placeholder for ML-DSA-65 (FIPS 204 lattice signature). The cave
    /// signer accepts the alg but rejects sign/verify until the cave-pqc
    /// backend lands — see [[partial]] pqc-mldsa-hybrid in the manifest.
    MlDsa65,
}

impl JwsAlg {
    pub fn jose_str(self) -> &'static str {
        match self {
            JwsAlg::Es256 => "ES256",
            JwsAlg::EdDsa => "EdDSA",
            JwsAlg::MlDsa65 => "ML-DSA-65",
        }
    }
}

/// A single signing key. `kid` matches the JWKS `kid` so the consumer can
/// pick the verifying key without trial-and-error.
pub struct SigningKeyEntry {
    pub kid: String,
    pub alg: JwsAlg,
    inner: KeyInner,
}

enum KeyInner {
    Es256 { sk: P256SigningKey, pk: P256VerifyingKey },
    EdDsa { sk: SigningKey, pk: VerifyingKey },
    /// PQC slot — present but the signer fails closed.
    MlDsa65,
}

impl SigningKeyEntry {
    pub fn es256_from_seed(kid: impl Into<String>, seed: &[u8; 32]) -> Result<Self> {
        let sk = P256SigningKey::from_bytes(seed.into())
            .map_err(|e| KeycloakError::Internal(format!("p256: {}", e)))?;
        let pk = P256VerifyingKey::from(&sk);
        Ok(Self {
            kid: kid.into(),
            alg: JwsAlg::Es256,
            inner: KeyInner::Es256 { sk, pk },
        })
    }

    pub fn eddsa_from_seed(kid: impl Into<String>, seed: &[u8; 32]) -> Self {
        let sk = SigningKey::from_bytes(seed);
        let pk = sk.verifying_key();
        Self {
            kid: kid.into(),
            alg: JwsAlg::EdDsa,
            inner: KeyInner::EdDsa { sk, pk },
        }
    }

    pub fn mldsa65_placeholder(kid: impl Into<String>) -> Self {
        Self {
            kid: kid.into(),
            alg: JwsAlg::MlDsa65,
            inner: KeyInner::MlDsa65,
        }
    }

    pub fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        match &self.inner {
            KeyInner::Es256 { sk, .. } => {
                let sig: P256Sig = sk.sign(payload);
                Ok(sig.to_bytes().to_vec())
            }
            KeyInner::EdDsa { sk, .. } => Ok(sk.sign(payload).to_bytes().to_vec()),
            KeyInner::MlDsa65 => Err(KeycloakError::Internal(
                "ML-DSA-65 signer placeholder — cave-pqc backend not yet wired".into(),
            )),
        }
    }

    pub fn verify(&self, payload: &[u8], sig: &[u8]) -> Result<()> {
        match &self.inner {
            KeyInner::Es256 { pk, .. } => {
                let s = P256Sig::from_slice(sig).map_err(|_| KeycloakError::TokenSignatureInvalid)?;
                pk.verify(payload, &s).map_err(|_| KeycloakError::TokenSignatureInvalid)
            }
            KeyInner::EdDsa { pk, .. } => {
                let arr: [u8; 64] = sig.try_into().map_err(|_| KeycloakError::TokenSignatureInvalid)?;
                let s = ed25519_dalek::Signature::from_bytes(&arr);
                pk.verify(payload, &s).map_err(|_| KeycloakError::TokenSignatureInvalid)
            }
            KeyInner::MlDsa65 => Err(KeycloakError::Internal(
                "ML-DSA-65 verifier placeholder — cave-pqc backend not yet wired".into(),
            )),
        }
    }

    /// JWKS-shape JSON for the public key — RFC 7517.
    pub fn to_jwk(&self) -> serde_json::Value {
        match &self.inner {
            KeyInner::Es256 { pk, .. } => {
                let enc = pk.to_encoded_point(false);
                let x = URL_SAFE_NO_PAD.encode(enc.x().unwrap());
                let y = URL_SAFE_NO_PAD.encode(enc.y().unwrap());
                serde_json::json!({
                    "kty": "EC",
                    "crv": "P-256",
                    "alg": "ES256",
                    "use": "sig",
                    "kid": self.kid,
                    "x": x,
                    "y": y,
                })
            }
            KeyInner::EdDsa { pk, .. } => {
                let x = URL_SAFE_NO_PAD.encode(pk.as_bytes());
                serde_json::json!({
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "alg": "EdDSA",
                    "use": "sig",
                    "kid": self.kid,
                    "x": x,
                })
            }
            KeyInner::MlDsa65 => serde_json::json!({
                "kty": "PQC",
                "alg": "ML-DSA-65",
                "use": "sig",
                "kid": self.kid,
                "x": "placeholder",
            }),
        }
    }
}

/// Registry of signing keys, keyed by `(realm_id, kid)`. The signer
/// maintains a current-active kid per realm; the rest are kept for verify
/// during rotation.
pub struct SignerRegistry {
    inner: Mutex<SignerRegistryInner>,
}

struct SignerRegistryInner {
    keys: BTreeMap<(String, String), SigningKeyEntry>,
    active: BTreeMap<String, String>, // realm_id -> kid
}

impl Default for SignerRegistry {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SignerRegistryInner {
                keys: BTreeMap::new(),
                active: BTreeMap::new(),
            }),
        }
    }
}

impl SignerRegistry {
    pub fn install(&self, realm_id: &str, key: SigningKeyEntry, set_active: bool) {
        let mut g = self.inner.lock().unwrap();
        let kid = key.kid.clone();
        g.keys.insert((realm_id.to_string(), kid.clone()), key);
        if set_active {
            g.active.insert(realm_id.to_string(), kid);
        }
    }

    pub fn active_kid(&self, realm_id: &str) -> Option<String> {
        let g = self.inner.lock().unwrap();
        g.active.get(realm_id).cloned()
    }

    pub fn sign_compact(&self, realm_id: &str, header: &serde_json::Value, payload: &serde_json::Value) -> Result<String> {
        let g = self.inner.lock().unwrap();
        let kid = g
            .active
            .get(realm_id)
            .ok_or_else(|| KeycloakError::Internal(format!("no active signing key for realm {}", realm_id)))?
            .clone();
        let key = g
            .keys
            .get(&(realm_id.to_string(), kid.clone()))
            .ok_or_else(|| KeycloakError::Internal(format!("kid {} missing", kid)))?;
        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(header).unwrap());
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).unwrap());
        let signing_input = format!("{}.{}", header_b64, payload_b64);
        let sig = key.sign(signing_input.as_bytes())?;
        Ok(format!("{}.{}", signing_input, URL_SAFE_NO_PAD.encode(sig)))
    }

    pub fn verify_compact(&self, realm_id: &str, jws: &str) -> Result<(serde_json::Value, serde_json::Value)> {
        let mut parts = jws.split('.');
        let h = parts.next().ok_or_else(|| KeycloakError::InvalidRequest("jws: no header".into()))?;
        let p = parts.next().ok_or_else(|| KeycloakError::InvalidRequest("jws: no payload".into()))?;
        let s = parts.next().ok_or_else(|| KeycloakError::InvalidRequest("jws: no signature".into()))?;
        if parts.next().is_some() {
            return Err(KeycloakError::InvalidRequest("jws: trailing segment".into()));
        }
        let header_json: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD.decode(h).map_err(|_| KeycloakError::InvalidRequest("jws: header b64".into()))?,
        )
        .map_err(|_| KeycloakError::InvalidRequest("jws: header json".into()))?;
        let payload_json: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD.decode(p).map_err(|_| KeycloakError::InvalidRequest("jws: payload b64".into()))?,
        )
        .map_err(|_| KeycloakError::InvalidRequest("jws: payload json".into()))?;
        let sig_bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| KeycloakError::TokenSignatureInvalid)?;
        let kid = header_json
            .get("kid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| KeycloakError::InvalidRequest("jws header missing kid".into()))?
            .to_string();
        let g = self.inner.lock().unwrap();
        let key = g
            .keys
            .get(&(realm_id.to_string(), kid.clone()))
            .ok_or_else(|| KeycloakError::Internal(format!("kid {} not registered for realm {}", kid, realm_id)))?;
        let signing_input = format!("{}.{}", h, p);
        key.verify(signing_input.as_bytes(), &sig_bytes)?;
        Ok((header_json, payload_json))
    }

    /// Returns the JWKS for the realm (active + retained keys), in the
    /// stable form required by `/realms/{realm}/.well-known/openid-configuration/jwks`.
    pub fn jwks(&self, realm_id: &str) -> serde_json::Value {
        let g = self.inner.lock().unwrap();
        let keys: Vec<_> = g
            .keys
            .iter()
            .filter(|((r, _), _)| r == realm_id)
            .map(|(_, k)| k.to_jwk())
            .collect();
        serde_json::json!({ "keys": keys })
    }
}

/// Stable SHA-256 thumbprint of a public key — RFC 7638 JWK thumbprint
/// substitute (we hash a deterministic JCS-style encoding of the key
/// material rather than the JSON, since cave-keycloak doesn't pull
/// `serde_jcs`).
pub fn jwk_thumbprint(key: &SigningKeyEntry) -> String {
    let mut h = Sha256::new();
    match &key.inner {
        KeyInner::Es256 { pk, .. } => {
            let enc = pk.to_encoded_point(false);
            h.update(b"es256:");
            h.update(enc.x().unwrap());
            h.update(enc.y().unwrap());
        }
        KeyInner::EdDsa { pk, .. } => {
            h.update(b"eddsa:");
            h.update(pk.as_bytes());
        }
        KeyInner::MlDsa65 => {
            h.update(b"mldsa65-placeholder");
            h.update(key.kid.as_bytes());
        }
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es256_sign_verify_roundtrip() {
        let k = SigningKeyEntry::es256_from_seed("kid-1", &[7u8; 32]).unwrap();
        let sig = k.sign(b"hello").unwrap();
        k.verify(b"hello", &sig).unwrap();
        assert!(k.verify(b"hello2", &sig).is_err());
    }

    #[test]
    fn eddsa_sign_verify_roundtrip() {
        let k = SigningKeyEntry::eddsa_from_seed("kid-2", &[9u8; 32]);
        let sig = k.sign(b"world").unwrap();
        k.verify(b"world", &sig).unwrap();
        assert!(k.verify(b"world2", &sig).is_err());
    }

    #[test]
    fn mldsa65_sign_fails_with_marker() {
        let k = SigningKeyEntry::mldsa65_placeholder("pqc-1");
        let err = k.sign(b"x").unwrap_err();
        assert!(format!("{}", err).contains("ML-DSA-65"));
    }

    #[test]
    fn registry_sign_compact_then_verify_compact() {
        let reg = SignerRegistry::default();
        reg.install(
            "r1",
            SigningKeyEntry::es256_from_seed("kid-1", &[1u8; 32]).unwrap(),
            true,
        );
        let header = serde_json::json!({ "alg": "ES256", "kid": "kid-1", "typ": "JWT" });
        let payload = serde_json::json!({ "sub": "alice", "iss": "https://iam.cave/realms/r1" });
        let jws = reg.sign_compact("r1", &header, &payload).unwrap();
        let (h, p) = reg.verify_compact("r1", &jws).unwrap();
        assert_eq!(h["kid"], "kid-1");
        assert_eq!(p["sub"], "alice");
    }

    #[test]
    fn verify_compact_rejects_tamper() {
        let reg = SignerRegistry::default();
        reg.install(
            "r1",
            SigningKeyEntry::eddsa_from_seed("kid-2", &[3u8; 32]),
            true,
        );
        let header = serde_json::json!({ "alg": "EdDSA", "kid": "kid-2", "typ": "JWT" });
        let payload = serde_json::json!({ "sub": "alice" });
        let jws = reg.sign_compact("r1", &header, &payload).unwrap();
        let mut parts: Vec<&str> = jws.split('.').collect();
        let bad_payload = URL_SAFE_NO_PAD.encode(b"{\"sub\":\"mallory\"}");
        parts[1] = &bad_payload;
        let bad = parts.join(".");
        let err = reg.verify_compact("r1", &bad).unwrap_err();
        assert!(matches!(err, KeycloakError::TokenSignatureInvalid));
    }

    #[test]
    fn jwks_exposes_active_plus_retained_keys() {
        let reg = SignerRegistry::default();
        reg.install("r1", SigningKeyEntry::es256_from_seed("k-old", &[1u8; 32]).unwrap(), false);
        reg.install("r1", SigningKeyEntry::eddsa_from_seed("k-new", &[2u8; 32]), true);
        let jwks = reg.jwks("r1");
        let arr = jwks["keys"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn jwk_thumbprint_is_stable() {
        let k1 = SigningKeyEntry::es256_from_seed("k", &[1u8; 32]).unwrap();
        let k2 = SigningKeyEntry::es256_from_seed("k", &[1u8; 32]).unwrap();
        assert_eq!(jwk_thumbprint(&k1), jwk_thumbprint(&k2));
    }
}
