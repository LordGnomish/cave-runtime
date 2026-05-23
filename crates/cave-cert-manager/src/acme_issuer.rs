// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! ACME issuer — drives the cert-manager Certificate ↔ ACME Order
//! projection.
//!
//! Cite: `pkg/issuer/acme/order/order.go::buildOrder` —
//! cert-manager projects a CertificateRequest into one Order per
//! attempt, watches its authorisations advance, and presents each
//! Challenge through the right solver (HTTP-01 ingress publisher /
//! DNS-01 zone provider). cave-cert-manager owns the state machine
//! driver only — the RFC 8555 protocol is implemented by
//! [`cave_acme::AcmeServer`].
//!
//! Solver scope:
//!   * `HTTP-01` — publishes `{token: key_authorization}` records that
//!     the cave-gateway / cave-net Ingress controller serves under
//!     `/.well-known/acme-challenge/`.
//!   * `DNS-01`  — emits `Dns01Plan` records describing the TXT entry
//!     the cave-dns provider must publish on `_acme-challenge.<zone>`.

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::{require_keychain_handle, IssueOutcome};
use crate::models::{
    AcmeChallengeSolver, AcmeSolver, CertificateRequest, DnsProvider, IssuerSpec,
};
use crate::selfsigned_issuer::build_synthetic_pem;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cave_acme::{
    AcmeServer, ChallengeType, Identifier, IdentifierType, Jwk,
};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// Plan describing the cave-net HTTP-01 publisher's job:
/// for each token, serve `key_authorization` at
/// `/.well-known/acme-challenge/<token>`. Cite RFC 8555 §8.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http01Plan {
    pub domain: String,
    pub token: String,
    pub key_authorization: String,
}

/// Plan describing the cave-dns provider's job: write a TXT record
/// `_acme-challenge.<zone>` with value `digest`. Cite RFC 8555 §8.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dns01Plan {
    pub zone: String,
    pub record_name: String,
    pub digest: String,
}

#[derive(Debug, Default)]
pub struct AcmeIssuer {
    pub server: AcmeServer,
    /// Per-domain published HTTP-01 plans keyed by `(domain, challenge_id)`.
    /// Used by tests to assert the solver fired.
    pub http_plans: HashMap<String, Http01Plan>,
    /// Per-zone DNS-01 plans (zone → plan).
    pub dns_plans: HashMap<String, Dns01Plan>,
}

