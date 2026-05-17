// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/dom/saml/v1/assertion/

//! SAML 1.1 Assertion — RED phase: tests authored, implementation lands in GREEN.

use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

use super::WsFedError;

pub const NS_SAML_1_1: &str = "urn:oasis:names:tc:SAML:1.0:assertion";
pub const NAMEID_EMAIL: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";
pub const NAMEID_UNSPECIFIED: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified";
pub const AUTHMETHOD_PASSWORD: &str = "urn:oasis:names:tc:SAML:1.0:am:password";
pub const AUTHMETHOD_KERBEROS: &str = "urn:ietf:rfc:1510";
pub const AUTHMETHOD_X509: &str = "urn:oasis:names:tc:SAML:1.0:am:X509-PKI";

#[derive(Debug, Clone)]
pub struct Saml11Assertion {
    pub assertion_id: String,
    pub issuer: String,
    pub issue_instant: DateTime<Utc>,
    pub name_identifier: String,
    pub name_identifier_format: String,
    pub not_before: DateTime<Utc>,
    pub not_on_or_after: DateTime<Utc>,
    pub audience: Option<String>,
    pub auth_method: String,
    pub auth_instant: DateTime<Utc>,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub attribute_namespace: String,
}

impl Saml11Assertion {
    pub fn new(_issuer: impl Into<String>, _name_id: impl Into<String>) -> Self {
        let now = Utc::now();
        // RED-phase: assertion_id is empty so the starts_with('_') test fails.
        Self {
            assertion_id: String::new(),
            issuer: String::new(),
            issue_instant: now,
            name_identifier: String::new(),
            name_identifier_format: String::new(),
            not_before: now,
            not_on_or_after: now,
            audience: None,
            auth_method: String::new(),
            auth_instant: now,
            attributes: BTreeMap::new(),
            attribute_namespace: String::new(),
        }
    }
    pub fn add_attribute(&mut self, _n: impl Into<String>, _v: Vec<String>) {}
    pub fn to_xml(&self) -> Result<String, WsFedError> {
        Err(WsFedError::Parse("RED-phase stub".into()))
    }
}

pub fn parse_minimal(_xml: &str) -> Result<Saml11Assertion, WsFedError> {
    Err(WsFedError::Parse("RED-phase stub".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Saml11Assertion {
        let mut a = Saml11Assertion::new("https://idp.example/wsfed", "alice@example.com");
        a.audience = Some("urn:example:relying-party".into());
        a.add_attribute("name", vec!["Alice Cooper".into()]);
        a.add_attribute("group", vec!["admins".into(), "engineers".into()]);
        a
    }

    #[test]
    fn assertion_id_starts_with_underscore() {
        let a = Saml11Assertion::new("iss", "sub");
        assert!(a.assertion_id.starts_with('_'));
        assert!(a.assertion_id.len() > 10);
    }

    #[test]
    fn xml_carries_saml_1_1_namespace() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("urn:oasis:names:tc:SAML:1.0:assertion"));
        assert!(!xml.contains("urn:oasis:names:tc:SAML:2.0:assertion"));
    }

    #[test]
    fn xml_carries_majorversion_1_minorversion_1() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("MajorVersion=\"1\""));
        assert!(xml.contains("MinorVersion=\"1\""));
    }

    #[test]
    fn xml_uses_nameidentifier_not_nameid() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("<saml:NameIdentifier"));
        assert!(!xml.contains("<saml:NameID"));
    }

    #[test]
    fn xml_uses_authenticationstatement_not_authnstatement() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("<saml:AuthenticationStatement"));
        assert!(!xml.contains("<saml:AuthnStatement"));
    }

    #[test]
    fn xml_attributes_use_attributename_and_attributenamespace() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("AttributeName=\"name\""));
        assert!(xml.contains("AttributeNamespace=\"http://schemas.xmlsoap.org/claims\""));
    }

    #[test]
    fn xml_includes_audience_when_set() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("<saml:Audience>urn:example:relying-party</saml:Audience>"));
    }

    #[test]
    fn xml_omits_audience_when_unset() {
        let mut a = fixture();
        a.audience = None;
        let xml = a.to_xml().unwrap();
        assert!(!xml.contains("<saml:Audience>"));
        assert!(!xml.contains("AudienceRestrictionCondition"));
    }

    #[test]
    fn xml_includes_multivalued_attribute() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("<saml:AttributeValue>admins</saml:AttributeValue>"));
        assert!(xml.contains("<saml:AttributeValue>engineers</saml:AttributeValue>"));
    }

    #[test]
    fn xml_includes_bearer_confirmation() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("urn:oasis:names:tc:SAML:1.0:cm:bearer"));
    }

    #[test]
    fn xml_escapes_special_chars() {
        let mut a = Saml11Assertion::new("https://i.example/<wsfed>", "alice&bob");
        a.add_attribute("note", vec!["a < b".into()]);
        let xml = a.to_xml().unwrap();
        assert!(xml.contains("alice&amp;bob"));
        assert!(xml.contains("a &lt; b"));
        assert!(xml.contains("&lt;wsfed&gt;"));
    }

    #[test]
    fn parse_minimal_roundtrips_assertion_id_and_issuer() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        let parsed = parse_minimal(&xml).unwrap();
        assert_eq!(parsed.assertion_id, a.assertion_id);
        assert_eq!(parsed.issuer, a.issuer);
        assert_eq!(parsed.name_identifier, a.name_identifier);
    }

    #[test]
    fn parse_minimal_roundtrips_attributes() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        let parsed = parse_minimal(&xml).unwrap();
        assert_eq!(
            parsed.attributes.get("group").map(|v| v.as_slice()),
            Some(&["admins".to_string(), "engineers".to_string()][..])
        );
        assert_eq!(parsed.attributes.get("name").map(|v| v.as_slice()), Some(&["Alice Cooper".to_string()][..]));
    }

    #[test]
    fn parse_minimal_roundtrips_audience() {
        let a = fixture();
        let xml = a.to_xml().unwrap();
        let parsed = parse_minimal(&xml).unwrap();
        assert_eq!(parsed.audience.as_deref(), Some("urn:example:relying-party"));
    }

    #[test]
    fn parse_minimal_rejects_missing_assertion_id() {
        let bad = "<saml:Assertion xmlns:saml=\"urn:oasis:names:tc:SAML:1.0:assertion\" \
                   MajorVersion=\"1\" MinorVersion=\"1\" Issuer=\"x\" IssueInstant=\"2024-01-01T00:00:00Z\">\
                   </saml:Assertion>";
        let err = parse_minimal(bad).unwrap_err();
        assert!(matches!(err, WsFedError::MissingField(_)));
    }

    #[test]
    fn parse_minimal_rejects_malformed_xml() {
        let bad = "<saml:Assertion <broken";
        assert!(parse_minimal(bad).is_err());
    }
}
