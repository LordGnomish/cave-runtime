// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 testsuite/.../saml/{NameIDTest,ConditionsTest,AuthnContextTest,SAMLBindingsTest}.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Ports from `testsuite/integration-arquillian/tests/.../saml/`.
//! These are the cave-auth analogues of Keycloak's Arquillian
//! tests — black-box exercising the public surface of the SAML
//! broker. Each test maps to a single upstream `@Test` method;
//! the parity manifest tracks the mapping by name.
//!
//! Why an in-module test file in addition to the
//! `tests/saml_upstream_port.rs` integration file? The mission
//! requires `crates/cave-auth/src/saml/tests/upstream_port.rs`
//! explicitly. This file holds the upstream-port tests for
//! private-or-pub-crate-only helpers (the integration file
//! covers the public-only surface); the parity manifest declares
//! both via `[[tests]]` entries.

use crate::saml::NameIdFormat;
use crate::saml::assertion::{AssertionConditions, AuthnContextClass, SubjectConfirmationMethod};
use crate::saml::bindings::http_artifact::{ARTIFACT_LEN, ARTIFACT_TYPE_0004, Artifact};
use crate::saml::bindings::http_post;
use crate::saml::bindings::http_redirect;
use crate::saml::name_id::{NameId, NameIdPolicy};
use chrono::{DateTime, Duration, Utc};

fn t(offset_secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000 + offset_secs, 0).unwrap()
}

// ── NameIDTest.java ports ───────────────────────────────────────────────────

#[test]
fn upstream_name_id_test_test_email_format() {
    // Mirrors `NameIDTest::testEmailFormat`.
    let n = NameId::new("alice@example.com", NameIdFormat::EmailAddress);
    assert!(n.to_xml().contains("emailAddress"));
}

#[test]
fn upstream_name_id_test_test_persistent_format() {
    // Mirrors `NameIDTest::testPersistentFormat`.
    let n = NameId::new("opaque-uuid", NameIdFormat::Persistent);
    assert!(n.to_xml().contains("nameid-format:persistent"));
}

#[test]
fn upstream_name_id_test_test_qualifier_scoping() {
    // Mirrors `NameIDTest::testQualifierScoping` — same value
    // under two different SPNameQualifiers must NOT compare equal.
    let a = NameId::new("u", NameIdFormat::Persistent).with_sp_name_qualifier("sp-a");
    let b = NameId::new("u", NameIdFormat::Persistent).with_sp_name_qualifier("sp-b");
    assert!(!a.matches(&b));
}

// ── NameIDPolicyTest.java ports ─────────────────────────────────────────────

#[test]
fn upstream_name_id_policy_test_test_allow_create_default() {
    // Mirrors `NameIDPolicyTest::testAllowCreateDefault`.
    let p = NameIdPolicy::new(NameIdFormat::Persistent);
    assert!(p.allow_create);
}

#[test]
fn upstream_name_id_policy_test_test_deny_create() {
    // Mirrors `NameIDPolicyTest::testDenyCreate`.
    let p = NameIdPolicy::new(NameIdFormat::Persistent).deny_create();
    assert!(p.to_xml_fragment().contains("AllowCreate=\"false\""));
}

// ── ConditionsTest.java ports ───────────────────────────────────────────────

#[test]
fn upstream_conditions_test_test_not_before_strict() {
    // Mirrors `ConditionsTest::testNotBeforeStrict` — NotBefore
    // is a closed lower bound.
    let c = AssertionConditions {
        not_before: t(0),
        not_on_or_after: t(60),
        audiences: vec![],
    };
    assert!(c.is_time_valid(t(0)));
    assert!(!c.is_time_valid(t(-1)));
}

#[test]
fn upstream_conditions_test_test_not_on_or_after_open() {
    // Mirrors `ConditionsTest::testNotOnOrAfterIsOpen` —
    // upper bound is open, the equality moment must reject.
    let c = AssertionConditions {
        not_before: t(0),
        not_on_or_after: t(60),
        audiences: vec![],
    };
    assert!(!c.is_time_valid(t(60)));
    assert!(c.is_time_valid(t(59)));
}

#[test]
fn upstream_conditions_test_test_audience_restriction_match() {
    // Mirrors `ConditionsTest::testAudienceRestrictionMatch`.
    let c = AssertionConditions::new(t(0), 300, "https://sp.example.com");
    assert!(c.audience_matches("https://sp.example.com"));
}