impl AcmeIssuer {
    /// Drive an Order through the full lifecycle:
    /// `newOrder → solve(authz×N) → finalize → emit chain`.
    pub fn issue(
        &mut self,
        spec: &IssuerSpec,
        req: &CertificateRequest,
    ) -> CertManagerResult<IssueOutcome> {
        let (directory_url, account_key_handle, emails, tos_agreed, solvers) = match spec {
            IssuerSpec::Acme {
                directory_url,
                account_key_keychain_handle,
                email,
                terms_of_service_agreed,
                solvers,
            } => (
                directory_url.clone(),
                account_key_keychain_handle.clone(),
                email.clone(),
                *terms_of_service_agreed,
                solvers.clone(),
            ),
            _ => {
                return Err(CertManagerError::InvalidSpec(
                    "AcmeIssuer.issue called with non-Acme spec".into(),
                ));
            }
        };

        // Enforce keychain-only secret access. The account key MUST resolve
        // through the keychain — cave-cert-manager NEVER stores key material.
        let _key_label = require_keychain_handle(&account_key_handle)?;
        if req.dns_names.is_empty() {
            return Err(CertManagerError::EmptyDnsNames);
        }

        // ── 1. Account: lookup-or-create against the directory URL. ───────
        let jwk = synthetic_jwk(&account_key_handle, &directory_url);
        let contact: Vec<String> = emails.iter().map(|e| format!("mailto:{}", e)).collect();
        let account_id = self.server.new_account(
            req.tenant_id.clone(),
            jwk.clone(),
            contact,
            tos_agreed,
            None,
        )?;

        // ── 2. newOrder over the requested SAN dnsNames. ─────────────────
        let identifiers: Vec<Identifier> =
            req.dns_names.iter().map(|d| Identifier::dns(d.to_lowercase())).collect();
        let order_id = self.server.new_order(
            &req.tenant_id,
            &account_id,
            identifiers.clone(),
        )?;

        // ── 3. Solve every challenge using the matching solver. ──────────
        // Snapshot the authorisation ids so we don't hold a borrow across
        // the mutable advance calls.
        let authz_ids: Vec<String> = self
            .server
            .order(&req.tenant_id, &order_id)?
            .authorization_ids
            .clone();
        for (ident, authz_id) in identifiers.iter().zip(authz_ids.iter()) {
            if ident.kind != IdentifierType::Dns {
                continue;
            }
            let solver = pick_solver(&solvers, &ident.value).ok_or_else(|| {
                CertManagerError::AcmeOrder(format!(
                    "no ACME solver matches dnsName {}",
                    ident.value
                ))
            })?;
            self.solve_one(&req.tenant_id, authz_id, &ident.value, solver, &jwk)?;
        }

        // ── 4. Finalize: Ready (auto from mark_challenge_valid) → Valid. ──
        let cert_url = format!("/cave-cert-manager/issued/{}", order_id);
        self.server
            .finalize_order(&req.tenant_id, &order_id, &cert_url)?;

        // ── 5. Synthesise the issued chain (real ASN.1 lives in Phase 2). ─
        let now = Utc::now();
        let not_after = now + Duration::seconds(req.duration_seconds);
        let mut hasher = Sha256::new();
        hasher.update(directory_url.as_bytes());
        hasher.update(req.tenant_id.as_bytes());
        hasher.update(req.name.as_bytes());
        hasher.update(req.revision.to_be_bytes());
        for d in &req.dns_names {
            hasher.update(d.as_bytes());
        }
        let serial = hex::encode(hasher.finalize()).chars().take(32).collect::<String>();

        let leaf = build_synthetic_pem(
            "ACME-ISSUED-LEAF",
            &req.name,
            &req.dns_names,
            &serial,
            &[],
            req.is_ca,
        );
        let ca = build_synthetic_pem(
            "ACME-INTERMEDIATE",
            &format!("ACME-directory:{}", directory_url),
            &[],
            "acme-intermediate",
            &[],
            true,
        );

        Ok(IssueOutcome {
            certificate_chain_pem: format!("{}{}", leaf, ca),
            ca_pem: ca,
            not_before: now,
            not_after,
            serial,
        })
    }

    fn solve_one(
        &mut self,
        tenant: &str,
        authz_id: &str,
        domain: &str,
        solver: &AcmeSolver,
        jwk: &Jwk,
    ) -> CertManagerResult<()> {
        // Look up the challenge of the right type on the authorisation.
        let (challenge_id, token) = {
            let authz = self.server.authorization(tenant, authz_id)?;
            let wanted = match &solver.challenge {
                AcmeChallengeSolver::Http01 { .. } => ChallengeType::Http01,
                AcmeChallengeSolver::Dns01 { .. } => ChallengeType::Dns01,
            };
            let ch = authz
                .challenges
                .iter()
                .find(|c| c.kind == wanted)
                .ok_or_else(|| CertManagerError::AcmeChallenge {
                    challenge_id: authz_id.to_string(),
                    reason: format!("no {} challenge on authorization", wanted.as_str()),
                })?;
            (ch.id.clone(), ch.token.clone())
        };

        match &solver.challenge {
            AcmeChallengeSolver::Http01 { .. } => {
                let key_auth = key_authorization(&token, jwk);
                self.http_plans.insert(
                    format!("{}::{}", domain, challenge_id),
                    Http01Plan {
                        domain: domain.into(),
                        token: token.clone(),
                        key_authorization: key_auth,
                    },
                );
            }
            AcmeChallengeSolver::Dns01 { provider } => {
                let zone = match provider {
                    DnsProvider::CaveDns { zone } => zone.clone(),
                    DnsProvider::Webhook { .. } => {
                        return Err(CertManagerError::AcmeChallenge {
                            challenge_id,
                            reason: "webhook DNS provider is Phase 2".into(),
                        });
                    }
                };
                let key_auth = key_authorization(&token, jwk);
                // Cite: RFC 8555 §8.4 — TXT value = base64url(SHA-256(key_auth))
                let digest = URL_SAFE_NO_PAD.encode(Sha256::digest(key_auth.as_bytes()));
                let record_name = format!("_acme-challenge.{}", domain.trim_end_matches('.'));
                self.dns_plans.insert(
                    zone.clone(),
                    Dns01Plan {
                        zone,
                        record_name,
                        digest,
                    },
                );
            }
        }

        self.server.mark_challenge_valid(tenant, &challenge_id)?;
        Ok(())
    }
}

