// SPDX-License-Identifier: AGPL-3.0-or-later
//! SAML 2.0 Single-Logout (SLO): `<samlp:LogoutRequest>` /
//! `<samlp:LogoutResponse>` writers + parsers, plus a session-index
//! ledger the IdP role uses to terminate the SP-side session set when
//! a principal logs out.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/LogoutRequestType.java
//!         saml-core/src/main/java/org/keycloak/saml/SAML2LogoutResponseBuilder.java
//!         services/src/main/java/org/keycloak/protocol/saml/profile/util/LogoutProtocolUtil.java
//!         services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java::logoutRequest
//!
//! SLO is the inverse of an SP-initiated login: when the user signs
//! out at the IdP, the IdP iterates the live `SessionIndex` ledger
//! for that principal and POSTs (or back-channel SOAP-POSTs) a
//! `<samlp:LogoutRequest>` to every SP that holds a session for them.
//! Each SP replies with `<samlp:LogoutResponse>`. If any SP failed,
//! the IdP's terminal status to the user's UA becomes `PartialLogout`.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use super::ns;
use super::response::StatusCode;
use super::{NameIdFormat, SamlError};

/// `urn:oasis:names:tc:SAML:2.0:status:PartialLogout` — IdP couldn't
/// terminate every active SP session (one or more LogoutResponse came
/// back non-Success or timed out). Per SAML 2.0 Core §3.7.3.
pub const SLO_STATUS_PARTIAL_LOGOUT: &str =
    "urn:oasis:names:tc:SAML:2.0:status:PartialLogout";

/// A SAML 2.0 `<samlp:LogoutRequest>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoutRequest {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub destination: String,
    pub issuer: String,
    /// The subject being logged out. Carried as `<saml:NameID>`.
    pub name_id: String,
    pub name_id_format: NameIdFormat,
    /// `<samlp:SessionIndex>` values — usually one per request, but
    /// can be multi when a single principal had multiple parallel
    /// sessions at the same SP.
    pub session_indexes: Vec<String>,
    /// `Reason` attribute (URN) — `urn:…:logout:user` etc. Optional.
    pub reason: Option<String>,
}

/// A SAML 2.0 `<samlp:LogoutResponse>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoutResponse {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub destination: String,
    /// `ID` of the LogoutRequest this response answers.
    pub in_response_to: String,
    pub issuer: String,
    pub status: StatusCode,
}

// ─── Writers ─────────────────────────────────────────────────────────────────

pub fn write_logout_request(r: &LogoutRequest) -> Result<Vec<u8>, SamlError> {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new(&mut buf);
    let issue = r
        .issue_instant
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut root = BytesStart::new("samlp:LogoutRequest");
    root.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
    root.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
    root.push_attribute(("ID", r.id.as_str()));
    root.push_attribute(("Version", "2.0"));
    root.push_attribute(("IssueInstant", issue.as_str()));
    root.push_attribute(("Destination", r.destination.as_str()));
    if let Some(reason) = &r.reason {
        root.push_attribute(("Reason", reason.as_str()));
    }
    w.write_event(Event::Start(root)).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Issuer")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.issuer))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Issuer"))).map_err(io_err)?;

    let mut nid = BytesStart::new("saml:NameID");
    nid.push_attribute(("Format", r.name_id_format.as_urn()));
    w.write_event(Event::Start(nid)).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.name_id))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:NameID"))).map_err(io_err)?;

    for idx in &r.session_indexes {
        w.write_event(Event::Start(BytesStart::new("samlp:SessionIndex")))
            .map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(idx))).map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("samlp:SessionIndex"))).map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("samlp:LogoutRequest"))).map_err(io_err)?;
    Ok(buf.into_inner())
}

