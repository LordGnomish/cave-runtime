// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Fulcio CA HTTP client.
//!
//! Maps to:
//!   * pkg/cosign/fulcioverifier  → FulcioClient
//!   * cmd/cosign/cli/fulcio      → fulcio sub-command surface
//!
//! Fulcio is the Sigstore certificate authority — it accepts a CSR-equivalent
//! (signed proof-of-possession of the OIDC identity) and returns an
//! ephemeral X.509 certificate whose SAN binds the OIDC subject into the
//! signing material.
//!
//! Production cave deployments target the public good Fulcio
//! (https://fulcio.sigstore.dev), but the client also supports a
//! cave-internal Fulcio for sovereign installs.

use crate::error::{Result, SignError};
use crate::oidc::IdToken;
use crate::signature::Keypair;
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const PUBLIC_GOOD_FULCIO_URL: &str = "https://fulcio.sigstore.dev";
pub const CAVE_FULCIO_DEFAULT_URL: &str = "http://cave-fulcio.cave.svc.cluster.local:5555";

/// Fulcio "signing certificate" response. We model the JSON shape from
/// Fulcio v1 (`/api/v2/signingCert`) — `signedCertificateDetachedSct` for
/// the cert + sct envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SigningCertificate {
    pub cert_pem: String,
    pub chain_pem: Vec<String>,
    /// Signed Certificate Timestamp — base64-encoded blob.
    pub sct_b64: Option<String>,
}

/// CSR-like request body Fulcio expects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrRequest {
    /// PEM-encoded public key.
    pub public_key_pem: String,
    /// Proof of possession: signature over `email||sub` with the same key.
    pub proof_b64: String,
    /// OIDC ID token (raw JWT) — Fulcio re-verifies against issuer JWKS.
    pub id_token: String,
}

/// Configured Fulcio endpoint. `request_certificate` is async-capable via
/// `reqwest`, but unit tests exercise the offline `mock_issue` path so the
/// crate's deep-port tests don't depend on network.
#[derive(Debug, Clone)]
pub struct FulcioClient {
    pub base_url: String,
}

impl Default for FulcioClient {
    fn default() -> Self {
        Self {
            base_url: PUBLIC_GOOD_FULCIO_URL.to_string(),
        }
    }
}

