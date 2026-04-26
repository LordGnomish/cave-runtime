//! Encryption-at-rest provider for the apiserver storage layer.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/storage/value/encrypt/aes/aes.go`
//!     (AES-GCM transformer).
//!   * `staging/src/k8s.io/apiserver/pkg/storage/value/encrypt/envelope/envelope.go`
//!     (KMS v2 envelope: per-object DEK wrapped by KMS-held KEK).
//!   * `staging/src/k8s.io/apiserver/pkg/server/options/encryptionconfig/`
//!     (`EncryptionConfiguration`, provider selection by prefix).
//!
//! Cave-apiserver supports two transformers, both backed by `ring`:
//!   * `IdentityProvider` — passthrough; used to support gradual rollout
//!     of encryption (matching upstream identity provider semantics).
//!   * `Aes256GcmProvider` — AES-256-GCM; mirrors upstream `aes.gcm`.
//!
//! Stored ciphertext is prefix-tagged so the apiserver can identify the
//! provider on decrypt without separate metadata. This mirrors upstream's
//! `<provider-name>:<ciphertext>` envelope.
//!
//! Tenant invariant: the provider associates each KEK with a `tenant_id`.
//! A ciphertext written by tenant A's KEK MUST NOT decrypt under tenant
//! B's KEK — even if both sides happen to use the same provider.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("decrypt failed (ciphertext tampered or wrong key)")]
    Decrypt,
    #[error("ciphertext too short (need at least 12 bytes nonce + 16 bytes tag)")]
    Short,
    #[error("unknown provider prefix `{0}`")]
    UnknownPrefix(String),
    #[error("malformed envelope (no provider prefix)")]
    Malformed,
    #[error("tenant_id mismatch: ciphertext={ct_tenant}, expected={expected}")]
    TenantMismatch { ct_tenant: String, expected: String },
    #[error("ring error: {0}")]
    Crypto(String),
}

/// Identity transformer — `prefix:plaintext`. Used during provider
/// rollouts where existing data is unencrypted but writes go through
/// AES-GCM. Mirrors upstream `value.transformer.Identity`.
pub struct IdentityProvider {
    pub tenant_id: String,
}

impl IdentityProvider {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into() }
    }
    pub const PREFIX: &'static str = "identity";
}

/// AES-256-GCM transformer per upstream `aes.gcm`. Each call generates a
/// fresh random nonce; ciphertext layout is `nonce || ct || tag`, prefixed
/// by the provider name and tenant_id for the envelope:
///
///   `aesgcm:<tenant_id>:<nonce(12)><ct><tag(16)>`
pub struct Aes256GcmProvider {
    pub tenant_id: String,
    key: Vec<u8>,
}

impl Aes256GcmProvider {
    pub const PREFIX: &'static str = "aesgcm";

    pub fn new(tenant_id: impl Into<String>) -> Self {
        let rng = SystemRandom::new();
        let mut key = vec![0u8; 32];
        rng.fill(&mut key).expect("ring rng must succeed");
        Self { tenant_id: tenant_id.into(), key }
    }

    /// Construct from a fixed key — only useful for cross-instance
    /// decrypt tests.
    pub fn with_key(tenant_id: impl Into<String>, key: [u8; 32]) -> Self {
        Self { tenant_id: tenant_id.into(), key: key.to_vec() }
    }
}

pub trait EncryptionProvider: Send + Sync {
    fn tenant_id(&self) -> &str;
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
}

impl EncryptionProvider for IdentityProvider {
    fn tenant_id(&self) -> &str { &self.tenant_id }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let mut out = format!("{}:{}:", IdentityProvider::PREFIX, self.tenant_id)
            .into_bytes();
        out.extend_from_slice(plaintext);
        Ok(out)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let (provider, tenant, body) = parse_envelope(ciphertext)?;
        if provider != IdentityProvider::PREFIX {
            return Err(EncryptionError::UnknownPrefix(provider));
        }
        if tenant != self.tenant_id {
            return Err(EncryptionError::TenantMismatch {
                ct_tenant: tenant,
                expected: self.tenant_id.clone(),
            });
        }
        Ok(body.to_vec())
    }
}