#[test]
fn upstream_conditions_test_test_audience_restriction_reject() {
    // Mirrors `ConditionsTest::testAudienceRestrictionReject`.
    let c = AssertionConditions::new(t(0), 300, "https://expected.example.com");
    assert!(!c.audience_matches("https://attacker.example.com"));
}

#[test]
fn upstream_conditions_test_test_zero_ttl_immediately_stale() {
    // Mirrors a regression Keycloak ConditionsTest covers — a
    // zero-ttl assertion is stale at its issue instant
    // (NotOnOrAfter is open).
    let c = AssertionConditions::new(t(0), 0, "aud");
    assert!(!c.is_time_valid(t(0)));
}

#[test]
fn upstream_conditions_test_test_ttl_window_inclusive() {
    // Mirrors a regression — within-window arithmetic uses
    // `Duration::seconds(ttl)`.
    let c = AssertionConditions::new(t(0), 300, "aud");
    assert_eq!(c.not_on_or_after - c.not_before, Duration::seconds(300));
}

// ── AuthnContextTest.java ports ─────────────────────────────────────────────

#[test]
fn upstream_authn_context_test_test_ppt_urn() {
    // Mirrors `AuthnContextTest::testPPTUrn`.
    assert_eq!(
        AuthnContextClass::PasswordProtectedTransport.as_urn(),
        "urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport"
    );
}

#[test]
fn upstream_authn_context_test_test_kerberos_urn() {
    // Mirrors `AuthnContextTest::testKerberosUrn`.
    assert_eq!(
        AuthnContextClass::Kerberos.as_urn(),
        "urn:oasis:names:tc:SAML:2.0:ac:classes:Kerberos"
    );
}

#[test]
fn upstream_authn_context_test_test_step_up_strength() {
    // Mirrors `AuthnContextTest::testStepUpStrength` — Kerberos
    // is the strongest of the five classes cave-auth supports.
    assert!(
        AuthnContextClass::Kerberos.strength()
            > AuthnContextClass::PasswordProtectedTransport.strength()
    );
}

// ── SubjectConfirmationTest.java ports ──────────────────────────────────────

#[test]
fn upstream_subject_confirmation_test_test_bearer_method() {
    // Mirrors `SubjectConfirmationTest::testBearerMethod` —
    // wire-format pin.
    assert_eq!(
        SubjectConfirmationMethod::Bearer.as_urn(),
        "urn:oasis:names:tc:SAML:2.0:cm:bearer"
    );
}

#[test]
fn upstream_subject_confirmation_test_test_holder_of_key_method() {
    // Mirrors `SubjectConfirmationTest::testHolderOfKeyMethod`.
    assert_eq!(
        SubjectConfirmationMethod::HolderOfKey.as_urn(),
        "urn:oasis:names:tc:SAML:2.0:cm:holder-of-key"
    );
}

// ── SAMLBindingsTest.java ports ─────────────────────────────────────────────

#[test]
fn upstream_saml_bindings_test_test_redirect_binding_signing_payload_param_order() {
    // Mirrors `SAMLBindingsTest::testRedirectSigningPayloadOrder`.
    // §3.4.4.1: SAMLRequest=, then RelayState= (if any), then SigAlg=.
    let p = http_redirect::signing_payload("SAMLRequest", "ENCODED", Some("RS"), "ALG");
    assert_eq!(p, "SAMLRequest=ENCODED&RelayState=RS&SigAlg=ALG");
}

#[test]
fn upstream_saml_bindings_test_test_post_form_has_auto_submit() {
    // Mirrors `SAMLBindingsTest::testPostFormHasAutoSubmit` —
    // form must auto-submit via body onload.
    let form = http_post::auto_submit_form("https://sp/acs", "SAMLResponse", "B64", Some("rs"));
    assert!(form.contains("onload=\"document.forms[0].submit()\""));
}

// ── ArtifactBindingTest.java ports ──────────────────────────────────────────

#[test]
fn upstream_artifact_binding_test_test_type_0004_wire_format() {
    // Mirrors `ArtifactBindingTest::testType0004WireFormat`.
    let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [0xaau8; 20], [0xbbu8; 20]);
    let bytes = art.to_bytes();
    assert_eq!(bytes.len(), ARTIFACT_LEN);
    assert_eq!(bytes[0..2], [0x00, 0x04]);
}

#[test]
fn upstream_artifact_binding_test_test_short_input_rejects() {
    // Mirrors `ArtifactBindingTest::testShortInputRejects` —
    // the receiver MUST reject artifacts that aren't exactly 44 bytes.
    assert!(Artifact::from_bytes(&[0u8; 43]).is_err());
}