pub fn write_logout_response(r: &LogoutResponse) -> Result<Vec<u8>, SamlError> {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new(&mut buf);
    let issue = r
        .issue_instant
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut root = BytesStart::new("samlp:LogoutResponse");
    root.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
    root.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
    root.push_attribute(("ID", r.id.as_str()));
    root.push_attribute(("Version", "2.0"));
    root.push_attribute(("IssueInstant", issue.as_str()));
    root.push_attribute(("Destination", r.destination.as_str()));
    root.push_attribute(("InResponseTo", r.in_response_to.as_str()));
    w.write_event(Event::Start(root)).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Issuer")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.issuer))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Issuer"))).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("samlp:Status")))
        .map_err(io_err)?;
    let mut code = BytesStart::new("samlp:StatusCode");
    code.push_attribute(("Value", r.status.as_urn()));
    w.write_event(Event::Empty(code)).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("samlp:Status"))).map_err(io_err)?;

    w.write_event(Event::End(BytesEnd::new("samlp:LogoutResponse"))).map_err(io_err)?;
    Ok(buf.into_inner())
}

// ─── Parsers ─────────────────────────────────────────────────────────────────

pub fn parse_logout_request(bytes: &[u8]) -> Result<LogoutRequest, SamlError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut id = None;
    let mut issue_instant = None;
    let mut destination = None;
    let mut issuer = None;
    let mut name_id = None;
    let mut name_id_format = NameIdFormat::Unspecified;
    let mut session_indexes: Vec<String> = Vec::new();
    let mut reason = None;
    let mut current: Option<&'static str> = None;
    let mut buf = Vec::new();
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "LogoutRequest" => {
                        saw_root = true;
                        for a in e.attributes().flatten() {
                            let k = local_name(a.key.as_ref());
                            let v = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match k.as_str() {
                                "ID" => id = Some(v),
                                "IssueInstant" => issue_instant = Some(v),
                                "Destination" => destination = Some(v),
                                "Reason" => reason = Some(v),
                                _ => {}
                            }
                        }
                    }
                    "Issuer" => current = Some("issuer"),
                    "NameID" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "Format" {
                                let v = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?;
                                name_id_format = NameIdFormat::from_urn(&v)
                                    .unwrap_or(NameIdFormat::Unspecified);
                            }
                        }
                        current = Some("nameid");
                    }
                    "SessionIndex" => current = Some("sessionindex"),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t
                    .unescape()
                    .map_err(|err| SamlError::Parse(err.to_string()))?
                    .into_owned();
                match current {
                    Some("issuer") => issuer = Some(txt),
                    Some("nameid") => name_id = Some(txt),
                    Some("sessionindex") => session_indexes.push(txt),
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "Issuer" || name == "NameID" || name == "SessionIndex" {
                    current = None;
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if !saw_root {
        return Err(SamlError::MissingField("LogoutRequest element".into()));
    }
    let id = id.ok_or_else(|| SamlError::MissingField("LogoutRequest/ID".into()))?;
    let issue_instant = issue_instant
        .ok_or_else(|| SamlError::MissingField("LogoutRequest/IssueInstant".into()))?;
    let destination = destination
        .ok_or_else(|| SamlError::MissingField("LogoutRequest/Destination".into()))?;
    let issuer = issuer.ok_or_else(|| SamlError::MissingField("LogoutRequest/Issuer".into()))?;
    let name_id = name_id.ok_or_else(|| SamlError::MissingField("LogoutRequest/NameID".into()))?;

    Ok(LogoutRequest {
        id,
        issue_instant: DateTime::parse_from_rfc3339(&issue_instant)
            .map_err(|e| SamlError::Parse(format!("IssueInstant: {e}")))?
            .with_timezone(&Utc),
        destination,
        issuer,
        name_id,
        name_id_format,
        session_indexes,
        reason,
    })
}

pub fn parse_logout_response(bytes: &[u8]) -> Result<LogoutResponse, SamlError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut id = None;
    let mut issue_instant = None;
    let mut destination = None;
    let mut in_response_to = None;
    let mut issuer = None;
    let mut status = StatusCode::Responder;
    let mut current: Option<&'static str> = None;
    let mut buf = Vec::new();
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "LogoutResponse" => {
                        saw_root = true;
                        for a in e.attributes().flatten() {
                            let k = local_name(a.key.as_ref());
                            let v = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match k.as_str() {
                                "ID" => id = Some(v),
                                "IssueInstant" => issue_instant = Some(v),
                                "Destination" => destination = Some(v),
                                "InResponseTo" => in_response_to = Some(v),
                                _ => {}
                            }
                        }
                    }
                    "Issuer" => current = Some("issuer"),
                    "StatusCode" => {
                        for a in e.attributes().flatten() {
                            if local_name(a.key.as_ref()) == "Value" {
                                let v = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?;
                                status = StatusCode::from_urn(&v).unwrap_or(StatusCode::Responder);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t
                    .unescape()
                    .map_err(|err| SamlError::Parse(err.to_string()))?
                    .into_owned();
                if matches!(current, Some("issuer")) {
                    issuer = Some(txt);
                }
            }
            Ok(Event::End(ref e)) => {
                if local_name(e.name().as_ref()) == "Issuer" {
                    current = None;
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if !saw_root {
        return Err(SamlError::MissingField("LogoutResponse element".into()));
    }
    let id = id.ok_or_else(|| SamlError::MissingField("LogoutResponse/ID".into()))?;
    let issue_instant = issue_instant
        .ok_or_else(|| SamlError::MissingField("LogoutResponse/IssueInstant".into()))?;
    let destination = destination
        .ok_or_else(|| SamlError::MissingField("LogoutResponse/Destination".into()))?;
    let in_response_to = in_response_to
        .ok_or_else(|| SamlError::MissingField("LogoutResponse/InResponseTo".into()))?;
    let issuer = issuer.ok_or_else(|| SamlError::MissingField("LogoutResponse/Issuer".into()))?;

    Ok(LogoutResponse {
        id,
        issue_instant: DateTime::parse_from_rfc3339(&issue_instant)
            .map_err(|e| SamlError::Parse(format!("IssueInstant: {e}")))?
            .with_timezone(&Utc),
        destination,
        in_response_to,
        issuer,
        status,
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

// ─── Session-index ledger ───────────────────────────────────────────────────

/// In-memory mapping of principal → set of active `SessionIndex`
/// values. The IdP role uses this when handling an SP-initiated SLO
/// (drop one specific session) or when terminating all sessions for a
/// principal (drop_all).
#[derive(Clone, Default)]
pub struct SessionIndexLedger {
    inner: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl SessionIndexLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track(&self, principal: &str, session_index: &str) {
        let mut g = self.inner.write().expect("poisoned");
        let entry = g.entry(principal.to_string()).or_default();
        if !entry.iter().any(|s| s == session_index) {
            entry.push(session_index.to_string());
        }
    }

    pub fn indexes_for(&self, principal: &str) -> Vec<String> {
        self.inner
            .read()
            .expect("poisoned")
            .get(principal)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop one specific session index for a principal. Returns whether
    /// the index was present.
    pub fn drop_index(&self, principal: &str, session_index: &str) -> bool {
        let mut g = self.inner.write().expect("poisoned");
        if let Some(v) = g.get_mut(principal) {
            let before = v.len();
            v.retain(|s| s != session_index);
            let removed = v.len() < before;
            if v.is_empty() {
                g.remove(principal);
            }
            return removed;
        }
        false
    }

    /// Drop every index for a principal and return the set that was
    /// removed (so the IdP can fan-out a LogoutRequest to each SP).
    pub fn drop_all(&self, principal: &str) -> Vec<String> {
        let mut g = self.inner.write().expect("poisoned");
        g.remove(principal).unwrap_or_default()
    }

    pub fn principals(&self) -> Vec<String> {
        self.inner.read().expect("poisoned").keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_logout_response_rejects_missing_root() {
        assert!(parse_logout_response(b"<other/>").is_err());
    }

    #[test]
    fn ledger_track_is_idempotent() {
        let l = SessionIndexLedger::new();
        l.track("u", "s1");
        l.track("u", "s1");
        assert_eq!(l.indexes_for("u").len(), 1);
    }

    #[test]
    fn drop_index_returns_false_for_unknown() {
        let l = SessionIndexLedger::new();
        l.track("u", "s1");
        assert!(!l.drop_index("u", "missing"));
        assert!(l.drop_index("u", "s1"));
        assert!(l.indexes_for("u").is_empty());
    }

    #[test]
    fn principals_lists_tracked() {
        let l = SessionIndexLedger::new();
        l.track("alice", "s1");
        l.track("bob", "s2");
        let mut ps = l.principals();
        ps.sort();
        assert_eq!(ps, vec!["alice", "bob"]);
    }
}
