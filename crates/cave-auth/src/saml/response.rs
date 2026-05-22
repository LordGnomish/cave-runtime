// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `<samlp:Response>` + `<saml:Assertion>` — the IdP → SP message
//! that carries the authenticated subject and any attribute
//! statements.
//!
//! Mirrors `org.keycloak.saml.processing.core.parsers.saml.SAMLParser`
//! + `org.keycloak.saml.processing.core.saml.v2.writers.SAMLResponseWriter`
//! from upstream `saml-core`.

use std::collections::BTreeMap;
use std::io::Cursor;

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use uuid::Uuid;

use super::{NameIdFormat, SamlError, SamlSubject, ns};

/// The two `<samlp:StatusCode>` values cave-auth ever emits. Real
/// SAML has more, but Keycloak in practice only differentiates
/// `Success` and "anything else".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Success,
    Responder,
}

impl StatusCode {
    pub fn as_urn(self) -> &'static str {
        match self {
            StatusCode::Success => "urn:oasis:names:tc:SAML:2.0:status:Success",
            StatusCode::Responder => "urn:oasis:names:tc:SAML:2.0:status:Responder",
        }
    }

    pub fn from_urn(s: &str) -> Option<Self> {
        match s {
            "urn:oasis:names:tc:SAML:2.0:status:Success" => Some(StatusCode::Success),
            "urn:oasis:names:tc:SAML:2.0:status:Responder" => Some(StatusCode::Responder),
            _ => None,
        }
    }
}

/// A SAML 2.0 Assertion. One Response wraps exactly one Assertion;
/// cave-auth doesn't emit (or accept) multi-Assertion responses
/// because no production IdP does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub issuer: String,
    pub subject_name_id: String,
    pub subject_name_id_format: NameIdFormat,
    /// `NotBefore` — earliest moment the Assertion may be used.
    pub not_before: DateTime<Utc>,
    /// `NotOnOrAfter` — first moment the Assertion is stale.
    pub not_on_or_after: DateTime<Utc>,
    /// `Audience` URIs that may consume this Assertion (the SP
    /// entity ID, usually). Multi-value to match the spec but
    /// almost always single.
    pub audiences: Vec<String>,
    /// AttributeStatement → flattened multi-value map.
    pub attributes: BTreeMap<String, Vec<String>>,
    /// AuthnStatement `SessionIndex` — opaque IdP session ref.
    pub session_index: Option<String>,
}

/// A SAML 2.0 Response wrapping an Assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub destination: String,
    pub in_response_to: Option<String>,
    pub issuer: String,
    pub status: StatusCode,
    pub assertion: Option<Assertion>,
}

impl Assertion {
    /// Builder helper — fresh ID, `IssueInstant = now`, validity
    /// window of `[now - 30s, now + 5min]` (matches Keycloak's
    /// default clock-skew tolerance).
    pub fn new(issuer: impl Into<String>, subject: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: format!("_{}", Uuid::new_v4().simple()),
            issue_instant: now,
            issuer: issuer.into(),
            subject_name_id: subject.into(),
            subject_name_id_format: NameIdFormat::EmailAddress,
            not_before: now - chrono::Duration::seconds(30),
            not_on_or_after: now + chrono::Duration::minutes(5),
            audiences: Vec::new(),
            attributes: BTreeMap::new(),
            session_index: None,
        }
    }

    pub fn with_audience(mut self, aud: impl Into<String>) -> Self {
        self.audiences.push(aud.into());
        self
    }

    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes
            .entry(name.into())
            .or_default()
            .push(value.into());
        self
    }

    /// Validity-window check at instant `now`. Includes a 30s
    /// clock-skew tolerance that matches Keycloak's
    /// `ASSERTION_SKEW_TIME_SEC = 30` default.
    pub fn is_time_valid(&self, now: DateTime<Utc>) -> bool {
        let skew = chrono::Duration::seconds(30);
        now + skew >= self.not_before && now < self.not_on_or_after + skew
    }
}