/// Match an ACMESolver against a domain — empty `dns_zones` matches all
/// (cert-manager `pkg/issuer/acme/dns/dns.go::Solve`). Longest zone wins.
pub fn pick_solver<'a>(solvers: &'a [AcmeSolver], domain: &str) -> Option<&'a AcmeSolver> {
    let domain = domain.trim_end_matches('.');
    let mut best: Option<(&AcmeSolver, usize)> = None;
    let mut catch_all: Option<&AcmeSolver> = None;
    for s in solvers {
        if s.dns_zones.is_empty() {
            catch_all = catch_all.or(Some(s));
            continue;
        }
        for z in &s.dns_zones {
            let zone = z.trim_end_matches('.');
            if domain == zone || domain.ends_with(&format!(".{}", zone)) {
                let score = zone.len();
                if best.map(|(_, s)| score > s).unwrap_or(true) {
                    best = Some((s, score));
                }
            }
        }
    }
    best.map(|(s, _)| s).or(catch_all)
}

fn synthetic_jwk(handle: &str, directory_url: &str) -> Jwk {
    // We don't synthesise real EC point bytes — keychain access is
    // deferred to runtime wiring. We mix handle + directory into the
    // deterministic stand-in so distinct accounts get distinct
    // thumbprints.
    let mut hasher = Sha256::new();
    hasher.update(handle.as_bytes());
    hasher.update(b"|");
    hasher.update(directory_url.as_bytes());
    let digest = hasher.finalize();
    let x = URL_SAFE_NO_PAD.encode(&digest[..16]);
    let y = URL_SAFE_NO_PAD.encode(&digest[16..]);
    Jwk::EC {
        crv: "P-256".into(),
        x,
        y,
    }
}

/// Cite: RFC 8555 §8.1 — `keyAuthorization = token || '.' ||
/// base64url(JWK_thumbprint)`.
pub fn key_authorization(token: &str, jwk: &Jwk) -> String {
    format!("{}.{}", token, jwk.thumbprint())
}

