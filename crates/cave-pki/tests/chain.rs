//! cave-pki — chain validation tests.

use cave_pki::{Ca, ChainValidator, CrlResponder, KeyAlgorithm, RevocationReason, ValidationResult};
use chrono::{Duration, Utc};

const TENANT: &str = "tenant-acme-prod";

fn primed_ca() -> (Ca, String) {
    let mut ca = Ca::new();
    ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20).unwrap();
    ca.generate_platform_intermediate("Cave Platform CA", KeyAlgorithm::EcdsaP384).unwrap();
    let tenant_serial = ca.generate_tenant_intermediate(TENANT, KeyAlgorithm::EcdsaP256).unwrap();
    (ca, tenant_serial)
}

/// Cite: RFC 5280 §6.1 — a chain that walks Tenant → Platform → Root
/// validates with depth = 3 and the root as trust anchor.
#[test]
fn full_three_tier_chain_validates_with_correct_depth() {
    let (ca, tenant_serial) = primed_ca();
    let v = ChainValidator::new(&ca);
    let result = v.validate(&tenant_serial).unwrap();
    match result {
        ValidationResult::Valid { trust_anchor, depth } => {
            assert_eq!(trust_anchor, ca.root_serial().unwrap());
            assert_eq!(depth, 3, "tenant + platform + root");
        }
        other => panic!("expected Valid, got {:?}", other),
    }
}

/// Cite: RFC 5280 §6.1.3 — a cert past `notAfter` must fail validation
/// with an explicit "expired" reason.
#[test]
fn expired_chain_fails_with_explicit_reason() {
    let (ca, tenant_serial) = primed_ca();
    let validator = ChainValidator::new(&ca)
        .at(Utc::now() + Duration::days(365 * 30));  // far past root expiry
    match validator.validate(&tenant_serial).unwrap() {
        ValidationResult::Invalid(reason) => assert!(reason.contains("expired")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

/// Cite: RFC 5280 §5 — a revoked intermediate fails validation with a
/// `Revoked` error rather than reporting a chain-walk failure.
#[test]
fn revoked_tenant_intermediate_fails_validation_with_revoked_error() {
    let (ca, tenant_serial) = primed_ca();
    let mut crl = CrlResponder::new();
    crl.revoke(&tenant_serial, RevocationReason::KeyCompromise, TENANT);

    let v = ChainValidator::new(&ca).with_crl(&crl);
    let err = v.validate(&tenant_serial).unwrap_err();
    let s = err.to_string();
    assert!(s.contains("revoked"));
    assert!(s.contains(&tenant_serial));
}