impl Response {
    /// New success Response wrapping the given Assertion.
    pub fn success(
        issuer: impl Into<String>,
        destination: impl Into<String>,
        in_response_to: Option<String>,
        assertion: Assertion,
    ) -> Self {
        Self {
            id: format!("_{}", Uuid::new_v4().simple()),
            issue_instant: Utc::now(),
            destination: destination.into(),
            in_response_to,
            issuer: issuer.into(),
            status: StatusCode::Success,
            assertion: Some(assertion),
        }
    }

    pub fn to_xml(&self) -> Result<Vec<u8>, SamlError> {
        let mut buf = Cursor::new(Vec::new());
        let mut w = Writer::new(&mut buf);
        write_response(&mut w, self)?;
        Ok(buf.into_inner())
    }

    pub fn from_xml(bytes: &[u8]) -> Result<Self, SamlError> {
        parse_response(bytes)
    }

    /// Promote a verified Response to a [`SamlSubject`] —
    /// the data the auth_middleware layer actually consumes.
    /// Returns `Err` if status isn't `Success` or no Assertion.
    pub fn into_subject(self) -> Result<SamlSubject, SamlError> {
        if self.status != StatusCode::Success {
            return Err(SamlError::Other(format!(
                "Response status is not Success: {}",
                self.status.as_urn()
            )));
        }
        let a = self
            .assertion
            .ok_or_else(|| SamlError::MissingField("Assertion".into()))?;
        Ok(SamlSubject {
            name_id: a.subject_name_id,
            name_id_format: a.subject_name_id_format,
            issuer: a.issuer,
            attributes: a.attributes,
            session_index: a.session_index,
        })
    }
}

// ── Writer ───────────────────────────────────────────────────────────────────

fn write_response<W: std::io::Write>(w: &mut Writer<W>, r: &Response) -> Result<(), SamlError> {
    let issue_instant = r
        .issue_instant
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut root = BytesStart::new("samlp:Response");
    root.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
    root.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
    root.push_attribute(("ID", r.id.as_str()));
    root.push_attribute(("Version", "2.0"));
    root.push_attribute(("IssueInstant", issue_instant.as_str()));
    root.push_attribute(("Destination", r.destination.as_str()));
    if let Some(irt) = &r.in_response_to {
        root.push_attribute(("InResponseTo", irt.as_str()));
    }
    w.write_event(Event::Start(root)).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Issuer")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.issuer)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Issuer")))
        .map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("samlp:Status")))
        .map_err(io_err)?;
    let mut code = BytesStart::new("samlp:StatusCode");
    code.push_attribute(("Value", r.status.as_urn()));
    w.write_event(Event::Empty(code)).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("samlp:Status")))
        .map_err(io_err)?;

    if let Some(a) = &r.assertion {
        write_assertion(w, a)?;
    }

    w.write_event(Event::End(BytesEnd::new("samlp:Response")))
        .map_err(io_err)?;
    Ok(())
}

