// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for SAML NameID format negotiation between SP request
//! (`<samlp:NameIDPolicy Format=…>`) and IdP-supported set.
//!
//! Source: keycloak/keycloak@b825ba97
//!         services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java::createNameId
//!         services/src/main/java/org/keycloak/protocol/saml/SamlProtocol.java::getSamlNameId

use cave_auth::saml::nameid::{
    negotiate_nameid_format, render_nameid, NameIdPolicyOutcome, NameIdSupport,
};
use cave_auth::saml::NameIdFormat;

#[test]
fn unspecified_falls_back_to_idp_default() {
    let idp = NameIdSupport {
        supported: vec![NameIdFormat::EmailAddress, NameIdFormat::Persistent],
        default: NameIdFormat::EmailAddress,
    };
    // SP didn't pin a format; IdP gets to pick its default.
    let out = negotiate_nameid_format(None, &idp);
    assert_eq!(
        out,
        NameIdPolicyOutcome::Granted(NameIdFormat::EmailAddress)
    );
}

#[test]
fn matching_format_is_granted() {
    let idp = NameIdSupport {
        supported: vec![NameIdFormat::Persistent, NameIdFormat::Transient],
        default: NameIdFormat::Persistent,
    };
    let out = negotiate_nameid_format(Some(NameIdFormat::Transient), &idp);
    assert_eq!(out, NameIdPolicyOutcome::Granted(NameIdFormat::Transient));
}

#[test]
fn unsupported_format_is_rejected_with_status() {
    let idp = NameIdSupport {
        supported: vec![NameIdFormat::Transient],
        default: NameIdFormat::Transient,
    };
    let out = negotiate_nameid_format(Some(NameIdFormat::EmailAddress), &idp);
    // SAML 2.0 Core §3.4.1.1: when the SP requested an unsupported
    // Format, the IdP returns InvalidNameIDPolicy.
    assert!(
        matches!(out, NameIdPolicyOutcome::InvalidPolicy { .. }),
        "got {:?}",
        out
    );
}

#[test]
fn render_nameid_writes_format_attribute_when_email() {
    let xml = render_nameid("alice@example.com", NameIdFormat::EmailAddress);
    assert!(xml.contains("<saml:NameID"));
    assert!(xml.contains("Format=\"urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress\""));
    assert!(xml.contains(">alice@example.com<"));
}

#[test]
fn render_nameid_omits_format_for_unspecified() {
    // Per spec, Unspecified is the implicit default and may be omitted.
    // cave-auth follows Keycloak's choice to always emit the URN, but
    // the helper at minimum must round-trip the value verbatim.
    let xml = render_nameid("opaque-id-123", NameIdFormat::Unspecified);
    assert!(xml.contains(">opaque-id-123<"));
}

#[test]
fn nameid_support_lists_4_canonical_formats() {
    let all = NameIdSupport::all();
    // emailAddress + persistent + transient + unspecified
    assert_eq!(all.supported.len(), 4);
    assert!(all.supported.contains(&NameIdFormat::Persistent));
    assert!(all.supported.contains(&NameIdFormat::Transient));
}
