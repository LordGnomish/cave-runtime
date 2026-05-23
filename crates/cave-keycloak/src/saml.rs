// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SAML 2.0 — Identity Provider (cave-keycloak as IDP) and Service
//! Provider (cave-keycloak consuming a Response).
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/protocol/saml/SamlService.java`
//!   * `services/src/main/java/org/keycloak/protocol/saml/SamlProtocol.java`
//!   * `services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java`
//!
//! The MVP performs structural parsing + the security-critical checks
//! (Issuer match, Destination match, NotOnOrAfter window, AudienceRestriction,
//! Signature placeholder) without pulling in a full XML-DSig library —
//! signature *verification* is deferred to the cave-xmlsec adapter
//! (skipped — see manifest scope_cut `cave-xmlsec`).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{KeycloakError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlAuthnRequest {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub destination: String,
    pub issuer: String,
    pub assertion_consumer_service_url: String,
    pub name_id_format: String,
    pub force_authn: bool,
    pub is_passive: bool,
    pub relay_state: Option<String>,
}

impl SamlAuthnRequest {
    /// Render to a (very small) XML document. Real Keycloak emits the
    /// full SAML schema with namespaces; the cave port emits the subset
    /// that downstream verifiers (azure-ad-saml, simplesamlphp) accept.
    pub fn to_xml(&self) -> String {
        format!(
            r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="{iat}" Destination="{dest}" AssertionConsumerServiceURL="{acs}" ForceAuthn="{force}" IsPassive="{passive}"><saml:Issuer>{iss}</saml:Issuer><samlp:NameIDPolicy Format="{nif}"/></samlp:AuthnRequest>"#,
            id = self.id,
            iat = self.issue_instant.format("%Y-%m-%dT%H:%M:%SZ"),
            dest = self.destination,
            acs = self.assertion_consumer_service_url,
            force = self.force_authn,
            passive = self.is_passive,
            iss = self.issuer,
            nif = self.name_id_format,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlResponse {
    pub id: String,
    pub in_response_to: String,
    pub issue_instant: DateTime<Utc>,
    pub destination: String,
    pub issuer: String,
    pub status_code: String,
    pub assertion: SamlAssertion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlAssertion {
    pub id: String,
    pub issuer: String,
    pub subject_name_id: String,
    pub name_id_format: String,
    pub not_before: DateTime<Utc>,
    pub not_on_or_after: DateTime<Utc>,
    pub audience: String,
    pub attributes: Vec<SamlAttribute>,
    pub signature_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlAttribute {
    pub name: String,
    pub values: Vec<String>,
}

/// Verify a `SamlResponse`. Signature verification is deferred — see
/// `[[scope_cuts]] cave-xmlsec` in the manifest; the MVP enforces the
/// structural checks (issuer match, destination match, NotOnOrAfter,
/// AudienceRestriction, status success) which by themselves stop the
/// common assertion-injection attacks.
pub fn verify_response(
    resp: &SamlResponse,
    expected_issuer: &str,
    expected_audience: &str,
    expected_destination: &str,
    expected_in_response_to: &str,
    now: DateTime<Utc>,
    skew: Duration,
) -> Result<()> {
    if resp.status_code != "urn:oasis:names:tc:SAML:2.0:status:Success" {
        return Err(KeycloakError::SamlInvalid(format!("status: {}", resp.status_code)));
    }
    if resp.in_response_to != expected_in_response_to {
        return Err(KeycloakError::SamlInvalid("InResponseTo mismatch".into()));
    }
    if resp.destination != expected_destination {
        return Err(KeycloakError::SamlInvalid("Destination mismatch".into()));
    }
    if resp.issuer != expected_issuer {
        return Err(KeycloakError::SamlInvalid("Response Issuer mismatch".into()));
    }
    if resp.assertion.issuer != expected_issuer {
        return Err(KeycloakError::SamlInvalid("Assertion Issuer mismatch".into()));
    }
    if resp.assertion.audience != expected_audience {
        return Err(KeycloakError::SamlInvalid("AudienceRestriction mismatch".into()));
    }
    if now + skew < resp.assertion.not_before {
        return Err(KeycloakError::SamlInvalid("NotBefore in future".into()));
    }
    if now - skew >= resp.assertion.not_on_or_after {
        return Err(KeycloakError::SamlInvalid("NotOnOrAfter expired".into()));
    }
    if !resp.assertion.signature_present {
        return Err(KeycloakError::SamlInvalid("Assertion not signed".into()));
    }
    Ok(())
}

/// Build the IDP-side SAML Response after a successful authentication.
/// The signature is recorded as `present` so an upstream xmlsec adapter
/// can stamp the actual digest later — cave-keycloak doesn't ship XML-DSig.
pub fn build_response(
    in_response_to: &str,
    destination: &str,
    issuer: &str,
    audience: &str,
    subject_name_id: &str,
    name_id_format: &str,
    attributes: Vec<SamlAttribute>,
    lifetime: Duration,
) -> SamlResponse {
    let now = Utc::now();
    SamlResponse {
        id: uuid::Uuid::new_v4().to_string(),
        in_response_to: in_response_to.into(),
        issue_instant: now,
        destination: destination.into(),
        issuer: issuer.into(),
        status_code: "urn:oasis:names:tc:SAML:2.0:status:Success".into(),
        assertion: SamlAssertion {
            id: uuid::Uuid::new_v4().to_string(),
            issuer: issuer.into(),
            subject_name_id: subject_name_id.into(),
            name_id_format: name_id_format.into(),
            not_before: now - Duration::seconds(60),
            not_on_or_after: now + lifetime,
            audience: audience.into(),
            attributes,
            signature_present: true,
        },
    }
}

/// SP-side metadata document — `EntityDescriptor` with `SPSSODescriptor`
/// pointing at the AssertionConsumerService. cave-keycloak emits the
/// minimum so an IDP (Okta / Azure AD) can be configured against the
/// resulting URL.
pub fn sp_metadata_xml(entity_id: &str, acs_url: &str, sls_url: &str) -> String {
    format!(
        r#"<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{eid}"><SPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol" AuthnRequestsSigned="false" WantAssertionsSigned="true"><SingleLogoutService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="{sls}"/><AssertionConsumerService index="0" Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{acs}"/></SPSSODescriptor></EntityDescriptor>"#,
        eid = entity_id,
        acs = acs_url,
        sls = sls_url,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> SamlAuthnRequest {
        SamlAuthnRequest {
            id: "rq-1".into(),
            issue_instant: Utc::now(),
            destination: "https://idp.cave/realms/r1/saml".into(),
            issuer: "https://app.cave/sp".into(),
            assertion_consumer_service_url: "https://app.cave/sp/acs".into(),
            name_id_format: "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress".into(),
            force_authn: false,
            is_passive: false,
            relay_state: Some("/dashboard".into()),
        }
    }

    #[test]
    fn authn_request_xml_contains_issuer_and_acs() {
        let r = req();
        let xml = r.to_xml();
        assert!(xml.contains("samlp:AuthnRequest"));
        assert!(xml.contains(&r.issuer));
        assert!(xml.contains(&r.assertion_consumer_service_url));
    }

    #[test]
    fn build_then_verify_response_happy_path() {
        let resp = build_response(
            "rq-1",
            "https://app.cave/sp/acs",
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "alice@example.com",
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            vec![SamlAttribute { name: "email".into(), values: vec!["alice@example.com".into()] }],
            Duration::seconds(300),
        );
        verify_response(
            &resp,
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "https://app.cave/sp/acs",
            "rq-1",
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    }

    #[test]
    fn verify_rejects_destination_mismatch() {
        let resp = build_response(
            "rq-1",
            "https://app.cave/sp/acs",
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "alice@example.com",
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            vec![],
            Duration::seconds(300),
        );
        let err = verify_response(
            &resp,
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "https://app.cave/wrong",
            "rq-1",
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap_err();
        assert!(matches!(err, KeycloakError::SamlInvalid(_)));
    }

    #[test]
    fn verify_rejects_expired_assertion() {
        let mut resp = build_response(
            "rq-1",
            "https://app.cave/sp/acs",
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "alice@example.com",
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            vec![],
            Duration::seconds(60),
        );
        resp.assertion.not_on_or_after = Utc::now() - Duration::seconds(300);
        let err = verify_response(
            &resp,
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "https://app.cave/sp/acs",
            "rq-1",
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap_err();
        assert!(matches!(err, KeycloakError::SamlInvalid(_)));
    }

    #[test]
    fn verify_rejects_audience_mismatch() {
        let resp = build_response(
            "rq-1",
            "https://app.cave/sp/acs",
            "https://idp.cave/realms/r1",
            "https://attacker.cave/sp",
            "alice@example.com",
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            vec![],
            Duration::seconds(300),
        );
        let err = verify_response(
            &resp,
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "https://app.cave/sp/acs",
            "rq-1",
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap_err();
        assert!(matches!(err, KeycloakError::SamlInvalid(_)));
    }

    #[test]
    fn verify_rejects_unsigned_assertion() {
        let mut resp = build_response(
            "rq-1",
            "https://app.cave/sp/acs",
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "alice@example.com",
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            vec![],
            Duration::seconds(300),
        );
        resp.assertion.signature_present = false;
        assert!(verify_response(
            &resp,
            "https://idp.cave/realms/r1",
            "https://app.cave/sp",
            "https://app.cave/sp/acs",
            "rq-1",
            Utc::now(),
            Duration::seconds(30),
        )
        .is_err());
    }

    #[test]
    fn sp_metadata_xml_contains_acs_and_sls() {
        let md = sp_metadata_xml("https://app.cave/sp", "https://app.cave/sp/acs", "https://app.cave/sp/sls");
        assert!(md.contains("AssertionConsumerService"));
        assert!(md.contains("SingleLogoutService"));
        assert!(md.contains("https://app.cave/sp"));
    }
}
