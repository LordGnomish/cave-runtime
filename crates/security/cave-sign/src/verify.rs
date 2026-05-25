// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Verification orchestrator — runs (signature, tlog, policy, sct) together.
//!
//! Maps to:
//!   * pkg/cosign/verifiers.go        → SignatureVerifier orchestration
//!   * pkg/cosign/verify.go           → Verify (top-level)
//!   * pkg/cosign/verify_bundle.go    → VerifyBundle

use crate::bundle::CosignBundle;
use crate::error::{Result, SignError};
use crate::models::{SigKind, Signature, VerifyResult};
use crate::policy::Policy;
use crate::rekor::RekorClient;
use base64::Engine;

/// Inputs the orchestrator needs to verify a bundle.
pub struct VerifyRequest<'a> {
    pub payload: &'a [u8],
    pub bundle: &'a CosignBundle,
    pub rekor: Option<&'a RekorClient>,
    pub policy: Option<&'a Policy>,
}

/// Top-level verify: signature → digest match → optional rekor binding →
/// optional policy → optional SCT presence.
pub fn verify(req: VerifyRequest<'_>) -> Result<VerifyResult> {
    // 1. Digest must match the bundle.
    let actual = crate::signature::sha256_digest_string(req.payload);
    if actual != req.bundle.artifact_digest {
        return Err(SignError::Verify(format!(
            "digest mismatch: bundle={}, actual={}",
            req.bundle.artifact_digest, actual
        )));
    }

    // 2. Resolve (algorithm, public-key bytes) from the bundle's cert_pem.
    //    Keypair → PUBLIC KEY block; keyless → mock CERTIFICATE wrapping JSON
    //    with the ephemeral public-key PEM inside.
    let (alg, pk_bytes) = resolve_verifier_key(req.bundle)?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(req.bundle.signed_payload_b64.as_bytes())
        .map_err(|e| SignError::InvalidSignature(format!("sig base64: {}", e)))?;
    crate::signature::verify(alg, &pk_bytes, req.payload, &sig_bytes)?;
    let sig = signature_from_bundle(req.bundle);

    // 2. Rekor binding when the bundle claims one.
    if req.bundle.has_rekor_entry() {
        let rk = req
            .rekor
            .ok_or_else(|| SignError::Tlog("bundle has rekor entry but no client provided".into()))?;
        crate::tlog::verify_against_rekor(req.bundle, rk)?;
    }

    // 3. Policy.
    let mut signer = None;
    if let Some(p) = req.policy {
        let claims = match crate::policy::extract_claims(&req.bundle.cert_pem) {
            Ok(c) => c,
            Err(_) if matches!(req.bundle.kind, SigKind::Keypair) => {
                // Keypair signatures don't carry cert claims.
                crate::policy::CertClaims::default()
            }
            Err(e) => return Err(e),
        };
        let identity = p.evaluate(&sig, &claims)?;
        signer = Some(identity);
    }

    Ok(VerifyResult {
        artifact_digest: req.bundle.artifact_digest.clone(),
        valid: true,
        signer,
        reason: None,
    })
}

/// Pull `(algorithm, public_key_bytes)` from a bundle's `cert_pem`.
/// Cosign keyless certs embed the ephemeral pubkey in the X.509 SAN; our
/// mock embeds a JSON blob with a nested PEM. Either way the call returns
/// the bytes the signature verifier needs.
fn resolve_verifier_key(b: &CosignBundle) -> Result<(crate::models::KeyAlgorithm, Vec<u8>)> {
    if b.cert_pem.contains("BEGIN PUBLIC KEY") {
        return crate::keypair::decode_public_pem(&b.cert_pem);
    }
    if b.cert_pem.contains("BEGIN CERTIFICATE") {
        let body = decode_cert_body(&b.cert_pem)?;
        let j: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| SignError::Cert(format!("cert json: {}", e)))?;
        let inner_pem = j["public_key"]
            .as_str()
            .ok_or_else(|| SignError::Cert("cert missing public_key".into()))?;
        return crate::keypair::decode_public_pem(inner_pem);
    }
    Err(SignError::Cert("unrecognised cert_pem block".into()))
}

fn decode_cert_body(pem: &str) -> Result<Vec<u8>> {
    let inner: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    base64::engine::general_purpose::STANDARD
        .decode(inner.as_bytes())
        .map_err(|e| SignError::Cert(format!("cert base64: {}", e)))
}

