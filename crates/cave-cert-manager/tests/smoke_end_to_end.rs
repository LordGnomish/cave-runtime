// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! End-to-end smoke — exercises the issuer + certificate fixtures the
//! task brief calls out:
//!   * SelfSigned cluster-issuer + Certificate fixture
//!   * ACME issuer with HTTP-01 mock challenge against a cave-acme
//!     in-memory directory
//!   * Renewal trigger (Certificate ages past renewBefore and the
//!     scheduler re-issues, bumping the revision)

use cave_cert_manager::acme_issuer::AcmeIssuer;
use cave_cert_manager::controller::{CertControlPlane, ReconcileEvent};
use cave_cert_manager::models::{
    AcmeChallengeSolver, AcmeSolver, Certificate, CertificateCondition,
    CertificateConditionType, CertificateSpec, CertificateStatus, ClusterIssuer, ConditionStatus,
    DnsProvider, IssuerRef, IssuerRefKind, IssuerSpec, PrivateKeyPolicy, SecretRef, Usage,
};
use cave_cert_manager::renewal::RenewalScheduler;
use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use uuid::Uuid;

fn fixture_cluster_issuer(name: &str, spec: IssuerSpec) -> ClusterIssuer {
    ClusterIssuer {
        id: Uuid::new_v4(),
        name: name.into(),
        tenant_id: "smoke-tenant".into(),
        spec,
        created_at: Utc::now(),
    }
}

fn fixture_certificate(name: &str, issuer: &str) -> Certificate {
    Certificate {
        id: Uuid::new_v4(),
        name: name.into(),
        namespace: "default".into(),
        tenant_id: "smoke-tenant".into(),
        spec: CertificateSpec {
            secret_name: format!("{}-tls", name),
            issuer_ref: IssuerRef {
                name: issuer.into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            dns_names: vec![format!("{}.smoke.example.com", name)],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 90 * 24 * 3600,
            renew_before_seconds: 30 * 24 * 3600,
            usages: vec![Usage::ServerAuth, Usage::DigitalSignature],
            private_key: PrivateKeyPolicy::default(),
            is_ca: false,
            subject: None,
            secret_template_labels: BTreeMap::new(),
            secret_template_annotations: BTreeMap::new(),
        },
        status: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
    }
}

#[test]
fn smoke_selfsigned_issues_and_renews_via_scheduler() {
    let mut cp = CertControlPlane::new();
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer(
            "selfsigned",
            IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
        ));
    let cert_id = cp.store.put_certificate(fixture_certificate("svc-a", "selfsigned"));

    // ── 1. Initial reconcile: emits Issued + writes Secret. ──────────
    let r1 = cp.controller().reconcile("smoke-tenant", cert_id).unwrap();
    assert_eq!(r1.new_revision, 1);
    assert!(matches!(r1.events[0], ReconcileEvent::Issued { .. }));

    // ── 2. Move cert's notAfter into the renewBefore window. ──────────
    {
        let cert = cp.store.certificate_mut("smoke-tenant", cert_id).unwrap();
        let now = Utc::now();
        // notAfter 10 days out → 10 < 30 (renewBefore) → due.
        let status = CertificateStatus {
            conditions: vec![CertificateCondition {
                kind: CertificateConditionType::Ready,
                status: ConditionStatus::True,
                reason: Some("Ready".into()),
                message: None,
                last_transition_time: now,
            }],
            serial: Some("aged-serial".into()),
            not_before: Some(now - Duration::seconds(60)),
            not_after: Some(now + Duration::days(10)),
            renewal_time: Some(now - Duration::days(1)),
            revision: 1,
            last_failure_message: None,
            secret_ref: Some(SecretRef {
                name: "svc-a-tls".into(),
                namespace: "default".into(),
            }),
        };
        cert.status = Some(status);
    }

    // ── 3. Scheduler picks the cert up; controller bumps to rev 2. ────
    let results = cp.controller().reconcile_due("smoke-tenant", Utc::now());
    assert_eq!(results.len(), 1);
    let r2 = results.into_iter().next().unwrap().unwrap();
    assert_eq!(r2.new_revision, 2);
    assert!(matches!(r2.events[0], ReconcileEvent::Renewed { .. }));
    let cert_ro = cp.store.certificate("smoke-tenant", cert_id).unwrap();
    assert_eq!(cert_ro.status.as_ref().unwrap().revision, 2);
}

