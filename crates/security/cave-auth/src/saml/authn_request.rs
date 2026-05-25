// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `<samlp:AuthnRequest>` — the SP → IdP message that initiates a
//! SAML login. cave-auth issues this when wearing the SP hat (eg.
//! federating *out* to a customer's IdP) and parses it when wearing
//! the IdP hat (eg. acting as the IdP for a downstream SP).
//!
//! Mirrors `org.keycloak.saml.SAMLAuthnRequestBuilder` /
//! `SAMLRequestParser` from the upstream `saml-core` module.

use std::io::Cursor;

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use uuid::Uuid;

use super::{NameIdFormat, SamlError, ns};

/// A SAML 2.0 `AuthnRequest`. Field naming follows the spec
/// (`Issuer`, `Destination`, `NameIDPolicy`...) rather than
/// snake_case, so the wire and the struct read identically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthnRequest {
    /// `ID` attribute — opaque, must be unique per request.
    /// The broker keys in-flight state on this value.
    pub id: String,
    /// `IssueInstant` — when the SP generated the request.
    pub issue_instant: DateTime<Utc>,
    /// `Destination` — the IdP SSO endpoint URL.
    pub destination: String,
    /// `<saml:Issuer>` — SP entity ID.
    pub issuer: String,
    /// `AssertionConsumerServiceURL` — where the IdP should
    /// POST the Response. Optional in the spec but required in
    /// practice by every real IdP.
    pub acs_url: Option<String>,
    /// `ProtocolBinding` — `urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST`
    /// for POST, `HTTP-Redirect` for Redirect. Optional.
    pub protocol_binding: Option<String>,
    /// `<samlp:NameIDPolicy Format=…>` — requested subject
    /// identifier shape.
    pub nameid_policy_format: Option<NameIdFormat>,
    /// `ForceAuthn="true"` — SP wants the IdP to re-prompt for
    /// credentials.
    pub force_authn: bool,
    /// `IsPassive="true"` — SP wants the IdP to NOT prompt.
    pub is_passive: bool,
}

impl AuthnRequest {
    /// Build a fresh `AuthnRequest` with a generated `ID` and an
    /// `IssueInstant` of `now`.
    pub fn new(issuer: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            id: format!("_{}", Uuid::new_v4().simple()),
            issue_instant: Utc::now(),
            destination: destination.into(),
            issuer: issuer.into(),
            acs_url: None,
            protocol_binding: None,
            nameid_policy_format: None,
            force_authn: false,
            is_passive: false,
        }
    }

    /// Builder: set the AssertionConsumerService URL.
    pub fn with_acs_url(mut self, url: impl Into<String>) -> Self {
        self.acs_url = Some(url.into());
        self
    }

    /// Builder: request a specific NameID format.
    pub fn with_nameid_format(mut self, fmt: NameIdFormat) -> Self {
        self.nameid_policy_format = Some(fmt);
        self
    }

    /// Builder: ask the IdP to re-prompt.
    pub fn force(mut self) -> Self {
        self.force_authn = true;
        self
    }

    /// Serialize to XML bytes. The shape matches Keycloak's
    /// `SAMLAuthnRequestBuilder.toDocument()` output — namespaces
    /// inline-declared on the root, attributes spec-ordered.
    pub fn to_xml(&self) -> Result<Vec<u8>, SamlError> {
        let mut buf = Cursor::new(Vec::new());
        let mut w = Writer::new(&mut buf);

        let issue_instant = self
            .issue_instant
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let mut req = BytesStart::new("samlp:AuthnRequest");
        req.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
        req.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
        req.push_attribute(("ID", self.id.as_str()));
        req.push_attribute(("Version", "2.0"));
        req.push_attribute(("IssueInstant", issue_instant.as_str()));
        req.push_attribute(("Destination", self.destination.as_str()));
        if let Some(url) = &self.acs_url {
            req.push_attribute(("AssertionConsumerServiceURL", url.as_str()));
        }
        if let Some(b) = &self.protocol_binding {
            req.push_attribute(("ProtocolBinding", b.as_str()));
        }
        if self.force_authn {
            req.push_attribute(("ForceAuthn", "true"));
        }
        if self.is_passive {
            req.push_attribute(("IsPassive", "true"));
        }
        w.write_event(Event::Start(req)).map_err(io_err)?;

        let issuer = BytesStart::new("saml:Issuer");
        w.write_event(Event::Start(issuer.clone()))
            .map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(&self.issuer)))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("saml:Issuer")))
            .map_err(io_err)?;

        if let Some(fmt) = self.nameid_policy_format {
            let mut nip = BytesStart::new("samlp:NameIDPolicy");
            nip.push_attribute(("Format", fmt.as_urn()));
            nip.push_attribute(("AllowCreate", "true"));
            w.write_event(Event::Empty(nip)).map_err(io_err)?;
        }

        w.write_event(Event::End(BytesEnd::new("samlp:AuthnRequest")))
            .map_err(io_err)?;

        Ok(buf.into_inner())
    }

    /// Parse XML bytes into an `AuthnRequest`. Tolerant of the
    /// minor namespace-prefix variations real-world IdPs emit
    /// (`saml2p:` vs `samlp:`).
    pub fn from_xml(bytes: &[u8]) -> Result<Self, SamlError> {
        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().trim_text(true);

        let mut id = None;
        let mut issue_instant = None;
        let mut destination = None;
        let mut acs_url = None;
        let mut protocol_binding = None;
        let mut force_authn = false;
        let mut is_passive = false;
        let mut issuer = None;
        let mut nameid_policy_format = None;

        // State: which child element are we currently inside?
        let mut in_issuer = false;

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let name = local_name(e.name().as_ref());
                    match name.as_str() {
                        "AuthnRequest" => {
                            for a in e.attributes().flatten() {
                                let key = local_name(a.key.as_ref());
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?
                                    .into_owned();
                                match key.as_str() {
                                    "ID" => id = Some(val),
                                    "IssueInstant" => issue_instant = Some(val),
                                    "Destination" => destination = Some(val),
                                    "AssertionConsumerServiceURL" => acs_url = Some(val),
                                    "ProtocolBinding" => protocol_binding = Some(val),
                                    "ForceAuthn" => force_authn = val == "true",
                                    "IsPassive" => is_passive = val == "true",
                                    _ => {}
                                }
                            }
                        }
                        "Issuer" => in_issuer = true,
                        "NameIDPolicy" => {
                            for a in e.attributes().flatten() {
                                if local_name(a.key.as_ref()) == "Format" {
                                    let val = a
                                        .unescape_value()
                                        .map_err(|err| SamlError::Parse(err.to_string()))?
                                        .into_owned();
                                    nameid_policy_format = NameIdFormat::from_urn(&val)
                                        .or(Some(NameIdFormat::Unspecified));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(e)) if in_issuer => {
                    let t = e
                        .unescape()
                        .map_err(|err| SamlError::Parse(err.to_string()))?
                        .into_owned();
                    issuer = Some(t);
                }
                Ok(Event::End(ref e)) => {
                    if local_name(e.name().as_ref()) == "Issuer" {
                        in_issuer = false;
                    }
                }
                _ => {}
            }
            buf.clear();
        }

        let id = id.ok_or_else(|| SamlError::MissingField("ID".into()))?;
        let issue_instant =
            issue_instant.ok_or_else(|| SamlError::MissingField("IssueInstant".into()))?;
        let destination =
            destination.ok_or_else(|| SamlError::MissingField("Destination".into()))?;
        let issuer = issuer.ok_or_else(|| SamlError::MissingField("Issuer".into()))?;
        let issue_instant: DateTime<Utc> = DateTime::parse_from_rfc3339(&issue_instant)
            .map_err(|e| SamlError::Parse(format!("IssueInstant: {e}")))?
            .with_timezone(&Utc);

        Ok(Self {
            id,
            issue_instant,
            destination,
            issuer,
            acs_url,
            protocol_binding,
            nameid_policy_format,
            force_authn,
            is_passive,
        })
    }
}

