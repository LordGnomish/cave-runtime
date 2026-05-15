// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for SAML XML DSig ECDSA-{SHA256, SHA384} sign + verify.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/api/saml/v2/sig/SAML2Signature.java
//!         saml-core-api/src/main/java/org/keycloak/saml/SignatureAlgorithm.java
//!
//! W3C XML DSig algorithm URNs:
//!  * ecdsa-sha256 = http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256
//!  * ecdsa-sha384 = http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384

use cave_auth::saml::signature::{
    ecdsa_p256_generate_pkcs8, ecdsa_p384_generate_pkcs8, sign_ecdsa_sha256,
    sign_ecdsa_sha384, verify_signature_ecdsa_sha256, verify_signature_ecdsa_sha384,
    SignedDocument, ALG_ECDSA_SHA256, ALG_ECDSA_SHA384,
};

#[test]
fn ecdsa_sha256_alg_urn_is_w3c_value() {
    assert_eq!(
        ALG_ECDSA_SHA256,
        "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256"
    );
}

#[test]
fn ecdsa_sha384_alg_urn_is_w3c_value() {
    assert_eq!(
        ALG_ECDSA_SHA384,
        "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384"
    );
}

#[test]
fn ecdsa_p256_sign_then_verify_round_trips() {
    let (pkcs8, spki_der) = ecdsa_p256_generate_pkcs8();
    let doc = SignedDocument::new(b"<saml:Assertion>p256-payload</saml:Assertion>");
    let sig = sign_ecdsa_sha256(&doc, &pkcs8).unwrap();
    verify_signature_ecdsa_sha256(&doc, &sig, &spki_der).unwrap();
}

#[test]
fn ecdsa_p256_verify_rejects_tampered_payload() {
    let (pkcs8, spki_der) = ecdsa_p256_generate_pkcs8();
    let doc_a = SignedDocument::new(b"<saml:Assertion>original</saml:Assertion>");
    let sig = sign_ecdsa_sha256(&doc_a, &pkcs8).unwrap();
    let doc_b = SignedDocument::new(b"<saml:Assertion>tampered</saml:Assertion>");
    assert!(verify_signature_ecdsa_sha256(&doc_b, &sig, &spki_der).is_err());
}

#[test]
fn ecdsa_p384_sign_then_verify_round_trips() {
    let (pkcs8, spki_der) = ecdsa_p384_generate_pkcs8();
    let doc = SignedDocument::new(b"<saml:Assertion>p384-payload</saml:Assertion>");
    let sig = sign_ecdsa_sha384(&doc, &pkcs8).unwrap();
    verify_signature_ecdsa_sha384(&doc, &sig, &spki_der).unwrap();
}

#[test]
fn ecdsa_p384_verify_rejects_tampered_payload() {
    let (pkcs8, spki_der) = ecdsa_p384_generate_pkcs8();
    let doc_a = SignedDocument::new(b"<saml:Assertion>original</saml:Assertion>");
    let sig = sign_ecdsa_sha384(&doc_a, &pkcs8).unwrap();
    let doc_b = SignedDocument::new(b"<saml:Assertion>tampered</saml:Assertion>");
    assert!(verify_signature_ecdsa_sha384(&doc_b, &sig, &spki_der).is_err());
}

#[test]
fn ecdsa_p256_verify_rejects_bad_base64() {
    let (_, spki_der) = ecdsa_p256_generate_pkcs8();
    let doc = SignedDocument::new(b"x");
    assert!(verify_signature_ecdsa_sha256(&doc, "!not!b64!", &spki_der).is_err());
}

#[test]
fn ecdsa_p384_verify_rejects_bad_base64() {
    let (_, spki_der) = ecdsa_p384_generate_pkcs8();
    let doc = SignedDocument::new(b"x");
    assert!(verify_signature_ecdsa_sha384(&doc, "!not!b64!", &spki_der).is_err());
}