#[test]
fn smoke_acme_http01_mock_challenge() {
    let acme_spec = IssuerSpec::Acme {
        directory_url: "https://acme-staging.smoke.example/directory".into(),
        account_key_keychain_handle: "keychain:smoke-acme-account".into(),
        email: vec!["ops@smoke.example".into()],
        terms_of_service_agreed: true,
        solvers: vec![AcmeSolver {
            dns_zones: vec![],
            challenge: AcmeChallengeSolver::Http01 {
                ingress_class: Some("cave-gw".into()),
                service_type: None,
            },
        }],
    };

    let mut cp = CertControlPlane::new();
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer("acme", acme_spec));
    let cert_id = cp.store.put_certificate(fixture_certificate("api", "acme"));

    let r = cp.controller().reconcile("smoke-tenant", cert_id).unwrap();
    assert_eq!(r.new_revision, 1);
    assert!(matches!(r.events[0], ReconcileEvent::Issued { .. }));
    // ACME issuer published exactly one HTTP-01 plan (one identifier).
    assert_eq!(cp.registry.acme.http_plans.len(), 1);
    let plan = cp.registry.acme.http_plans.values().next().unwrap();
    assert_eq!(plan.domain, "api.smoke.example.com");
    assert!(plan.key_authorization.contains('.'));
}

#[test]
fn smoke_acme_dns01_emits_digest_only() {
    let acme_spec = IssuerSpec::Acme {
        directory_url: "https://acme-staging.smoke.example/directory".into(),
        account_key_keychain_handle: "keychain:smoke-acme-account".into(),
        email: vec![],
        terms_of_service_agreed: true,
        solvers: vec![AcmeSolver {
            dns_zones: vec!["smoke.example.com".into()],
            challenge: AcmeChallengeSolver::Dns01 {
                provider: DnsProvider::CaveDns {
                    zone: "smoke.example.com.".into(),
                },
            },
        }],
    };
    let mut cp = CertControlPlane::new();
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer("acme-dns", acme_spec));
    let cert_id = cp.store.put_certificate(fixture_certificate("dns", "acme-dns"));
    let _ = cp.controller().reconcile("smoke-tenant", cert_id).unwrap();

    let plan = cp.registry.acme.dns_plans.values().next().unwrap();
    assert!(plan.record_name.starts_with("_acme-challenge."));
    assert_eq!(plan.digest.len(), 43); // base64url no-pad of sha-256
}

#[test]
fn smoke_renewal_scheduler_orders_by_renew_at_ascending() {
    // Build two certs with different times-to-renew and assert ordering.
    let mut cp = CertControlPlane::new();
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer(
            "selfsigned",
            IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
        ));
    let a_id = cp.store.put_certificate(fixture_certificate("alpha", "selfsigned"));
    let b_id = cp.store.put_certificate(fixture_certificate("beta", "selfsigned"));

    // Both certs have no status → both are "InitialIssuance" at `now`.
    let plans = RenewalScheduler.plan(
        &cp.store
            .list_certificates("smoke-tenant")
            .iter()
            .map(|c| (*c).clone())
            .collect::<Vec<_>>(),
        Utc::now(),
    );
    assert_eq!(plans.len(), 2);
    assert!(plans[0].renew_at <= plans[1].renew_at);

    // Drain via reconcile_due → both round-trip cleanly.
    let results = cp.controller().reconcile_due("smoke-tenant", Utc::now());
    assert_eq!(results.len(), 2);
    for r in &results {
        r.as_ref().unwrap();
    }
    // Both certs now have rev 1.
    assert_eq!(
        cp.store.certificate("smoke-tenant", a_id).unwrap().status.as_ref().unwrap().revision,
        1
    );
    assert_eq!(
        cp.store.certificate("smoke-tenant", b_id).unwrap().status.as_ref().unwrap().revision,
        1
    );
}

#[test]
fn smoke_acme_issuer_state_isolated_from_other_issuers() {
    // Ensure the issuer registry doesn't leak state across reconciles
    // for the wrong backend.
    let mut cp = CertControlPlane::new();
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer(
            "selfsigned",
            IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
        ));
    cp.store
        .put_cluster_issuer(fixture_cluster_issuer(
            "acme",
            IssuerSpec::Acme {
                directory_url: "https://acme/d".into(),
                account_key_keychain_handle: "keychain:k".into(),
                email: vec![],
                terms_of_service_agreed: true,
                solvers: vec![AcmeSolver {
                    dns_zones: vec![],
                    challenge: AcmeChallengeSolver::Http01 {
                        ingress_class: None,
                        service_type: None,
                    },
                }],
            },
        ));
    let s_id = cp.store.put_certificate(fixture_certificate("ss", "selfsigned"));
    let a_id = cp.store.put_certificate(fixture_certificate("ac", "acme"));
    let _ = cp.controller().reconcile("smoke-tenant", s_id).unwrap();
    let _ = cp.controller().reconcile("smoke-tenant", a_id).unwrap();
    // SelfSigned issuance does NOT touch the ACME server.
    // ACME issuance creates exactly one account + one HTTP-01 plan.
    assert_eq!(cp.registry.acme.http_plans.len(), 1);
}