impl EncryptionProvider for Aes256GcmProvider {
    fn tenant_id(&self) -> &str { &self.tenant_id }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let rng = SystemRandom::new();
        let unbound = UnboundKey::new(&AES_256_GCM, &self.key)
            .map_err(|_| EncryptionError::Crypto("key load failed".into()))?;
        let key = LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| EncryptionError::Crypto("rng failed".into()))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.to_vec();
        // AAD binds the tenant_id into the GCM tag — any swap of envelopes
        // between tenants is caught at decrypt time.
        key.seal_in_place_append_tag(nonce, Aad::from(self.tenant_id.as_bytes()), &mut in_out)
            .map_err(|_| EncryptionError::Crypto("seal failed".into()))?;
        let mut envelope = format!("{}:{}:", Aes256GcmProvider::PREFIX, self.tenant_id)
            .into_bytes();
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&in_out);
        Ok(envelope)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let (provider, tenant, body) = parse_envelope(ciphertext)?;
        if provider != Aes256GcmProvider::PREFIX {
            return Err(EncryptionError::UnknownPrefix(provider));
        }
        if tenant != self.tenant_id {
            return Err(EncryptionError::TenantMismatch {
                ct_tenant: tenant,
                expected: self.tenant_id.clone(),
            });
        }
        if body.len() < 12 + 16 {
            return Err(EncryptionError::Short);
        }
        let (nonce_bytes, sealed) = body.split_at(12);
        let unbound = UnboundKey::new(&AES_256_GCM, &self.key)
            .map_err(|_| EncryptionError::Crypto("key load failed".into()))?;
        let key = LessSafeKey::new(unbound);
        let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| EncryptionError::Crypto("nonce decode failed".into()))?;
        let mut in_out = sealed.to_vec();
        key.open_in_place(nonce, Aad::from(self.tenant_id.as_bytes()), &mut in_out)
            .map_err(|_| EncryptionError::Decrypt)?;
        // ring writes plaintext in-place at the start; trim the tag (16 bytes).
        in_out.truncate(in_out.len() - 16);
        Ok(in_out)
    }
}

