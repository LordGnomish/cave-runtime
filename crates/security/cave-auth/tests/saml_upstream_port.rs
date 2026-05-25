// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 testsuite/integration-arquillian/tests/.../saml/SAMLBindingsTest.java + AssertionTest.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Integration scenarios ported from Keycloak's SAML testsuite —
//! exercises the broker flow end-to-end through the public surface
//! of `cave_auth::saml::{name_id, assertion, bindings}`.
//!
//! These are deliberately black-box: each scenario starts from the
//! public surface and verifies an end-to-end behaviour Keycloak's
//! Arquillian integration tests assert on. They run against the
//! library API, not against the existing in-module unit tests.

use cave_auth::saml::NameIdFormat;
use cave_auth::saml::assertion::{AssertionConditions, AuthnContextClass};
use cave_auth::saml::bindings::{
    BINDING_ARTIFACT, BINDING_POST, BINDING_REDIRECT, http_artifact, http_post, http_redirect,
};
use cave_auth::saml::name_id::{NameId, NameIdPolicy};
use chrono::{Duration, Utc};

/// Scenario 1 (ports `SAMLBindingsTest::testHttpRedirectRoundtrip`):
/// an SP can encode an AuthnRequest into the HTTP-Redirect binding
/// and the IdP receiver decodes it byte-for-byte.
#[test]
fn scenario_http_redirect_roundtrip() {
    let xml = br#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_a" Version="2.0"/>"#;
    let encoded = http_redirect::encode(xml).expect("encode");
    let decoded = http_redirect::decode(&encoded).expect("decode");
    assert_eq!(decoded, xml);
}

/// Scenario 2 (ports `SAMLBindingsTest::testHttpPostRoundtrip`):
/// the POST binding base64-encodes the message body and decodes it
/// back. The SAML 2.0 spec mandates standard base64, no DEFLATE.
#[test]
fn scenario_http_post_roundtrip() {
    let xml = br#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_r" Version="2.0"/>"#;
    let encoded = http_post::encode(xml);
    let decoded = http_post::decode(&encoded).expect("decode");
    assert_eq!(decoded, xml);
}

/// Scenario 3 (ports `SAMLBindingsTest::testArtifactReceiver`):
/// the Artifact binding gives the receiver a fixed-length opaque
/// reference, *not* the message itself. Source-ID + MessageHandle
/// round-trip through the type-0004 SAML 2.0 artifact format.
#[test]
fn scenario_http_artifact_format() {
    let source_id = [0xaau8; 20];
    let message_handle = [0xbbu8; 20];
    let art = http_artifact::Artifact::new(0x0004, 0, source_id, message_handle);
    let encoded = art.to_base64();
    let decoded = http_artifact::Artifact::from_base64(&encoded).expect("decode");
    assert_eq!(decoded.type_code, 0x0004);
    assert_eq!(decoded.source_id, source_id);
    assert_eq!(decoded.message_handle, message_handle);
}

/// Scenario 4 (ports `AssertionTest::testConditionsAudienceMatch`):
/// an Assertion whose AudienceRestriction names the SP entity ID
/// validates; one that names a different audience does not.
#[test]
fn scenario_assertion_audience_restriction_validates() {
    let now = Utc::now();
    let conds = AssertionConditions {
        not_before: now - Duration::seconds(60),
        not_on_or_after: now + Duration::seconds(60),
        audiences: vec!["https://sp.example.com/saml".to_string()],
    };
    assert!(conds.is_time_valid(now));
    assert!(conds.audience_matches("https://sp.example.com/saml"));
    assert!(!conds.audience_matches("https://other-sp.example.com"));
}

/// Scenario 5 (ports `AssertionTest::testAuthnContextClass`):
/// the AuthnContextClassRef on an Assertion drives step-up MFA
/// decisions in upstream. We at least round-trip the URN for the
/// classes Keycloak emits.
#[test]
fn scenario_authn_context_class_urn_roundtrip() {
    for class in [
        AuthnContextClass::PasswordProtectedTransport,
        AuthnContextClass::Password,
        AuthnContextClass::Kerberos,
        AuthnContextClass::PreviousSession,
        AuthnContextClass::Unspecified,
    ] {
        let urn = class.as_urn();
        assert_eq!(AuthnContextClass::from_urn(urn), Some(class));
    }
}

/// Scenario 6 (ports `SAMLEndpointTest::testNameIDPolicy`):
/// the SP can request a specific NameIDPolicy and the IdP echoes
/// it back. Verifies the policy serialisation produces the wire
/// shape Keycloak SAMLEndpoint reads.
#[test]
fn scenario_name_id_policy_wire_shape() {
    let policy = NameIdPolicy::new(NameIdFormat::Persistent)
        .with_sp_name_qualifier("https://sp.example.com/saml");
    let xml = policy.to_xml_fragment();
    assert!(xml.contains("Format=\""));
    assert!(xml.contains("AllowCreate=\"true\""));
    assert!(xml.contains("SPNameQualifier=\"https://sp.example.com/saml\""));
}

/// Scenario 7: binding URN constants match the SAML 2.0
/// `Bindings` schema. Regression — these are wire identifiers,
/// any drift breaks every SP integration.
#[test]
fn scenario_binding_urns_match_spec() {
    assert_eq!(
        BINDING_REDIRECT,
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect"
    );
    assert_eq!(
        BINDING_POST,
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
    );
    assert_eq!(
        BINDING_ARTIFACT,
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Artifact"
    );
}

/// Scenario 8 (ports `NameIDTest::testMatchesByQualifier`):
/// SAML §8.3.7 — two NameID values that compare equal must agree
/// on both Format and both Qualifiers. cave-auth follows the
/// same rule.
#[test]
fn scenario_name_id_matching_honors_qualifiers() {
    let a = NameId::new("u", NameIdFormat::Persistent).with_name_qualifier("idp-A");
    let b = NameId::new("u", NameIdFormat::Persistent).with_name_qualifier("idp-B");
    assert!(!a.matches(&b), "different NameQualifier must NOT match");
}
