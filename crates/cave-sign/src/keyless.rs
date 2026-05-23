// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keyless signing — the Fulcio + OIDC + Rekor orchestration.
//!
//! Maps to:
//!   * pkg/cosign/keyless_sign.go    → KeylessSigner
//!   * cmd/cosign/cli/sign           → keyless code-path inside `cosign sign`
//!
//! Flow:
//!   1. Caller produces an OIDC `IdToken` (we don't reach the IdP).
//!   2. We generate an ephemeral keypair and ask Fulcio for a cert binding
//!      the OIDC subject to the key.
//!   3. We sign the artifact bytes with the ephemeral key.
//!   4. We upload the (digest, signature, cert) tuple to Rekor.
//!   5. Result: a `Signature` whose `kind = SigKind::Keyless` and whose
//!      `log_index` points at the Rekor entry.

use crate::bundle::CosignBundle;
use crate::error::{Result, SignError};
use crate::fulcio::{FulcioClient, SigningCertificate};
use crate::models::{KeyAlgorithm, SigKind, Signature};
use crate::oidc::IdToken;
use crate::rekor::{HashedRekordEntry, RekorClient};
use crate::signature::{Keypair, sha256_digest_string};
use base64::Engine;

#[derive(Debug, Clone)]
pub struct KeylessSigner {
    pub fulcio: FulcioClient,
    pub algorithm: KeyAlgorithm,
}

impl KeylessSigner {
    pub fn new(fulcio: FulcioClient) -> Self {
        Self {
            fulcio,
            algorithm: KeyAlgorithm::EcdsaP256,
        }
    }

    pub fn with_algorithm(mut self, alg: KeyAlgorithm) -> Self {
        self.algorithm = alg;
        self
    }

    /// Sign `payload` keylessly. Uses Fulcio's `mock_issue` path so the
    /// flow exercises offline; production callers wire `request_certificate`
    /// instead.
    pub fn sign_blob(
        &self,
        payload: &[u8],
        token: &IdToken,
        rekor: &RekorClient,
    ) -> Result<KeylessSignature> {
        if token.is_expired_at(chrono::Utc::now().timestamp()) {
            return Err(SignError::Oidc("token already expired".into()));
        }
        let kp = Keypair::generate(self.algorithm)?;
        let csr = self.fulcio.build_csr(&kp, token)?;
        let cert = self.fulcio.mock_issue(&csr, token)?;
        self.assemble(&kp, payload, cert, rekor)
    }

    /// Async live path. Same flow, but Fulcio is queried over HTTP.
    pub async fn sign_blob_live(
        &self,
        payload: &[u8],
        token: &IdToken,
        rekor: &RekorClient,
    ) -> Result<KeylessSignature> {
        if token.is_expired_at(chrono::Utc::now().timestamp()) {
            return Err(SignError::Oidc("token already expired".into()));
        }
        let kp = Keypair::generate(self.algorithm)?;
        let csr = self.fulcio.build_csr(&kp, token)?;
        let cert = self.fulcio.request_certificate(&csr).await?;
        self.assemble(&kp, payload, cert, rekor)
    }

    fn assemble(
        &self,
        kp: &Keypair,
        payload: &[u8],
        cert: SigningCertificate,
        rekor: &RekorClient,
    ) -> Result<KeylessSignature> {
        let sig_bytes = kp.sign(payload)?;
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&sig_bytes);
        let digest = sha256_digest_string(payload);
        let chain_pem = if cert.chain_pem.is_empty() {
            None
        } else {
            Some(cert.chain_pem.join("\n"))
        };

        let digest_hex = digest.trim_start_matches("sha256:").to_string();
        let log = rekor.upload_offline(HashedRekordEntry {
            digest_hex,
            signature_b64: sig_b64.clone(),
            public_key_pem: cert.cert_pem.clone(),
        })?;

        let signature = Signature {
            kind: SigKind::Keyless,
            sig_b64: sig_b64.clone(),
            cert_pem: cert.cert_pem.clone(),
            chain_pem: chain_pem.clone(),
            log_index: Some(log.log_index),
        };
        let bundle = CosignBundle {
            kind: SigKind::Keyless,
            signed_payload_b64: sig_b64,
            cert_pem: cert.cert_pem.clone(),
            chain_pem: chain_pem.clone(),
            rekor_log_index: Some(log.log_index),
            rekor_uuid: Some(log.uuid),
            rekor_integrated_time: Some(log.integrated_time),
            artifact_digest: digest.clone(),
        };
        Ok(KeylessSignature {
            artifact_digest: digest,
            signature,
            bundle,
            ephemeral_public_key_pem: crate::keypair::encode_public_pem(
                kp.algorithm,
                kp.public_key_bytes(),
            ),
        })
    }
}

#[derive(Debug, Clone)]
pub struct KeylessSignature {
    pub artifact_digest: String,
    pub signature: Signature,
    pub bundle: CosignBundle,
    /// PEM-encoded ephemeral public key (kept for testability — the
    /// production flow only stores the Fulcio cert).
    pub ephemeral_public_key_pem: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc::build_fixture_jwt;
    use serde_json::json;

    fn token() -> IdToken {
        let raw = build_fixture_jwt(&json!({
            "iss":"https://oidc.cave.svc","sub":"alice","aud":"sigstore",
            "exp": chrono::Utc::now().timestamp() + 3600,
            "email":"alice@example.com",
        }));
        IdToken::parse(&raw).unwrap()
    }

    fn expired_token() -> IdToken {
        let raw = build_fixture_jwt(&json!({
            "iss":"x","sub":"y","aud":"z","exp": 1i64,
        }));
        IdToken::parse(&raw).unwrap()
    }

    #[test]
    fn keyless_sign_succeeds() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"hello", &token(), &rk).unwrap();
        assert_eq!(out.signature.kind, SigKind::Keyless);
        assert!(out.signature.log_index.is_some());
        assert!(out.bundle.has_rekor_entry());
        assert!(out.artifact_digest.starts_with("sha256:"));
    }

    #[test]
    fn keyless_sign_rejects_expired_token() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let err = signer
            .sign_blob(b"x", &expired_token(), &rk)
            .expect_err("must reject");
        assert!(matches!(err, SignError::Oidc(_)));
    }

    #[test]
    fn keyless_sign_with_ed25519() {
        let signer = KeylessSigner::new(FulcioClient::default()).with_algorithm(KeyAlgorithm::Ed25519);
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"abc", &token(), &rk).unwrap();
        assert!(out.signature.cert_pem.contains("CERTIFICATE"));
    }

    #[test]
    fn ephemeral_public_key_pem_set() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"a", &token(), &rk).unwrap();
        assert!(out.ephemeral_public_key_pem.contains("BEGIN PUBLIC KEY"));
    }

    #[test]
    fn rekor_index_advances() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let a = signer.sign_blob(b"a", &token(), &rk).unwrap();
        let b = signer.sign_blob(b"b", &token(), &rk).unwrap();
        assert!(a.signature.log_index.unwrap() < b.signature.log_index.unwrap());
    }

    #[test]
    fn bundle_carries_chain() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"x", &token(), &rk).unwrap();
        assert!(out.bundle.chain_pem.is_some());
    }
}