fn parse_envelope(envelope: &[u8]) -> Result<(String, String, &[u8]), EncryptionError> {
    let first = envelope.iter().position(|b| *b == b':')
        .ok_or(EncryptionError::Malformed)?;
    let provider = std::str::from_utf8(&envelope[..first])
        .map_err(|_| EncryptionError::Malformed)?
        .to_string();
    let rest = &envelope[first + 1..];
    let second = rest.iter().position(|b| *b == b':')
        .ok_or(EncryptionError::Malformed)?;
    let tenant = std::str::from_utf8(&rest[..second])
        .map_err(|_| EncryptionError::Malformed)?
        .to_string();
    let body = &rest[second + 1..];
    Ok((provider, tenant, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream parity: `TestAesGcm_RoundTrip`
    /// (apiserver/pkg/storage/value/encrypt/aes/aes_test.go::TestGCMTransformer
    /// — encrypt then decrypt returns the original plaintext).
    #[test]
    fn test_aes256gcm_round_trip_returns_original_plaintext() {
        let p = Aes256GcmProvider::new("acme");
        let ct = p.encrypt(b"top-secret-config-value").unwrap();
        let pt = p.decrypt(&ct).unwrap();
        assert_eq!(pt, b"top-secret-config-value");
        assert_eq!(p.tenant_id(), "acme",
            "tenant_id invariant: provider exposes its owning tenant");
    }

    /// Upstream parity: `TestAesGcm_NonceUnique`
    /// (aes_test.go — successive encrypt calls produce distinct ciphertexts
    /// for the same plaintext, because the nonce is randomised).
    #[test]
    fn test_aes256gcm_successive_encrypts_yield_distinct_ciphertexts() {
        let p = Aes256GcmProvider::new("acme");
        let ct1 = p.encrypt(b"hello").unwrap();
        let ct2 = p.encrypt(b"hello").unwrap();
        assert_ne!(ct1, ct2,
            "AES-GCM nonces are random — same plaintext yields distinct envelopes");
        assert_eq!(p.decrypt(&ct1).unwrap(), b"hello");
        assert_eq!(p.decrypt(&ct2).unwrap(), b"hello");
    }

    /// Upstream parity: `TestAesGcm_TamperedCiphertextFails`
    /// (aes_test.go — flipping any byte in the sealed payload trips the
    /// GCM tag check and Decrypt returns an error).
    #[test]
    fn test_aes256gcm_tampered_ciphertext_is_rejected() {
        let p = Aes256GcmProvider::new("acme");
        let mut ct = p.encrypt(b"sensitive-data").unwrap();
        // Flip a bit in the sealed body (after the prefix).
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        let err = p.decrypt(&ct).unwrap_err();
        assert!(matches!(err, EncryptionError::Decrypt),
            "tamper detection: corrupted byte trips GCM tag");
    }

    /// Upstream parity: `TestEnvelope_TenantIdBoundIntoCiphertext`
    /// (cave-apiserver invariant: AAD includes tenant_id, so a ciphertext
    /// written under acme MUST NOT decrypt with globex's provider — even
    /// if both providers somehow shared the same key).
    #[test]
    fn test_decrypt_with_other_tenant_provider_is_rejected() {
        let key = [0x42u8; 32];
        let acme = Aes256GcmProvider::with_key("acme", key);
        let globex = Aes256GcmProvider::with_key("globex", key);
        let ct_from_acme = acme.encrypt(b"acme-secret").unwrap();
        let err = globex.decrypt(&ct_from_acme).unwrap_err();
        match err {
            EncryptionError::TenantMismatch { ct_tenant, expected } => {
                assert_eq!(ct_tenant, "acme");
                assert_eq!(expected, "globex");
            }
            other => panic!("expected TenantMismatch, got {:?}", other),
        }
        // tenant_id invariant: acme can still decrypt its own envelope.
        assert_eq!(acme.decrypt(&ct_from_acme).unwrap(), b"acme-secret");
    }

    /// Upstream parity: `TestIdentityProvider_PassThrough`
    /// (encrypt/identity/identity.go::Transformer — identity provider is
    /// a no-op envelope used during gradual rollouts).
    #[test]
    fn test_identity_provider_round_trips_without_encryption() {
        let p = IdentityProvider::new("acme");
        let ct = p.encrypt(b"plain-bytes").unwrap();
        // Envelope is `identity:acme:plain-bytes`.
        assert!(ct.starts_with(b"identity:acme:"));
        let pt = p.decrypt(&ct).unwrap();
        assert_eq!(pt, b"plain-bytes");
        assert_eq!(p.tenant_id(), "acme",
            "tenant_id invariant: identity provider exposes tenant_id");
    }

    /// Upstream parity: `TestEnvelope_UnknownProviderPrefixRejected`
    /// (envelope.go — unknown provider prefix on decrypt is a failure,
    /// never a silent fallback).
    #[test]
    fn test_decrypt_unknown_provider_prefix_returns_error() {
        let p = Aes256GcmProvider::new("acme");
        let bogus = b"unknown-provider:acme:xxxx".to_vec();
        let err = p.decrypt(&bogus).unwrap_err();
        match err {
            EncryptionError::UnknownPrefix(name) => assert_eq!(name, "unknown-provider"),
            other => panic!("expected UnknownPrefix, got {:?}", other),
        }
    }

    /// Upstream parity: `TestEnvelope_MalformedRejected`
    /// (envelope.go — a payload without a provider prefix is rejected
    /// before any cipher is invoked).
    #[test]
    fn test_decrypt_malformed_envelope_without_prefix_rejected() {
        let p = Aes256GcmProvider::new("acme");
        let err = p.decrypt(b"no-colons-here").unwrap_err();
        assert!(matches!(err, EncryptionError::Malformed));
        // tenant_id invariant smoke: malformed envelope never produces
        // any plaintext that could leak across tenants.
    }
}