fn write_assertion<W: std::io::Write>(w: &mut Writer<W>, a: &Assertion) -> Result<(), SamlError> {
    let issue_instant = a
        .issue_instant
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let not_before = a
        .not_before
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let not_after = a
        .not_on_or_after
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut root = BytesStart::new("saml:Assertion");
    root.push_attribute(("ID", a.id.as_str()));
    root.push_attribute(("Version", "2.0"));
    root.push_attribute(("IssueInstant", issue_instant.as_str()));
    w.write_event(Event::Start(root)).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Issuer")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&a.issuer)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Issuer")))
        .map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Subject")))
        .map_err(io_err)?;
    let mut nid = BytesStart::new("saml:NameID");
    nid.push_attribute(("Format", a.subject_name_id_format.as_urn()));
    w.write_event(Event::Start(nid)).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&a.subject_name_id)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:NameID")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Subject")))
        .map_err(io_err)?;

    let mut cond = BytesStart::new("saml:Conditions");
    cond.push_attribute(("NotBefore", not_before.as_str()));
    cond.push_attribute(("NotOnOrAfter", not_after.as_str()));
    if a.audiences.is_empty() {
        w.write_event(Event::Empty(cond)).map_err(io_err)?;
    } else {
        w.write_event(Event::Start(cond)).map_err(io_err)?;
        w.write_event(Event::Start(BytesStart::new("saml:AudienceRestriction")))
            .map_err(io_err)?;
        for aud in &a.audiences {
            w.write_event(Event::Start(BytesStart::new("saml:Audience")))
                .map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(aud)))
                .map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("saml:Audience")))
                .map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new("saml:AudienceRestriction")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("saml:Conditions")))
            .map_err(io_err)?;
    }

    let mut authn = BytesStart::new("saml:AuthnStatement");
    authn.push_attribute(("AuthnInstant", issue_instant.as_str()));
    if let Some(idx) = &a.session_index {
        authn.push_attribute(("SessionIndex", idx.as_str()));
    }
    w.write_event(Event::Start(authn)).map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("saml:AuthnContext")))
        .map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("saml:AuthnContextClassRef")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(
        "urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport",
    )))
    .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:AuthnContextClassRef")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:AuthnContext")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:AuthnStatement")))
        .map_err(io_err)?;

    if !a.attributes.is_empty() {
        w.write_event(Event::Start(BytesStart::new("saml:AttributeStatement")))
            .map_err(io_err)?;
        for (name, values) in &a.attributes {
            let mut attr = BytesStart::new("saml:Attribute");
            attr.push_attribute(("Name", name.as_str()));
            w.write_event(Event::Start(attr)).map_err(io_err)?;
            for v in values {
                w.write_event(Event::Start(BytesStart::new("saml:AttributeValue")))
                    .map_err(io_err)?;
                w.write_event(Event::Text(BytesText::new(v)))
                    .map_err(io_err)?;
                w.write_event(Event::End(BytesEnd::new("saml:AttributeValue")))
                    .map_err(io_err)?;
            }
            w.write_event(Event::End(BytesEnd::new("saml:Attribute")))
                .map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new("saml:AttributeStatement")))
            .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("saml:Assertion")))
        .map_err(io_err)?;
    Ok(())
}

// ── Parser ───────────────────────────────────────────────────────────────────

