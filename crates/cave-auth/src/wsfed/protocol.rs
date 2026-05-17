// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/builders/

//! WS-Trust RST/RSTR — RED phase: tests authored, implementation lands in GREEN.

use super::WsFedError;

pub const NS_WST: &str = "";
pub const NS_WSP: &str = "";
pub const NS_WSA: &str = "";
pub const TOKEN_TYPE_SAML_1_1: &str = "";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassiveRst {
    pub wa: String,
    pub wtrealm: Option<String>,
    pub wctx: Option<String>,
    pub wreply: Option<String>,
}

impl PassiveRst {
    pub fn from_query(_q: &std::collections::BTreeMap<String, String>) -> Result<Self, WsFedError> {
        Err(WsFedError::Parse("RED-phase stub".into()))
    }
}

#[derive(Debug, Clone)]
pub struct Rstr {
    pub assertion_xml: String,
    pub created: chrono::DateTime<chrono::Utc>,
    pub expires: chrono::DateTime<chrono::Utc>,
    pub applies_to: String,
}

impl Rstr {
    pub fn to_xml(&self) -> Result<String, WsFedError> {
        Err(WsFedError::Parse("RED-phase stub".into()))
    }
    pub fn parse(_xml: &str) -> Result<Self, WsFedError> {
        Err(WsFedError::Parse("RED-phase stub".into()))
    }
}

pub fn encode_wresult(_xml: &str) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn passive_rst_extracts_all_fields() {
        let mut q = BTreeMap::new();
        q.insert("wa".into(), "wsignin1.0".into());
        q.insert("wtrealm".into(), "urn:rp".into());
        q.insert("wctx".into(), "rm=0&id=passive".into());
        q.insert("wreply".into(), "https://rp.example/wsfed".into());
        let r = PassiveRst::from_query(&q).unwrap();
        assert_eq!(r.wa, "wsignin1.0");
        assert_eq!(r.wtrealm.as_deref(), Some("urn:rp"));
        assert_eq!(r.wctx.as_deref(), Some("rm=0&id=passive"));
        assert_eq!(r.wreply.as_deref(), Some("https://rp.example/wsfed"));
    }

    #[test]
    fn passive_rst_requires_wa() {
        let q = BTreeMap::new();
        let err = PassiveRst::from_query(&q).unwrap_err();
        assert!(matches!(err, WsFedError::MissingField(s) if s == "wa"));
    }

    #[test]
    fn rstr_round_trips_through_xml() {
        let r = Rstr {
            assertion_xml: "<saml:Assertion xmlns:saml=\"urn:oasis:names:tc:SAML:1.0:assertion\" \
                            AssertionID=\"_abc\" Issuer=\"https://i\" \
                            IssueInstant=\"2024-01-01T00:00:00Z\" MajorVersion=\"1\" MinorVersion=\"1\">\
                            </saml:Assertion>".into(),
            created: chrono::Utc::now(),
            expires: chrono::Utc::now() + chrono::Duration::minutes(5),
            applies_to: "urn:rp:realm".into(),
        };
        let xml = r.to_xml().unwrap();
        let back = Rstr::parse(&xml).unwrap();
        assert!(back.assertion_xml.contains("AssertionID=\"_abc\""));
        assert_eq!(back.applies_to, "urn:rp:realm");
    }

    #[test]
    fn rstr_xml_uses_wst_namespace() {
        let r = Rstr {
            assertion_xml: "<x/>".into(),
            created: chrono::Utc::now(),
            expires: chrono::Utc::now(),
            applies_to: "urn:x".into(),
        };
        let xml = r.to_xml().unwrap();
        assert!(xml.contains("xmlns:wst=\"http://schemas.xmlsoap.org/ws/2005/02/trust\""));
        assert!(xml.contains("<wst:RequestSecurityTokenResponse"));
        assert!(xml.contains("<wst:RequestedSecurityToken>"));
        assert!(xml.contains("<wst:TokenType>urn:oasis:names:tc:SAML:1.0:assertion</wst:TokenType>"));
    }

    #[test]
    fn rstr_xml_includes_applies_to() {
        let r = Rstr {
            assertion_xml: "<x/>".into(),
            created: chrono::Utc::now(),
            expires: chrono::Utc::now(),
            applies_to: "urn:rp:special".into(),
        };
        let xml = r.to_xml().unwrap();
        assert!(xml.contains("<wsa:Address>urn:rp:special</wsa:Address>"));
    }

    #[test]
    fn rstr_parse_rejects_missing_token() {
        let bad = "<wst:RequestSecurityTokenResponse></wst:RequestSecurityTokenResponse>";
        assert!(Rstr::parse(bad).is_err());
    }

    #[test]
    fn encode_wresult_is_identity() {
        let xml = "<wst:RequestSecurityTokenResponse/>";
        assert_eq!(encode_wresult(xml), xml);
    }

    #[test]
    fn rstr_carries_lifetime() {
        let created = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let expires = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:05:00Z").unwrap().with_timezone(&chrono::Utc);
        let r = Rstr {
            assertion_xml: "<x/>".into(),
            created, expires,
            applies_to: "urn:rp".into(),
        };
        let xml = r.to_xml().unwrap();
        assert!(xml.contains("2024-01-01T00:00:00Z"));
        assert!(xml.contains("2024-01-01T00:05:00Z"));
    }
}