fn signature_from_bundle(b: &CosignBundle) -> Signature {
    Signature {
        kind: b.kind,
        sig_b64: b.signed_payload_b64.clone(),
        cert_pem: b.cert_pem.clone(),
        chain_pem: b.chain_pem.clone(),
        log_index: b.rekor_log_index,
    }
}

/// Verify a base64-encoded raw signature directly. Used by callers that
/// don't bother with a full bundle.
pub fn verify_raw_signature(
    payload: &[u8],
    sig_b64: &str,
    public_key_pem: &str,
) -> Result<()> {
    let sig = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.as_bytes())
        .map_err(|e| SignError::InvalidSignature(format!("sig base64: {}", e)))?;
    let (alg, pk) = crate::keypair::decode_public_pem(public_key_pem)?;
    crate::signature::verify(alg, &pk, payload, &sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::{sign_blob_keypair, sign_blob_keypair_with_rekor};
    use crate::keyless::KeylessSigner;
    use crate::fulcio::FulcioClient;
    use crate::models::KeyAlgorithm;
    use crate::oidc::{build_fixture_jwt, IdToken};
    use crate::policy::Rule;
    use crate::signature::Keypair;
    use serde_json::json;

    fn token() -> IdToken {
        let raw = build_fixture_jwt(&json!({
            "iss":"https://oidc.cave.svc","sub":"alice","aud":"sigstore",
            "exp": chrono::Utc::now().timestamp() + 3600,
            "email":"alice@example.com",
        }));
        IdToken::parse(&raw).unwrap()
    }

    #[test]
    fn keypair_no_rekor_passes() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let b = sign_blob_keypair(b"x", &kp).unwrap();
        let req = VerifyRequest {
            payload: b"x",
            bundle: &b.bundle,
            rekor: None,
            policy: None,
        };
        let out = verify(req).unwrap();
        assert!(out.valid);
        assert_eq!(out.artifact_digest, b.bundle.artifact_digest);
    }

    #[test]
    fn rekor_binding_required_when_present() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        let rk = RekorClient::default();
        let b = sign_blob_keypair_with_rekor(b"y", &kp, &rk).unwrap();
        let req = VerifyRequest {
            payload: b"y",
            bundle: &b.bundle,
            rekor: None,
            policy: None,
        };
        let err = verify(req).expect_err("must reject");
        assert!(matches!(err, SignError::Tlog(_)));
    }

    #[test]
    fn keyless_with_policy_passes() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"abc", &token(), &rk).unwrap();
        let policy = Policy::new("cave-default")
            .require(Rule::CertificateIdentity { glob: "*@example.com".into() })
            .require(Rule::CertificateIssuer { exact: "https://oidc.cave.svc".into() })
            .require(Rule::RequireRekorEntry);
        let req = VerifyRequest {
            payload: b"abc",
            bundle: &out.bundle,
            rekor: Some(&rk),
            policy: Some(&policy),
        };
        let vr = verify(req).unwrap();
        assert_eq!(vr.signer.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn policy_rejects_wrong_identity() {
        let signer = KeylessSigner::new(FulcioClient::default());
        let rk = RekorClient::default();
        let out = signer.sign_blob(b"abc", &token(), &rk).unwrap();
        let policy = Policy::new("p").require(Rule::CertificateIdentity {
            glob: "*@cave.io".into(),
        });
        let req = VerifyRequest {
            payload: b"abc",
            bundle: &out.bundle,
            rekor: Some(&rk),
            policy: Some(&policy),
        };
        let err = verify(req).expect_err("must reject");
        assert!(matches!(err, SignError::Policy(_)));
    }

    #[test]
    fn raw_signature_verify_helper() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[3u8; 32]).unwrap();
        let b = sign_blob_keypair(b"v", &kp).unwrap();
        verify_raw_signature(b"v", &b.signature.sig_b64, &b.signature.cert_pem).unwrap();
    }

    #[test]
    fn raw_signature_rejects_wrong_payload() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[4u8; 32]).unwrap();
        let b = sign_blob_keypair(b"v", &kp).unwrap();
        assert!(verify_raw_signature(b"V", &b.signature.sig_b64, &b.signature.cert_pem).is_err());
    }
}
