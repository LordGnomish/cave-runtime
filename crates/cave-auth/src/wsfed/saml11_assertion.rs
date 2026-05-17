// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/dom/saml/v1/assertion/

//! SAML **1.1** Assertion — the payload WS-Fed wraps in an RSTR.
//!
//! Notable wire-format differences from SAML 2.0:
//!
//! * Namespace is `urn:oasis:names:tc:SAML:1.0:assertion` (yes, 1.0).
//!   SAML 1.1 reuses 1.0 namespace URIs.
//! * Subject identifier is `<saml:NameIdentifier>` (one word, not
//!   `<saml:NameID>`).
//! * `<saml:AuthenticationStatement>` (not `AuthnStatement`).
//! * `<saml:AttributeStatement>` has `AttributeName` and
//!   `AttributeNamespace` (not `Name`).
//! * `<saml:Conditions>` has `NotBefore`/`NotOnOrAfter` directly (no
//!   `<saml:AudienceRestrictionCondition>`-shape rename).
//!
//! These deltas matter when an AD FS RP expects byte-exact wire format.

use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::WsFedError;

/// SAML 1.1 assertion namespace — confusingly named "1.0" by spec.
pub const NS_SAML_1_1: &str = "urn:oasis:names:tc:SAML:1.0:assertion";

/// `<saml:NameIdentifier Format="…">` formats SAML 1.1 ships with.
pub const NAMEID_EMAIL: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";
pub const NAMEID_UNSPECIFIED: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified";

/// `<saml:AuthenticationMethod>` URIs SAML 1.1 RPs accept.
pub const AUTHMETHOD_PASSWORD: &str = "urn:oasis:names:tc:SAML:1.0:am:password";
pub const AUTHMETHOD_KERBEROS: &str = "urn:ietf:rfc:1510";
pub const AUTHMETHOD_X509: &str = "urn:oasis:names:tc:SAML:1.0:am:X509-PKI";

/// SAML 1.1 Assertion shaped to fit WS-Fed RSTR payload.
#[derive(Debug, Clone)]
pub struct Saml11Assertion {
    /// `AssertionID` attribute (required, must start with `_`).
    pub assertion_id: String,
    /// `Issuer` attribute — the IdP entity ID (a URI in 1.1 too).
    pub issuer: String,
    /// `IssueInstant`.
    pub issue_instant: DateTime<Utc>,
    /// Subject `NameIdentifier`.
    pub name_identifier: String,
    /// `Format` attribute on `NameIdentifier`.
    pub name_identifier_format: String,
    /// Conditions window `NotBefore`.
    pub not_before: DateTime<Utc>,
    /// Conditions window `NotOnOrAfter`.
    pub not_on_or_after: DateTime<Utc>,
    /// Optional `Audience` on `AudienceRestrictionCondition`.
    pub audience: Option<String>,
    /// Authentication method URI.
    pub auth_method: String,
    /// `AuthenticationInstant`.
    pub auth_instant: DateTime<Utc>,
    /// Attribute statement entries — name → value(s).
    pub attributes: BTreeMap<String, Vec<String>>,
    /// `AttributeNamespace` shared across the AttributeStatement —
    /// most AD FS deployments use this single URI.
    pub attribute_namespace: String,
}