impl FulcioClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            base_url: url.into(),
        }
    }

    /// Build the CSR body Fulcio expects from a keypair + OIDC token.
    pub fn build_csr(&self, kp: &Keypair, token: &IdToken) -> Result<CsrRequest> {
        let public_key_pem =
            crate::keypair::encode_public_pem(kp.algorithm, kp.public_key_bytes());
        let payload = token.identity().as_bytes();
        let proof = kp.sign(payload)?;
        let proof_b64 = base64::engine::general_purpose::STANDARD.encode(proof);
        Ok(CsrRequest {
            public_key_pem,
            proof_b64,
            id_token: token.raw.clone(),
        })
    }

    /// Verify a CSR's proof-of-possession matches the embedded public key
    /// and the OIDC identity. This is exactly what Fulcio does server-side
    /// — we expose it so the cave-fulcio operator can reuse the check.
    pub fn verify_csr(&self, csr: &CsrRequest, expected_identity: &str) -> Result<()> {
        let (alg, pk_bytes) = crate::keypair::decode_public_pem(&csr.public_key_pem)?;
        let proof = base64::engine::general_purpose::STANDARD
            .decode(csr.proof_b64.as_bytes())
            .map_err(|e| SignError::Fulcio(format!("proof base64: {}", e)))?;
        crate::signature::verify(alg, &pk_bytes, expected_identity.as_bytes(), &proof)
            .map_err(|e| SignError::Fulcio(format!("proof verify: {}", e)))?;
        Ok(())
    }

    /// Offline cert issuer used by tests + cave-internal Fulcio stub.
    /// Returns a self-signed-like cert PEM that round-trips through our
    /// own verifier; this is **not** a real X.509 — production must call
    /// `request_certificate` against a true Fulcio.
    pub fn mock_issue(&self, csr: &CsrRequest, token: &IdToken) -> Result<SigningCertificate> {
        self.verify_csr(csr, token.identity())?;
        let cert_body = serde_json::json!({
            "issuer": "cave-fulcio-mock",
            "subject_alt_name": token.identity(),
            "oidc_issuer": token.issuer,
            "public_key": csr.public_key_pem,
            "not_after": token.exp,
        });
        let cert_pem = wrap_pem("CERTIFICATE", &serde_json::to_vec(&cert_body).unwrap());
        let chain_pem = vec![wrap_pem(
            "CERTIFICATE",
            b"cave-fulcio-mock-root",
        )];
        Ok(SigningCertificate {
            cert_pem,
            chain_pem,
            sct_b64: Some(
                base64::engine::general_purpose::STANDARD.encode(b"mock-sct"),
            ),
        })
    }

    /// Live HTTP request. Returns `SignError::Fulcio` if the server is
    /// unreachable or the JSON shape doesn't match.
    pub async fn request_certificate(&self, csr: &CsrRequest) -> Result<SigningCertificate> {
        let url = format!("{}/api/v2/signingCert", self.base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&csr)
            .send()
            .await
            .map_err(|e| SignError::Fulcio(format!("post {}: {}", url, e)))?;
        if !resp.status().is_success() {
            return Err(SignError::Fulcio(format!(
                "fulcio status {}",
                resp.status()
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SignError::Fulcio(format!("json decode: {}", e)))?;
        SigningCertificate::from_fulcio_v2(&json)
    }
}

impl SigningCertificate {
    pub fn from_fulcio_v2(j: &serde_json::Value) -> Result<Self> {
        let detached = &j["signedCertificateDetachedSct"];
        let chain_node = if detached.is_object() {
            &detached["chain"]
        } else {
            &j["signedCertificateEmbeddedSct"]["chain"]
        };
        let cert_pem = chain_node["certificates"][0]
            .as_str()
            .ok_or_else(|| SignError::Fulcio("missing first cert".into()))?
            .to_string();
        let chain_pem = chain_node["certificates"]
            .as_array()
            .map(|a| a.iter().skip(1).filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let sct_b64 = j["signedCertificateDetachedSct"]["signedCertificateTimestamp"]
            .as_str()
            .map(str::to_string);
        Ok(Self {
            cert_pem,
            chain_pem,
            sct_b64,
        })
    }
}

fn wrap_pem(header: &str, body: &[u8]) -> String {
    let mut s = String::new();
    s.push_str(&format!("-----BEGIN {}-----\n", header));
    let b64 = base64::engine::general_purpose::STANDARD.encode(body);
    for chunk in b64.as_bytes().chunks(64) {
        s.push_str(std::str::from_utf8(chunk).unwrap());
        s.push('\n');
    }
    s.push_str(&format!("-----END {}-----\n", header));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KeyAlgorithm;
    use crate::oidc::build_fixture_jwt;
    use serde_json::json;

    fn fixture_token() -> IdToken {
        let raw = build_fixture_jwt(&json!({
            "iss":"https://oidc.cave.svc","sub":"workload://alice",
            "aud":"sigstore","exp": 1_999_999_999i64,
            "email":"alice@example.com",
        }));
        IdToken::parse(&raw).unwrap()
    }

    #[test]
    fn build_csr_pop_signs_identity() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let tok = fixture_token();
        let fc = FulcioClient::new(CAVE_FULCIO_DEFAULT_URL);
        let csr = fc.build_csr(&kp, &tok).unwrap();
        assert!(csr.public_key_pem.contains("BEGIN PUBLIC KEY"));
        assert!(!csr.proof_b64.is_empty());
        fc.verify_csr(&csr, tok.identity()).unwrap();
    }

    #[test]
    fn verify_csr_rejects_wrong_identity() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        let tok = fixture_token();
        let fc = FulcioClient::default();
        let csr = fc.build_csr(&kp, &tok).unwrap();
        let err = fc.verify_csr(&csr, "bob@example.com").expect_err("must reject");
        assert!(matches!(err, SignError::Fulcio(_)));
    }

    #[test]
    fn mock_issue_returns_cert_chain() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[3u8; 32]).unwrap();
        let tok = fixture_token();
        let fc = FulcioClient::default();
        let csr = fc.build_csr(&kp, &tok).unwrap();
        let cert = fc.mock_issue(&csr, &tok).unwrap();
        assert!(cert.cert_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(cert.chain_pem.len(), 1);
        assert!(cert.sct_b64.is_some());
    }

    #[test]
    fn fulcio_v2_response_parses() {
        let j = json!({
            "signedCertificateDetachedSct": {
                "chain": {
                    "certificates": [
                        "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----",
                        "-----BEGIN CERTIFICATE-----\nINT\n-----END CERTIFICATE-----",
                    ]
                },
                "signedCertificateTimestamp": "AAAA",
            }
        });
        let cert = SigningCertificate::from_fulcio_v2(&j).unwrap();
        assert!(cert.cert_pem.contains("LEAF"));
        assert_eq!(cert.chain_pem.len(), 1);
        assert!(cert.chain_pem[0].contains("INT"));
        assert_eq!(cert.sct_b64.as_deref(), Some("AAAA"));
    }

    #[test]
    fn fulcio_v2_embedded_sct_variant_parses() {
        let j = json!({
            "signedCertificateEmbeddedSct": {
                "chain": {"certificates": ["X","Y"]}
            }
        });
        let cert = SigningCertificate::from_fulcio_v2(&j).unwrap();
        assert_eq!(cert.cert_pem, "X");
        assert_eq!(cert.chain_pem, vec!["Y".to_string()]);
        assert!(cert.sct_b64.is_none());
    }

    #[test]
    fn default_url_is_public_good() {
        let fc = FulcioClient::default();
        assert_eq!(fc.base_url, PUBLIC_GOOD_FULCIO_URL);
    }
}
