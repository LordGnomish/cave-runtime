// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-certs — Certificate + Issuer CRD tests pinned to cert-manager v1.20.2.

use cave_certs::crds::{
    renewal_due_at, CertificateSpec, CertificateStatus, IssuerConfig, IssuerRef, IssuerSpec,
    PrivateKeyAlgorithm,
};
use chrono::{Duration, Utc};

const TENANT: &str = "tenant-acme-prod";

fn spec(dns: &[&str]) -> CertificateSpec {
    CertificateSpec::new(
        TENANT,
        format!("{}-tls", TENANT),
        IssuerRef::issuer("letsencrypt-prod"),
        dns.iter().map(|s| s.to_string()).collect(),
    )
}

/// Cite: cert-manager v1.20.2 `pkg/apis/certmanager/v1/types_certificate.go`
/// — defaults: 90d duration, 30d renewBefore, ECDSA P-256 key,
/// usages = [DigitalSignature, KeyEncipherment, ServerAuth].
#[test]
fn certificate_spec_default_values_match_cert_manager() {
    let s = spec(&[&format!("svc.{}.cave-runtime.test", TENANT)]);
    assert_eq!(s.duration_seconds, CertificateSpec::DEFAULT_DURATION_SECS);
    assert_eq!(s.duration_seconds, 90 * 86_400);
    assert_eq!(s.renew_before_seconds, 30 * 86_400);
    assert_eq!(s.private_key_algorithm, PrivateKeyAlgorithm::Ecdsa256);
    assert!(s.usages.contains(&cave_certs::crds::KeyUsage::ServerAuth));
}

/// Cite: cert-manager validation — at least one of dnsNames /
/// commonName MUST be set; renewBefore MUST be < duration.
#[test]
fn certificate_spec_validation_rejects_invalid_combinations() {
    // No DNS + no CN ⇒ rejected
    let mut s = spec(&[]);
    assert!(s.validate().is_err());

    s.common_name = Some(format!("svc.{}.cave-runtime.test", TENANT));
    assert!(s.validate().is_ok());

    // renewBefore >= duration ⇒ rejected
    s.renew_before_seconds = s.duration_seconds;
    assert!(s.validate().is_err());

    // Empty tenant ⇒ rejected
    s.renew_before_seconds = 30 * 86_400;
    s.tenant_id = "".into();
    assert!(s.validate().is_err());

    // Uppercase DNS name ⇒ rejected (cert-manager normalises but cave is strict)
    s.tenant_id = TENANT.into();
    s.dns_names = vec!["UPPER.example.com".into()];
    assert!(s.validate().is_err());
}

/// Cite: cert-manager IssuerConfig — exactly one of ca, acme, vault.
/// cave's enum encodes that as a tagged union; validation enforces
/// per-variant invariants (acme.email must contain '@', etc.).
#[test]
fn issuer_spec_validation_per_variant() {
    let ca = IssuerSpec {
        tenant_id: TENANT.into(),
        config: IssuerConfig::Ca { secret_name: "ca-tls".into() },
    };
    assert!(ca.validate().is_ok());

    let bad_ca = IssuerSpec {
        tenant_id: TENANT.into(),
        config: IssuerConfig::Ca { secret_name: "".into() },
    };
    assert!(bad_ca.validate().is_err());

    let acme = IssuerSpec {
        tenant_id: TENANT.into(),
        config: IssuerConfig::Acme {
            server: "https://acme-v02.api.letsencrypt.org/directory".into(),
            email: format!("ops@{}.cave-runtime.test", TENANT),
            private_key_secret_ref: "le-account-key".into(),
            external_account_binding_kid: None,
        },
    };
    assert!(acme.validate().is_ok());

    let bad_email = IssuerSpec {
        tenant_id: TENANT.into(),
        config: IssuerConfig::Acme {
            server: "https://acme.example.com/directory".into(),
            email: "no-at-sign".into(),
            private_key_secret_ref: "le-account-key".into(),
            external_account_binding_kid: None,
        },
    };
    assert!(bad_email.validate().is_err());

    let vault = IssuerSpec {
        tenant_id: TENANT.into(),
        config: IssuerConfig::Vault {
            server: "https://vault.cave-runtime.internal:8200".into(),
            path: "pki/tenant-acme-prod".into(),
            role: "web-server".into(),
        },
    };
    assert!(vault.validate().is_ok());
}

/// Cite: cert-manager controller `pkg/controller/certificates/trigger`
/// `shouldReissue` — renewal is due when `now >= notAfter - renewBefore`.
#[test]
fn renewal_due_at_returns_correct_window_open() {
    let s = spec(&[&format!("svc.{}.cave-runtime.test", TENANT)]);
    let mut status = CertificateStatus::default();

    // No notAfter ⇒ no due time.
    assert!(renewal_due_at(&s, &status).is_none());

    let now = Utc::now();
    status.not_before = Some(now - Duration::days(60));
    status.not_after  = Some(now + Duration::days(30));
    let due = renewal_due_at(&s, &status).unwrap();
    // notAfter is now+30d; renewBefore is 30d ⇒ due == now (give or take ms).
    let delta = (due - now).num_seconds().abs();
    assert!(delta < 5, "due should be ~now ± 5s, got {} seconds off", delta);
}

/// Cite: cert-manager IssuerRef — `kind` is one of `Issuer` /
/// `ClusterIssuer`; `group` defaults to `cert-manager.io`.
#[test]
fn issuer_ref_helpers_emit_canonical_kind_and_group() {
    let i = IssuerRef::issuer("letsencrypt-prod");
    assert_eq!(i.kind, "Issuer");
    assert_eq!(i.group, "cert-manager.io");
    assert_eq!(i.name, "letsencrypt-prod");

    let c = IssuerRef::cluster_issuer("ca-internal");
    assert_eq!(c.kind, "ClusterIssuer");
    assert_eq!(c.group, "cert-manager.io");
}