impl Saml11Assertion {
    /// Build a new assertion with sensible defaults.
    pub fn new(issuer: impl Into<String>, name_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            assertion_id: format!("_{}", uuid::Uuid::new_v4().simple()),
            issuer: issuer.into(),
            issue_instant: now,
            name_identifier: name_id.into(),
            name_identifier_format: NAMEID_EMAIL.to_string(),
            not_before: now,
            not_on_or_after: now + chrono::Duration::minutes(5),
            audience: None,
            auth_method: AUTHMETHOD_PASSWORD.to_string(),
            auth_instant: now,
            attributes: BTreeMap::new(),
            attribute_namespace: "http://schemas.xmlsoap.org/claims".to_string(),
        }
    }

    /// Add or overwrite an attribute.
    pub fn add_attribute(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.attributes.insert(name.into(), values);
    }

    /// Render this assertion to canonical-ish SAML 1.1 XML.
    ///
    /// We do *not* run exc-c14n; the caller signs the bytes we produce. As
    /// long as both sides treat these bytes as authoritative the signature
    /// round-trips. AD FS specifically tolerates this — it canonicalises
    /// the assertion fragment it receives.
    pub fn to_xml(&self) -> Result<String, WsFedError> {
        let mut out = String::with_capacity(1024);
        write!(
            out,
            "<saml:Assertion xmlns:saml=\"{ns}\" MajorVersion=\"1\" MinorVersion=\"1\" \
             AssertionID=\"{aid}\" Issuer=\"{iss}\" IssueInstant=\"{iat}\">",
            ns = NS_SAML_1_1,
            aid = xml_escape_attr(&self.assertion_id),
            iss = xml_escape_attr(&self.issuer),
            iat = self.issue_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
        .map_err(|e| WsFedError::Parse(format!("fmt: {e}")))?;

        // Conditions
        write!(
            out,
            "<saml:Conditions NotBefore=\"{nb}\" NotOnOrAfter=\"{nooa}\">",
            nb = self.not_before.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            nooa = self.not_on_or_after.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
        .unwrap();
        if let Some(aud) = &self.audience {
            write!(
                out,
                "<saml:AudienceRestrictionCondition><saml:Audience>{a}</saml:Audience>\
                 </saml:AudienceRestrictionCondition>",
                a = xml_escape(aud)
            )
            .unwrap();
        }
        out.push_str("</saml:Conditions>");

        // AuthenticationStatement
        write!(
            out,
            "<saml:AuthenticationStatement AuthenticationMethod=\"{m}\" AuthenticationInstant=\"{i}\">\
             <saml:Subject><saml:NameIdentifier Format=\"{nf}\">{n}</saml:NameIdentifier>\
             <saml:SubjectConfirmation><saml:ConfirmationMethod>urn:oasis:names:tc:SAML:1.0:cm:bearer\
             </saml:ConfirmationMethod></saml:SubjectConfirmation></saml:Subject>\
             </saml:AuthenticationStatement>",
            m = xml_escape_attr(&self.auth_method),
            i = self.auth_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            nf = xml_escape_attr(&self.name_identifier_format),
            n = xml_escape(&self.name_identifier),
        )
        .unwrap();

        // AttributeStatement
        if !self.attributes.is_empty() {
            write!(
                out,
                "<saml:AttributeStatement><saml:Subject><saml:NameIdentifier Format=\"{nf}\">{n}\
                 </saml:NameIdentifier></saml:Subject>",
                nf = xml_escape_attr(&self.name_identifier_format),
                n = xml_escape(&self.name_identifier),
            )
            .unwrap();
            for (name, values) in &self.attributes {
                write!(
                    out,
                    "<saml:Attribute AttributeName=\"{n}\" AttributeNamespace=\"{ns}\">",
                    n = xml_escape_attr(name),
                    ns = xml_escape_attr(&self.attribute_namespace),
                )
                .unwrap();
                for v in values {
                    write!(out, "<saml:AttributeValue>{v}</saml:AttributeValue>", v = xml_escape(v))
                        .unwrap();
                }
                out.push_str("</saml:Attribute>");
            }
            out.push_str("</saml:AttributeStatement>");
        }

        out.push_str("</saml:Assertion>");
        Ok(out)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_escape_attr(s: &str) -> String {
    xml_escape(s).replace('"', "&quot;")
}

/// Parse the subset of SAML 1.1 we emit — enough for round-trip tests
/// and for the RSTR consumer to lift attributes back out.
pub fn parse_minimal(xml: &str) -> Result<Saml11Assertion, WsFedError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut a = Saml11Assertion {
        assertion_id: String::new(),
        issuer: String::new(),
        issue_instant: Utc::now(),
        name_identifier: String::new(),
        name_identifier_format: NAMEID_UNSPECIFIED.to_string(),
        not_before: Utc::now(),
        not_on_or_after: Utc::now(),
        audience: None,
        auth_method: AUTHMETHOD_PASSWORD.to_string(),
        auth_instant: Utc::now(),
        attributes: BTreeMap::new(),
        attribute_namespace: String::new(),
    };

    let mut current_attr_name: Option<String> = None;
    let mut current_attr_values: Vec<String> = Vec::new();
    let mut in_audience = false;
    let mut in_name_id = false;
    let mut in_attr_value = false;
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| WsFedError::Parse(format!("xml: {e}")))?
        {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                let local = std::str::from_utf8(name.as_ref())
                    .unwrap_or("")
                    .rsplit(':')
                    .next()
                    .unwrap_or("");
                match local {
                    "Assertion" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                "AssertionID" => a.assertion_id = val,
                                "Issuer" => a.issuer = val,
                                "IssueInstant" => {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(&val) {
                                        a.issue_instant = dt.with_timezone(&Utc);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "Conditions" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                "NotBefore" => {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(&val) {
                                        a.not_before = dt.with_timezone(&Utc);
                                    }
                                }
                                "NotOnOrAfter" => {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(&val) {
                                        a.not_on_or_after = dt.with_timezone(&Utc);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "Audience" => {
                        in_audience = true;
                    }
                    "AuthenticationStatement" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                "AuthenticationMethod" => a.auth_method = val,
                                "AuthenticationInstant" => {
                                    if let Ok(dt) = DateTime::parse_from_rfc3339(&val) {
                                        a.auth_instant = dt.with_timezone(&Utc);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "NameIdentifier" => {
                        in_name_id = true;
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            if key == "Format" {
                                a.name_identifier_format = val;
                            }
                        }
                    }
                    "Attribute" => {
                        let mut name = None;
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key {
                                "AttributeName" => name = Some(val),
                                "AttributeNamespace" => a.attribute_namespace = val,
                                _ => {}
                            }
                        }
                        current_attr_name = name;
                        current_attr_values.clear();
                    }
                    "AttributeValue" => {
                        in_attr_value = true;
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name();
                let local = std::str::from_utf8(name.as_ref())
                    .unwrap_or("")
                    .rsplit(':')
                    .next()
                    .unwrap_or("");
                match local {
                    "Audience" => in_audience = false,
                    "NameIdentifier" => in_name_id = false,
                    "AttributeValue" => in_attr_value = false,
                    "Attribute" => {
                        if let Some(n) = current_attr_name.take() {
                            a.attributes.insert(n, std::mem::take(&mut current_attr_values));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                let s = t.unescape().unwrap_or_default().to_string();
                if in_audience {
                    a.audience = Some(s);
                } else if in_name_id {
                    // Only capture the first NameIdentifier occurrence —
                    // the second occurrence inside AttributeStatement
                    // repeats the same subject and would double the value.
                    if a.name_identifier.is_empty() {
                        a.name_identifier.push_str(&s);
                    }
                } else if in_attr_value {
                    current_attr_values.push(s);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if a.assertion_id.is_empty() {
        return Err(WsFedError::MissingField("AssertionID".into()));
    }
    if a.issuer.is_empty() {
        return Err(WsFedError::MissingField("Issuer".into()));
    }
    if a.name_identifier.is_empty() {
        return Err(WsFedError::MissingField("NameIdentifier".into()));
    }
    Ok(a)
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
        // SAML 1.1 must NOT contain SAML 2.0 namespace anywhere.
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