fn local_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

fn io_err(e: std::io::Error) -> SamlError {
    SamlError::Parse(format!("xml write: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_id_and_instant() {
        let r = AuthnRequest::new("sp", "https://idp.example/sso");
        assert!(r.id.starts_with('_'));
        assert!(!r.force_authn);
        assert!(!r.is_passive);
    }

    #[test]
    fn to_xml_contains_required_attributes() {
        let r = AuthnRequest::new("https://sp.example", "https://idp.example/sso")
            .with_acs_url("https://sp.example/acs")
            .with_nameid_format(NameIdFormat::EmailAddress)
            .force();
        let bytes = r.to_xml().unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("samlp:AuthnRequest"));
        assert!(s.contains(&r.id));
        assert!(s.contains("Destination=\"https://idp.example/sso\""));
        assert!(s.contains("AssertionConsumerServiceURL=\"https://sp.example/acs\""));
        assert!(s.contains("ForceAuthn=\"true\""));
        assert!(s.contains("saml:Issuer"));
        assert!(s.contains("https://sp.example"));
        assert!(s.contains(NameIdFormat::EmailAddress.as_urn()));
    }

    #[test]
    fn xml_round_trips_through_parser() {
        let orig = AuthnRequest::new("https://sp.example", "https://idp.example/sso")
            .with_acs_url("https://sp.example/acs")
            .with_nameid_format(NameIdFormat::Persistent);
        let bytes = orig.to_xml().unwrap();
        let parsed = AuthnRequest::from_xml(&bytes).unwrap();
        assert_eq!(parsed.id, orig.id);
        assert_eq!(parsed.destination, orig.destination);
        assert_eq!(parsed.issuer, orig.issuer);
        assert_eq!(parsed.acs_url, orig.acs_url);
        assert_eq!(parsed.nameid_policy_format, orig.nameid_policy_format);
    }

    #[test]
    fn parser_tolerates_alternate_namespace_prefix() {
        let xml = r#"<saml2p:AuthnRequest
            xmlns:saml2p="urn:oasis:names:tc:SAML:2.0:protocol"
            xmlns:saml2="urn:oasis:names:tc:SAML:2.0:assertion"
            ID="_abc" Version="2.0"
            IssueInstant="2026-05-13T10:00:00Z"
            Destination="https://idp/sso">
            <saml2:Issuer>https://sp.example</saml2:Issuer>
        </saml2p:AuthnRequest>"#;
        let r = AuthnRequest::from_xml(xml.as_bytes()).unwrap();
        assert_eq!(r.id, "_abc");
        assert_eq!(r.destination, "https://idp/sso");
        assert_eq!(r.issuer, "https://sp.example");
    }

    #[test]
    fn parser_rejects_missing_required() {
        let xml = r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
            Version="2.0" IssueInstant="2026-05-13T10:00:00Z"
            Destination="https://idp/sso"/>"#;
        assert!(matches!(
            AuthnRequest::from_xml(xml.as_bytes()).unwrap_err(),
            SamlError::MissingField(_)
        ));
    }

    #[test]
    fn parser_rejects_malformed_xml() {
        assert!(AuthnRequest::from_xml(b"<not xml").is_err());
    }
}