fn parse_response(bytes: &[u8]) -> Result<Response, SamlError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut response_id = None;
    let mut response_issue = None;
    let mut response_dest = None;
    let mut response_in_response_to = None;
    let mut response_issuer = None;
    let mut status = StatusCode::Responder;

    let mut assertion_id = None;
    let mut assertion_issue = None;
    let mut assertion_issuer = None;
    let mut subject_name_id = None;
    let mut subject_format = NameIdFormat::Unspecified;
    let mut not_before = None;
    let mut not_on_or_after = None;
    let mut audiences: Vec<String> = Vec::new();
    let mut attributes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut session_index: Option<String> = None;

    let mut current_text_target: Option<&'static str> = None;
    let mut in_response_issuer = false;
    let mut in_assertion = false;
    let mut in_assertion_issuer = false;
    let mut current_attr: Option<String> = None;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "Response" => {
                        for a in e.attributes().flatten() {
                            let key = local_name(a.key.as_ref());
                            let val = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match key.as_str() {
                                "ID" => response_id = Some(val),
                                "IssueInstant" => response_issue = Some(val),
                                "Destination" => response_dest = Some(val),
                                "InResponseTo" => response_in_response_to = Some(val),
                                _ => {}
                            }
                        }
                    }
                    "Issuer" => {
                        if in_assertion {
                            in_assertion_issuer = true;
                        } else {
                            in_response_issuer = true;
                        }
                    }
                    "StatusCode" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "Value" {
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?;
                                status =
                                    StatusCode::from_urn(&val).unwrap_or(StatusCode::Responder);
                            }
                        }
                    }
                    "Assertion" => {
                        in_assertion = true;
                        for a in e.attributes().flatten() {
                            let key = local_name(a.key.as_ref());
                            let val = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match key.as_str() {
                                "ID" => assertion_id = Some(val),
                                "IssueInstant" => assertion_issue = Some(val),
                                _ => {}
                            }
                        }
                    }
                    "NameID" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "Format" {
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?;
                                subject_format = NameIdFormat::from_urn(&val)
                                    .unwrap_or(NameIdFormat::Unspecified);
                            }
                        }
                        current_text_target = Some("subject_name_id");
                    }
                    "Conditions" => {
                        for a in e.attributes().flatten() {
                            let key = local_name(a.key.as_ref());
                            let val = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match key.as_str() {
                                "NotBefore" => not_before = Some(val),
                                "NotOnOrAfter" => not_on_or_after = Some(val),
                                _ => {}
                            }
                        }
                    }
                    "Audience" => current_text_target = Some("audience"),
                    "AuthnStatement" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "SessionIndex" {
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?
                                    .into_owned();
                                session_index = Some(val);
                            }
                        }
                    }
                    "Attribute" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "Name" {
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?
                                    .into_owned();
                                current_attr = Some(val);
                            }
                        }
                    }
                    "AttributeValue" => current_text_target = Some("attribute_value"),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t
                    .unescape()
                    .map_err(|err| SamlError::Parse(err.to_string()))?
                    .into_owned();
                if in_response_issuer {
                    response_issuer = Some(txt);
                } else if in_assertion_issuer {
                    assertion_issuer = Some(txt);
                } else if let Some(target) = current_text_target {
                    match target {
                        "subject_name_id" => subject_name_id = Some(txt),
                        "audience" => audiences.push(txt),
                        "attribute_value" => {
                            if let Some(name) = &current_attr {
                                attributes.entry(name.clone()).or_default().push(txt);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "Issuer" => {
                        in_response_issuer = false;
                        in_assertion_issuer = false;
                    }
                    "Assertion" => in_assertion = false,
                    "Attribute" => current_attr = None,
                    "NameID" | "Audience" | "AttributeValue" => {
                        current_text_target = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }

    let response_id = response_id.ok_or_else(|| SamlError::MissingField("Response/ID".into()))?;
    let response_issue =
        response_issue.ok_or_else(|| SamlError::MissingField("Response/IssueInstant".into()))?;
    let response_dest =
        response_dest.ok_or_else(|| SamlError::MissingField("Response/Destination".into()))?;
    let response_issuer =
        response_issuer.ok_or_else(|| SamlError::MissingField("Response/Issuer".into()))?;

    let response_issue: DateTime<Utc> = DateTime::parse_from_rfc3339(&response_issue)
        .map_err(|e| SamlError::Parse(format!("Response/IssueInstant: {e}")))?
        .with_timezone(&Utc);

    let assertion = if let Some(aid) = assertion_id {
        let aissue = assertion_issue
            .ok_or_else(|| SamlError::MissingField("Assertion/IssueInstant".into()))?;
        let aissuer =
            assertion_issuer.ok_or_else(|| SamlError::MissingField("Assertion/Issuer".into()))?;
        let nid =
            subject_name_id.ok_or_else(|| SamlError::MissingField("Assertion/NameID".into()))?;
        let nb =
            not_before.ok_or_else(|| SamlError::MissingField("Conditions/NotBefore".into()))?;
        let noa = not_on_or_after
            .ok_or_else(|| SamlError::MissingField("Conditions/NotOnOrAfter".into()))?;
        Some(Assertion {
            id: aid,
            issue_instant: DateTime::parse_from_rfc3339(&aissue)
                .map_err(|e| SamlError::Parse(format!("Assertion/IssueInstant: {e}")))?
                .with_timezone(&Utc),
            issuer: aissuer,
            subject_name_id: nid,
            subject_name_id_format: subject_format,
            not_before: DateTime::parse_from_rfc3339(&nb)
                .map_err(|e| SamlError::Parse(format!("NotBefore: {e}")))?
                .with_timezone(&Utc),
            not_on_or_after: DateTime::parse_from_rfc3339(&noa)
                .map_err(|e| SamlError::Parse(format!("NotOnOrAfter: {e}")))?
                .with_timezone(&Utc),
            audiences,
            attributes,
            session_index,
        })
    } else {
        None
    };

    Ok(Response {
        id: response_id,
        issue_instant: response_issue,
        destination: response_dest,
        in_response_to: response_in_response_to,
        issuer: response_issuer,
        status,
        assertion,
    })
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

    fn sample_assertion() -> Assertion {
        Assertion::new("https://idp.example", "alice@example.com")
            .with_audience("https://sp.example")
            .with_attribute("email", "alice@example.com")
            .with_attribute("groups", "engineering")
            .with_attribute("groups", "platform")
    }

    #[test]
    fn assertion_new_sets_validity_window() {
        let a = Assertion::new("idp", "user");
        assert!(a.not_before < a.issue_instant);
        assert!(a.not_on_or_after > a.issue_instant);
    }

    #[test]
    fn time_valid_inside_window() {
        let a = sample_assertion();
        assert!(a.is_time_valid(Utc::now()));
    }

    #[test]
    fn time_invalid_before_not_before() {
        let mut a = sample_assertion();
        a.not_before = Utc::now() + chrono::Duration::hours(1);
        a.not_on_or_after = a.not_before + chrono::Duration::minutes(5);
        assert!(!a.is_time_valid(Utc::now()));
    }

    #[test]
    fn time_invalid_after_not_on_or_after() {
        let mut a = sample_assertion();
        a.not_before = Utc::now() - chrono::Duration::hours(2);
        a.not_on_or_after = Utc::now() - chrono::Duration::hours(1);
        assert!(!a.is_time_valid(Utc::now()));
    }

    #[test]
    fn response_xml_round_trips() {
        let a = sample_assertion();
        let r = Response::success(
            "https://idp.example",
            "https://sp.example/acs",
            Some("_req-1".to_string()),
            a,
        );
        let bytes = r.to_xml().unwrap();
        let parsed = Response::from_xml(&bytes).unwrap();
        assert_eq!(parsed.id, r.id);
        assert_eq!(parsed.destination, r.destination);
        assert_eq!(parsed.in_response_to.as_deref(), Some("_req-1"));
        assert_eq!(parsed.issuer, r.issuer);
        assert_eq!(parsed.status, StatusCode::Success);
        let pa = parsed.assertion.unwrap();
        assert_eq!(pa.subject_name_id, "alice@example.com");
        assert_eq!(pa.subject_name_id_format, NameIdFormat::EmailAddress);
        assert_eq!(pa.audiences, vec!["https://sp.example".to_string()]);
        assert_eq!(
            pa.attributes.get("groups").map(|v| v.as_slice()),
            Some(&["engineering".to_string(), "platform".to_string()][..])
        );
    }

    #[test]
    fn into_subject_extracts_payload() {
        let a = sample_assertion();
        let r = Response::success("idp", "dest", None, a);
        let s = r.into_subject().unwrap();
        assert_eq!(s.name_id, "alice@example.com");
        assert_eq!(s.issuer, "https://idp.example");
        assert_eq!(s.attributes.len(), 2);
    }

    #[test]
    fn into_subject_rejects_non_success() {
        let a = sample_assertion();
        let mut r = Response::success("idp", "dest", None, a);
        r.status = StatusCode::Responder;
        assert!(r.into_subject().is_err());
    }

    #[test]
    fn into_subject_rejects_missing_assertion() {
        let r = Response {
            id: "_r".into(),
            issue_instant: Utc::now(),
            destination: "d".into(),
            in_response_to: None,
            issuer: "i".into(),
            status: StatusCode::Success,
            assertion: None,
        };
        assert!(r.into_subject().is_err());
    }

    #[test]
    fn parser_rejects_malformed() {
        assert!(Response::from_xml(b"<not xml").is_err());
    }
}
