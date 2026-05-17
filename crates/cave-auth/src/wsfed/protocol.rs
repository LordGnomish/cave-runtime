// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/builders/

//! WS-Trust `RequestSecurityToken` (RST) and `RequestSecurityTokenResponse` (RSTR)
//! envelopes used by WS-Federation passive profile.
//!
//! The RSTR is what an IdP returns to a relying party after sign-in;
//! it carries the SAML 1.1 assertion. The RST is the request the RP
//! sends *to* the IdP — but in WS-Fed passive (browser-based) flows the
//! "RST" is actually expressed via query parameters (`wa`, `wtrealm`,
//! `wctx`, `wreply`), and the RSTR is the only XML body that flies
//! across the wire. We model both anyway so the active profile can
//! reuse the codec.

use std::fmt::Write as _;

use super::WsFedError;

pub const NS_WST: &str = "http://schemas.xmlsoap.org/ws/2005/02/trust";
pub const NS_WSP: &str = "http://schemas.xmlsoap.org/ws/2004/09/policy";
pub const NS_WSA: &str = "http://www.w3.org/2005/08/addressing";

/// SAML 1.1 token-type URI (the only one we issue).
pub const TOKEN_TYPE_SAML_1_1: &str = "urn:oasis:names:tc:SAML:1.0:assertion";

/// Browser-passive sign-in parameters (the "RST" the RP actually sends —
/// not an XML body but a query-string set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassiveRst {
    /// `wa` — sign-in / sign-out verb (always one of the [`super::WsAction`] strings).
    pub wa: String,
    /// `wtrealm` — relying-party realm URI. Required for sign-in.
    pub wtrealm: Option<String>,
    /// `wctx` — opaque per-request context the RP wants echoed back.
    pub wctx: Option<String>,
    /// `wreply` — reply URL to redirect the user to after sign-in.
    pub wreply: Option<String>,
}

impl PassiveRst {
    /// Build from a flat query-string map.
    pub fn from_query(q: &std::collections::BTreeMap<String, String>) -> Result<Self, WsFedError> {
        let wa = q.get("wa").cloned().ok_or_else(|| WsFedError::MissingField("wa".into()))?;
        Ok(Self {
            wa,
            wtrealm: q.get("wtrealm").cloned(),
            wctx: q.get("wctx").cloned(),
            wreply: q.get("wreply").cloned(),
        })
    }
}

/// XML body of an RSTR.
#[derive(Debug, Clone)]
pub struct Rstr {
    /// SAML 1.1 assertion XML — already serialised by
    /// [`super::saml11_assertion::Saml11Assertion::to_xml`], with the
    /// `<ds:Signature>` already inserted if signing is enabled.
    pub assertion_xml: String,
    /// `Lifetime/Created` instant.
    pub created: chrono::DateTime<chrono::Utc>,
    /// `Lifetime/Expires` instant.
    pub expires: chrono::DateTime<chrono::Utc>,
    /// `AppliesTo` endpoint — typically equal to `wtrealm` from the RST.
    pub applies_to: String,
}

impl Rstr {
    /// Serialise to the wire XML AD FS expects.
    pub fn to_xml(&self) -> Result<String, WsFedError> {
        let mut out = String::with_capacity(self.assertion_xml.len() + 512);
        write!(
            out,
            "<wst:RequestSecurityTokenResponse xmlns:wst=\"{ns_wst}\" \
             xmlns:wsp=\"{ns_wsp}\" xmlns:wsa=\"{ns_wsa}\">\
             <wst:Lifetime>\
             <wsu:Created xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">{created}</wsu:Created>\
             <wsu:Expires xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">{expires}</wsu:Expires>\
             </wst:Lifetime>\
             <wsp:AppliesTo><wsa:EndpointReference><wsa:Address>{applies_to}</wsa:Address></wsa:EndpointReference></wsp:AppliesTo>\
             <wst:RequestedSecurityToken>{assertion}</wst:RequestedSecurityToken>\
             <wst:TokenType>{tt}</wst:TokenType>\
             <wst:RequestType>http://schemas.xmlsoap.org/ws/2005/02/trust/Issue</wst:RequestType>\
             <wst:KeyType>http://schemas.xmlsoap.org/ws/2005/05/identity/NoProofKey</wst:KeyType>\
             </wst:RequestSecurityTokenResponse>",
            ns_wst = NS_WST,
            ns_wsp = NS_WSP,
            ns_wsa = NS_WSA,
            created = self.created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            expires = self.expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            applies_to = xml_escape(&self.applies_to),
            assertion = self.assertion_xml,
            tt = TOKEN_TYPE_SAML_1_1,
        )
        .map_err(|e| WsFedError::Parse(format!("fmt: {e}")))?;
        Ok(out)
    }

    /// Parse the subset of RSTR we emit.
    pub fn parse(xml: &str) -> Result<Self, WsFedError> {
        // Pull out the assertion fragment.
        let start_tag = "<wst:RequestedSecurityToken>";
        let end_tag = "</wst:RequestedSecurityToken>";
        let start = xml.find(start_tag).ok_or_else(|| WsFedError::MissingField("RequestedSecurityToken".into()))?;
        let end = xml.find(end_tag).ok_or_else(|| WsFedError::MissingField("RequestedSecurityToken end".into()))?;
        let assertion_xml = xml[start + start_tag.len()..end].to_string();
        let applies_to = extract_text_between(xml, "<wsa:Address>", "</wsa:Address>")
            .unwrap_or_default();
        let created = extract_text_between(xml, "<wsu:Created", "</wsu:Created>")
            .and_then(|s| s.split('>').nth(1).map(str::to_string))
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let expires = extract_text_between(xml, "<wsu:Expires", "</wsu:Expires>")
            .and_then(|s| s.split('>').nth(1).map(str::to_string))
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        Ok(Self { assertion_xml, created, expires, applies_to })
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn extract_text_between(haystack: &str, open: &str, close: &str) -> Option<String> {
    let start = haystack.find(open)?;
    let after = &haystack[start + open.len()..];
    let end = after.find(close)?;
    Some(after[..end].to_string())
}

/// Encode the RSTR for POST submission to the relying party.
///
/// AD FS expects `wresult` to be the raw XML (URL-encoded inside the
/// form), not base64. We return the XML — the HTTP layer URL-encodes it.
pub fn encode_wresult(rstr_xml: &str) -> String {
    rstr_xml.to_string()
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