/// Helper used by tests + the renewal smoke test — gives us a freshly
/// scoped Uuid string so callers can disambiguate Certificate names.
pub fn synth_name() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateRequestStatus, IssuerRef, IssuerRefKind, Usage,
    };

    fn req(name: &str) -> CertificateRequest {
        CertificateRequest {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            certificate_id: Uuid::new_v4(),
            revision: 1,
            issuer_ref: IssuerRef {
                name: "letsencrypt".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec!["api.example.com".into(), "www.example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 90 * 24 * 3600,
            is_ca: false,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        }
    }

    fn acme_spec(solver: AcmeChallengeSolver, zones: Vec<String>) -> IssuerSpec {
        IssuerSpec::Acme {
            directory_url: "https://acme-staging.example.com/directory".into(),
            account_key_keychain_handle: "keychain:cave-acme-account".into(),
            email: vec!["ops@example.com".into()],
            terms_of_service_agreed: true,
            solvers: vec![AcmeSolver {
                dns_zones: zones,
                challenge: solver,
            }],
        }
    }

    #[test]
    fn http01_solve_publishes_plan_per_domain() {
        let mut issuer = AcmeIssuer::default();
        let outcome = issuer
            .issue(
                &acme_spec(
                    AcmeChallengeSolver::Http01 {
                        ingress_class: Some("cave-gw".into()),
                        service_type: None,
                    },
                    vec![],
                ),
                &req("demo"),
            )
            .unwrap();
        assert_eq!(issuer.http_plans.len(), 2);
        let any = issuer.http_plans.values().next().unwrap();
        assert!(any.key_authorization.contains('.'));
        assert!(outcome.certificate_chain_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn dns01_solve_emits_digest_not_raw_token() {
        let mut issuer = AcmeIssuer::default();
        let _ = issuer
            .issue(
                &acme_spec(
                    AcmeChallengeSolver::Dns01 {
                        provider: DnsProvider::CaveDns {
                            zone: "example.com.".into(),
                        },
                    },
                    vec!["example.com".into()],
                ),
                &req("demo"),
            )
            .unwrap();
        let plan = issuer.dns_plans.values().next().unwrap();
        assert!(plan.record_name.starts_with("_acme-challenge."));
        assert_eq!(plan.digest.len(), 43); // base64url no-pad of sha-256
        assert!(!plan.digest.contains('='));
    }

    #[test]
    fn rejects_handle_without_keychain_scheme() {
        let mut issuer = AcmeIssuer::default();
        let mut spec = acme_spec(
            AcmeChallengeSolver::Http01 {
                ingress_class: None,
                service_type: None,
            },
            vec![],
        );
        if let IssuerSpec::Acme {
            account_key_keychain_handle,
            ..
        } = &mut spec
        {
            *account_key_keychain_handle = "plaintext".into();
        }
        let err = issuer.issue(&spec, &req("demo")).unwrap_err();
        assert!(matches!(err, CertManagerError::VaultKeychainScheme(_)));
    }

    #[test]
    fn longest_zone_wins_solver_selection() {
        let solvers = vec![
            AcmeSolver {
                dns_zones: vec!["example.com".into()],
                challenge: AcmeChallengeSolver::Http01 {
                    ingress_class: Some("default".into()),
                    service_type: None,
                },
            },
            AcmeSolver {
                dns_zones: vec!["api.example.com".into()],
                challenge: AcmeChallengeSolver::Dns01 {
                    provider: DnsProvider::CaveDns {
                        zone: "api.example.com.".into(),
                    },
                },
            },
        ];
        let picked = pick_solver(&solvers, "api.example.com").unwrap();
        assert!(matches!(picked.challenge, AcmeChallengeSolver::Dns01 { .. }));
    }

    #[test]
    fn empty_zone_solver_is_catch_all() {
        let solvers = vec![AcmeSolver {
            dns_zones: vec![],
            challenge: AcmeChallengeSolver::Http01 {
                ingress_class: None,
                service_type: None,
            },
        }];
        assert!(pick_solver(&solvers, "anything.example.com").is_some());
    }

    #[test]
    fn webhook_dns_provider_is_rejected_as_phase_two() {
        let mut issuer = AcmeIssuer::default();
        let err = issuer
            .issue(
                &acme_spec(
                    AcmeChallengeSolver::Dns01 {
                        provider: DnsProvider::Webhook {
                            group_name: "acme.example.com".into(),
                            solver_name: "my-solver".into(),
                        },
                    },
                    vec![],
                ),
                &req("demo"),
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::AcmeChallenge { .. }));
    }

    #[test]
    fn rejects_empty_dns_names() {
        let mut issuer = AcmeIssuer::default();
        let mut r = req("demo");
        r.dns_names.clear();
        let err = issuer
            .issue(
                &acme_spec(
                    AcmeChallengeSolver::Http01 {
                        ingress_class: None,
                        service_type: None,
                    },
                    vec![],
                ),
                &r,
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::EmptyDnsNames));
    }

    #[test]
    fn key_authorization_format_matches_rfc8555() {
        let jwk = synthetic_jwk("keychain:abc", "https://acme.example/d");
        let ka = key_authorization("abc-token", &jwk);
        let mut parts = ka.split('.');
        assert_eq!(parts.next().unwrap(), "abc-token");
        assert_eq!(parts.next().unwrap().len(), 43);
        assert!(parts.next().is_none());
    }

    #[test]
    fn no_matching_solver_errors() {
        let mut issuer = AcmeIssuer::default();
        let spec = IssuerSpec::Acme {
            directory_url: "https://acme.example/d".into(),
            account_key_keychain_handle: "keychain:cave-acme-account".into(),
            email: vec![],
            terms_of_service_agreed: true,
            solvers: vec![AcmeSolver {
                dns_zones: vec!["other.example".into()],
                challenge: AcmeChallengeSolver::Http01 {
                    ingress_class: None,
                    service_type: None,
                },
            }],
        };
        let err = issuer.issue(&spec, &req("demo")).unwrap_err();
        assert!(matches!(err, CertManagerError::AcmeOrder(_)));
    }
}
