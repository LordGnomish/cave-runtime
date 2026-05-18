// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-certs — renewal controller tests.

use cave_certs::crds::{CertificateSpec, CertificateStatus, IssuerRef};
use cave_certs::renewal::{RenewalController, RenewalDecision};
use chrono::{Duration, Utc};

const TENANT: &str = "tenant-acme-prod";

fn spec() -> CertificateSpec {
    CertificateSpec::new(
        TENANT,
        format!("{}-tls", TENANT),
        IssuerRef::issuer("letsencrypt-prod"),
        vec![format!("svc.{}.cave-runtime.test", TENANT)],
    )
}

/// Cite: cert-manager v1.20.2
/// `pkg/controller/certificates/trigger::shouldReissue` —
/// the renewal trigger fires exactly when `now >= notAfter - renewBefore`.
#[test]
fn renewal_decision_covers_all_four_states() {
    let s = spec();
    let ctl = RenewalController::new(TENANT);
    let now = Utc::now();

    // 1. NeedsInitialIssuance — no notAfter
    let status_initial = CertificateStatus::default();
    assert_eq!(ctl.evaluate(&s, &status_initial, now), RenewalDecision::NeedsInitialIssuance);

    // 2. NotYet — cert valid, well outside renewal window
    let mut fresh = CertificateStatus::default();
    fresh.not_before = Some(now - Duration::days(1));
    fresh.not_after = Some(now + Duration::days(89));   // 89d remaining; window = 30d
    let dec = ctl.evaluate(&s, &fresh, now);
    assert!(matches!(dec, RenewalDecision::NotYet { .. }));

    // 3. Due — inside renewal window
    let mut due = CertificateStatus::default();
    due.not_before = Some(now - Duration::days(60));
    due.not_after = Some(now + Duration::days(15));     // 15d remaining < 30d ⇒ due
    assert_eq!(ctl.evaluate(&s, &due, now), RenewalDecision::Due);

    // 4. Expired — past notAfter
    let mut expired = CertificateStatus::default();
    expired.not_before = Some(now - Duration::days(120));
    expired.not_after  = Some(now - Duration::days(1));
    assert_eq!(ctl.evaluate(&s, &expired, now), RenewalDecision::Expired);
}

/// Cite: cave multi-tenant invariant — a controller scoped to tenant
/// A MUST NOT decide on behalf of tenant B.
#[test]
fn renewal_controller_ignores_cross_tenant_specs() {
    let mut s = spec();
    s.tenant_id = "tenant-other".into();
    let ctl = RenewalController::new(TENANT);
    let now = Utc::now();
    let mut due = CertificateStatus::default();
    due.not_after = Some(now + Duration::days(15));
    let dec = ctl.evaluate(&s, &due, now);
    // Cross-tenant ⇒ NotYet (no-op) regardless of the actual due time.
    assert!(matches!(dec, RenewalDecision::NotYet { .. }));
}
