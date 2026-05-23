// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SelfSigned issuer — generates a per-request self-signed certificate.
//!
//! Cite: `pkg/issuer/selfsigned/sign.go::Sign` — cert-manager's
//! SelfSigned issuer signs the requested SANs with the
//! CertificateRequest's own private key. No CA hierarchy is touched.

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::IssueOutcome;
use crate::models::{CertificateRequest, IssuerSpec};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Default)]
pub struct SelfSignedIssuer;

impl SelfSignedIssuer {
    pub fn issue(
        &self,
        spec: &IssuerSpec,
        req: &CertificateRequest,
    ) -> CertManagerResult<IssueOutcome> {
        let crl_dps = match spec {
            IssuerSpec::SelfSigned {
                crl_distribution_points,
            } => crl_distribution_points.clone(),
            _ => {
                return Err(CertManagerError::InvalidSpec(
                    "SelfSignedIssuer.issue called with non-SelfSigned spec".into(),
                ));
            }
        };

        let now = Utc::now();
        let not_after = now + Duration::seconds(req.duration_seconds);
        // Synthetic serial = sha256(name|revision|first SAN). cite cert-manager
        // tests `selfsigned/sign_test.go` — serial only needs to be unique for
        // the issuer's lifetime; sha-derived is plenty for our in-memory store.
        let mut hasher = Sha256::new();
        hasher.update(req.name.as_bytes());
        hasher.update(req.revision.to_be_bytes());
        if let Some(first) = req.dns_names.first() {
            hasher.update(first.as_bytes());
        }
        let serial = hex::encode(hasher.finalize()).chars().take(32).collect::<String>();

        let pem = build_synthetic_pem(
            "SELF-SIGNED",
            &req.name,
            &req.dns_names,
            &serial,
            &crl_dps,
            req.is_ca,
        );

        Ok(IssueOutcome {
            certificate_chain_pem: pem.clone(),
            // Self-signed has no CA chain — leaf == CA.
            ca_pem: pem,
            not_before: now,
            not_after,
            serial,
        })
    }
}

/// Internal helper — emits a stand-in PEM that round-trips through the
/// secret reconciler and the renewal scheduler. Real X.509 generation
/// is deferred until cave-pki carries an ASN.1 encoder
/// (see `[[partial]] kube-apply-client`-style note in the manifest).
pub(crate) fn build_synthetic_pem(
    label: &str,
    cn: &str,
    sans: &[String],
    serial: &str,
    crl_dps: &[String],
    is_ca: bool,
) -> String {
    let mut body = format!(
        "CN={cn}\nSerial={serial}\nIsCA={is_ca}\nSANs={sans}\n",
        cn = cn,
        serial = serial,
        is_ca = is_ca,
        sans = sans.join(","),
    );
    if !crl_dps.is_empty() {
        body.push_str(&format!("CRLs={}\n", crl_dps.join(",")));
    }
    format!(
        "-----BEGIN CERTIFICATE-----\n{}\n# {label}\n-----END CERTIFICATE-----\n",
        base64_lines(&body.into_bytes()),
        label = label,
    )
}

/// 64-column base64 lines per RFC 7468 §3 ("textual encoding").
fn base64_lines(bytes: &[u8]) -> String {
    use base64::Engine as _;
    let e = base64::engine::general_purpose::STANDARD;
    let s = e.encode(bytes);
    s.as_bytes()
        .chunks(64)
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateRequestStatus, IssuerRef, IssuerRefKind, Usage,
    };
    use uuid::Uuid;

    fn req(is_ca: bool) -> CertificateRequest {
        CertificateRequest {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            certificate_id: Uuid::new_v4(),
            revision: 3,
            issuer_ref: IssuerRef {
                name: "selfsigned".into(),
                kind: IssuerRefKind::Issuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec!["api.example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: Some("api.example.com".into()),
            duration_seconds: 3600,
            is_ca,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        }
    }

    #[test]
    fn issues_pem_with_expected_validity() {
        let outcome = SelfSignedIssuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &req(false),
            )
            .unwrap();
        assert!(outcome.certificate_chain_pem.contains("BEGIN CERTIFICATE"));
        let lifetime = (outcome.not_after - outcome.not_before).num_seconds();
        assert!((lifetime - 3600).abs() <= 1);
    }

    #[test]
    fn ca_flag_lands_in_pem_body() {
        let outcome = SelfSignedIssuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &req(true),
            )
            .unwrap();
        let body = outcome.certificate_chain_pem.clone();
        // base64-encoded body must round-trip back to plaintext containing IsCA=true
        let inner = body
            .lines()
            .filter(|l| !l.starts_with("-----") && !l.starts_with('#'))
            .collect::<String>();
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(inner.as_bytes())
            .unwrap();
        let decoded = String::from_utf8(bytes).unwrap();
        assert!(decoded.contains("IsCA=true"));
        assert!(decoded.contains("CN=demo"));
    }

    #[test]
    fn distinct_revisions_produce_distinct_serials() {
        let mut r1 = req(false);
        r1.revision = 1;
        let mut r2 = req(false);
        r2.revision = 2;
        let issuer = SelfSignedIssuer;
        let s1 = issuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &r1,
            )
            .unwrap()
            .serial;
        let s2 = issuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &r2,
            )
            .unwrap()
            .serial;
        assert_ne!(s1, s2);
    }

    #[test]
    fn rejects_wrong_spec_variant() {
        let err = SelfSignedIssuer
            .issue(
                &IssuerSpec::Ca {
                    secret_name: "x".into(),
                    crl_distribution_points: vec![],
                },
                &req(false),
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::InvalidSpec(_)));
    }

    #[test]
    fn crl_dps_carried_into_pem() {
        let outcome = SelfSignedIssuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec!["http://crl.example.com/root.crl".into()],
                },
                &req(true),
            )
            .unwrap();
        let body = outcome.certificate_chain_pem;
        let inner = body
            .lines()
            .filter(|l| !l.starts_with("-----") && !l.starts_with('#'))
            .collect::<String>();
        use base64::Engine as _;
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(inner.as_bytes())
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.contains("CRLs=http://crl.example.com/root.crl"));
    }
}
