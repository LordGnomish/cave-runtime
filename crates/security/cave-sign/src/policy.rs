// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Verification policy — certificate-identity + certificate-issuer matching.
//!
//! Maps to:
//!   * cmd/cosign/cli/verify --certificate-identity-regexp / --certificate-oidc-issuer
//!   * pkg/cosign/certextensions.go → matchCertificateExtensions
//!   * pkg/cosign/cue               → CUE policy DSL (we use a stripped-down rule list)
//!
//! A `Policy` is a conjunction of rules — every rule must pass.
//! `glob` patterns support `*` only (greedy) and case-sensitive matching,
//! matching cosign's `--certificate-identity` flag semantics.

use crate::error::{Result, SignError};
use crate::models::Signature;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Policy {
    /// Each rule is a (key, matcher) pair.
    pub rules: Vec<Rule>,
    /// Optional human-readable label.
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Rule {
    /// `--certificate-identity` / `--certificate-identity-regexp`. We
    /// implement glob (`*`) matching, not full regex — matches cosign 2.x
    /// `--certificate-identity` semantics.
    CertificateIdentity { glob: String },
    /// `--certificate-oidc-issuer` exact match.
    CertificateIssuer { exact: String },
    /// `--certificate-github-workflow-trigger` etc. We model arbitrary
    /// SAN-style extensions through this generic key/glob.
    CertificateExtension { key: String, glob: String },
    /// The signature must carry a Rekor log entry (i.e. proven keyless).
    RequireRekorEntry,
    /// The signature must be of `SigKind::Keyless`.
    RequireKeyless,
}

/// Extracted cert claims. In a real flow `extract_claims` parses an X.509
/// cert; for cave-sign's offline tests we use the JSON body the Fulcio
/// mock encodes.
#[derive(Debug, Clone, Default)]
pub struct CertClaims {
    pub identity: String,
    pub issuer: String,
    pub extensions: std::collections::BTreeMap<String, String>,
}

impl Policy {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: Vec::new(),
        }
    }

    pub fn require(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Evaluate the policy. Returns the matched identity (handy for audit
    /// logging) if every rule passes; otherwise the first failure.
    pub fn evaluate(&self, sig: &Signature, claims: &CertClaims) -> Result<String> {
        for r in &self.rules {
            match r {
                Rule::CertificateIdentity { glob } => {
                    if !glob_match(glob, &claims.identity) {
                        return Err(SignError::Policy(format!(
                            "identity {:?} does not match {:?}",
                            claims.identity, glob
                        )));
                    }
                }
                Rule::CertificateIssuer { exact } => {
                    if &claims.issuer != exact {
                        return Err(SignError::Policy(format!(
                            "issuer {:?} != required {:?}",
                            claims.issuer, exact
                        )));
                    }
                }
                Rule::CertificateExtension { key, glob } => {
                    let v = claims.extensions.get(key).map(String::as_str).unwrap_or("");
                    if !glob_match(glob, v) {
                        return Err(SignError::Policy(format!(
                            "extension {:?} ({:?}) does not match {:?}",
                            key, v, glob
                        )));
                    }
                }
                Rule::RequireRekorEntry => {
                    if sig.log_index.is_none() {
                        return Err(SignError::Policy("rekor entry required".into()));
                    }
                }
                Rule::RequireKeyless => {
                    if sig.kind != crate::models::SigKind::Keyless {
                        return Err(SignError::Policy("keyless signature required".into()));
                    }
                }
            }
        }
        Ok(claims.identity.clone())
    }
}

/// Extract claims from a Fulcio-mock certificate PEM. Real X.509 parsing
/// is deferred to Phase 2 (cave-vault owns chain validation), so for now
/// we look up the JSON body the mock encodes.
pub fn extract_claims(cert_pem: &str) -> Result<CertClaims> {
    let body = decode_pem_body(cert_pem)?;
    if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&body) {
        let identity = j["subject_alt_name"].as_str().unwrap_or("").to_string();
        let issuer = j["oidc_issuer"].as_str().unwrap_or("").to_string();
        let extensions = j["extensions"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        return Ok(CertClaims {
            identity,
            issuer,
            extensions,
        });
    }
    Err(SignError::Cert("unrecognised certificate body".into()))
}

fn decode_pem_body(pem: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    let inner: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    base64::engine::general_purpose::STANDARD
        .decode(inner.as_bytes())
        .map_err(|e| SignError::Cert(format!("pem base64: {}", e)))
}

/// Minimal `*`-style glob matcher — cosign's `--certificate-identity` flag
/// supports glob, not regex; this matches that surface.
pub fn glob_match(pattern: &str, input: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == input;
    }
    let mut cursor = 0usize;
    // First part must prefix `input` (anchored).
    if let Some(first) = parts.first() {
        if !first.is_empty() {
            if !input.starts_with(first) {
                return false;
            }
            cursor += first.len();
        }
    }
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        if let Some(idx) = input[cursor..].find(part) {
            cursor += idx + part.len();
        } else {
            return false;
        }
    }
    if let Some(last) = parts.last() {
        if !last.is_empty() {
            if cursor + last.len() > input.len() {
                return false;
            }
            return input[cursor..].ends_with(last);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulcio::FulcioClient;
    use crate::keypair::encode_public_pem;
    use crate::models::{KeyAlgorithm, SigKind};
    use crate::oidc::IdToken;
    use crate::signature::Keypair;
    use serde_json::json;

    fn fixture_sig() -> Signature {
        Signature {
            kind: SigKind::Keyless,
            sig_b64: "x".into(),
            cert_pem: "p".into(),
            chain_pem: None,
            log_index: Some(1),
        }
    }

    fn fixture_claims() -> CertClaims {
        CertClaims {
            identity: "alice@example.com".into(),
            issuer: "https://accounts.google.com".into(),
            extensions: Default::default(),
        }
    }

    #[test]
    fn empty_policy_passes() {
        let p = Policy::new("empty");
        let out = p.evaluate(&fixture_sig(), &fixture_claims()).unwrap();
        assert_eq!(out, "alice@example.com");
    }

    #[test]
    fn identity_glob_matches() {
        let p = Policy::new("p").require(Rule::CertificateIdentity {
            glob: "*@example.com".into(),
        });
        p.evaluate(&fixture_sig(), &fixture_claims()).unwrap();
    }

    #[test]
    fn identity_glob_rejects() {
        let p = Policy::new("p").require(Rule::CertificateIdentity {
            glob: "*@cave.io".into(),
        });
        let err = p
            .evaluate(&fixture_sig(), &fixture_claims())
            .expect_err("must reject");
        assert!(matches!(err, SignError::Policy(_)));
    }

    #[test]
    fn issuer_exact_matches() {
        let p = Policy::new("p").require(Rule::CertificateIssuer {
            exact: "https://accounts.google.com".into(),
        });
        p.evaluate(&fixture_sig(), &fixture_claims()).unwrap();
    }

    #[test]
    fn issuer_exact_rejects() {
        let p = Policy::new("p").require(Rule::CertificateIssuer {
            exact: "https://gitlab.com".into(),
        });
        assert!(p.evaluate(&fixture_sig(), &fixture_claims()).is_err());
    }

    #[test]
    fn require_rekor_passes_when_index_set() {
        let p = Policy::new("p").require(Rule::RequireRekorEntry);
        p.evaluate(&fixture_sig(), &fixture_claims()).unwrap();
    }

    #[test]
    fn require_rekor_fails_when_absent() {
        let mut s = fixture_sig();
        s.log_index = None;
        let p = Policy::new("p").require(Rule::RequireRekorEntry);
        assert!(p.evaluate(&s, &fixture_claims()).is_err());
    }

    #[test]
    fn require_keyless_rejects_keypair() {
        let mut s = fixture_sig();
        s.kind = SigKind::Keypair;
        let p = Policy::new("p").require(Rule::RequireKeyless);
        assert!(p.evaluate(&s, &fixture_claims()).is_err());
    }

    #[test]
    fn extension_glob_matches() {
        let mut c = fixture_claims();
        c.extensions.insert(
            "github-workflow-trigger".into(),
            "push".into(),
        );
        let p = Policy::new("p").require(Rule::CertificateExtension {
            key: "github-workflow-trigger".into(),
            glob: "p*".into(),
        });
        p.evaluate(&fixture_sig(), &c).unwrap();
    }

    #[test]
    fn glob_anchors_both_ends() {
        assert!(glob_match("alice@*.com", "alice@example.com"));
        assert!(!glob_match("alice@*.com", "alice@example.org"));
        assert!(glob_match("*", "literally-anything"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exacto"));
        assert!(!glob_match("alice@example.com", "alice@example.com.evil"));
    }

    #[test]
    fn glob_multiple_stars() {
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "ac"));
    }

    #[test]
    fn extract_claims_from_mock_cert() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[42u8; 32]).unwrap();
        let raw = crate::oidc::build_fixture_jwt(&json!({
            "iss":"https://oidc.cave.svc","sub":"alice|1","aud":"sigstore",
            "exp":1_999_999_999i64,"email":"alice@example.com",
        }));
        let tok = IdToken::parse(&raw).unwrap();
        let fc = FulcioClient::default();
        let csr = fc.build_csr(&kp, &tok).unwrap();
        let cert = fc.mock_issue(&csr, &tok).unwrap();
        let claims = extract_claims(&cert.cert_pem).unwrap();
        assert_eq!(claims.identity, "alice@example.com");
        assert_eq!(claims.issuer, "https://oidc.cave.svc");
    }

    #[test]
    fn fulcio_keypair_pem_does_not_carry_claims() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[1u8; 32]).unwrap();
        let pem = encode_public_pem(kp.algorithm, kp.public_key_bytes());
        // The keypair PEM body decodes to raw bytes — not JSON — so we
        // reject it as a cert with a clear error.
        let err = extract_claims(&pem).expect_err("must fail");
        assert!(matches!(err, SignError::Cert(_)));
    }
}
