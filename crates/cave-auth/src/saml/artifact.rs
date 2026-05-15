// SPDX-License-Identifier: AGPL-3.0-or-later
//! SAML 2.0 HTTP-Artifact binding (§3.6) + back-channel ArtifactResolve /
//! ArtifactResponse SOAP messages.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/ArtifactResolveType.java
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/ArtifactResponseType.java
//!         services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java
//!
//! Type-4 artifact wire format per SAML 2.0 Bindings §3.6.4:
//!
//! ```text
//! 0       2       4                       24                      44
//! +-------+-------+-----------------------+-----------------------+
//! | Type  | Idx   | SourceID (20 bytes)   | MessageHandle (20 b)  |
//! | 0x0004| 0x0000| SHA-1 of issuer       | random                |
//! +-------+-------+-----------------------+-----------------------+
//! ```
//!
//! The artifact is base64-encoded into the `SAMLart=` query/form param.
//! When the SP receives it, it back-channel POSTs a SOAP envelope wrapping
//! `<samlp:ArtifactResolve>` to the IdP's `ArtifactResolutionService`. The
//! IdP replies with a SOAP envelope wrapping `<samlp:ArtifactResponse>`,
//! which itself wraps the actual `<samlp:Response>`.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, RwLock};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use rand::RngCore;

use super::ns;
use super::response::{Response, StatusCode};
use super::SamlError;

/// Spec URN for HTTP-Artifact.
pub const ARTIFACT_BINDING_URN: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Artifact";

/// SOAP 1.1 envelope namespace — the back-channel transport.
pub const SOAP_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// Type-4 artifact format constants. Bytes 0-1 = 0x0004, bytes 2-3 =
/// endpoint index (most IdPs ship a single ARS endpoint at index 0).
pub const TYPE_CODE_TYPE4: [u8; 2] = [0x00, 0x04];

/// SAML 2.0 type-4 artifact. 44 bytes on the wire after base64 decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// `EndpointIndex` — which `ArtifactResolutionService` of the IdP
    /// to hit. Usually `0`.
    pub endpoint_index: u16,
    /// 20-byte `SourceID` — the SHA-1 of the issuer entity ID per spec.
    /// We don't enforce the SHA-1 derivation here (caller supplies it)
    /// because cave-auth lets operators pin any 20-byte source id.
    pub source_id: [u8; 20],
    /// 20-byte `MessageHandle` — random opaque value the IdP uses to
    /// look up the cached response.
    pub message_handle: [u8; 20],
}

impl Artifact {
    /// Build a type-4 artifact from a 20-byte source id and a 20-byte
    /// message handle. Truncates / zero-pads if the inputs are off-size
    /// — real callers always pass exactly 20 bytes.
    pub fn new_type4(source_id: &[u8], message_handle: &[u8]) -> Self {
        let mut sid = [0u8; 20];
        let mut mh = [0u8; 20];
        let sl = source_id.len().min(20);
        let ml = message_handle.len().min(20);
        sid[..sl].copy_from_slice(&source_id[..sl]);
        mh[..ml].copy_from_slice(&message_handle[..ml]);
        Self {
            endpoint_index: 0,
            source_id: sid,
            message_handle: mh,
        }
    }

    /// Generate a fresh type-4 artifact with a random message handle.
    pub fn generate(source_id: &[u8]) -> Self {
        let mut mh = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut mh);
        Self::new_type4(source_id, &mh)
    }

    /// Serialise to the 44-byte wire form.
    pub fn to_bytes(&self) -> [u8; 44] {
        let mut out = [0u8; 44];
        out[0..2].copy_from_slice(&TYPE_CODE_TYPE4);
        out[2..4].copy_from_slice(&self.endpoint_index.to_be_bytes());
        out[4..24].copy_from_slice(&self.source_id);
        out[24..44].copy_from_slice(&self.message_handle);
        out
    }

    /// Base64-encode for the `SAMLart=` parameter.
    pub fn to_base64(&self) -> String {
        B64.encode(self.to_bytes())
    }

    /// Decode from a 44-byte buffer. Errors if the TypeCode isn't 0x0004
    /// or the length is wrong.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SamlError> {
        if bytes.len() != 44 {
            return Err(SamlError::Binding(format!(
                "artifact length {} != 44",
                bytes.len()
            )));
        }
        if bytes[0..2] != TYPE_CODE_TYPE4 {
            return Err(SamlError::Binding(format!(
                "unknown TypeCode {:02x}{:02x} (only type-4 is supported)",
                bytes[0], bytes[1]
            )));
        }
        let endpoint_index = u16::from_be_bytes([bytes[2], bytes[3]]);
        let mut source_id = [0u8; 20];
        let mut message_handle = [0u8; 20];
        source_id.copy_from_slice(&bytes[4..24]);
        message_handle.copy_from_slice(&bytes[24..44]);
        Ok(Self {
            endpoint_index,
            source_id,
            message_handle,
        })
    }

    /// Decode from a base64 `SAMLart=` string.
    pub fn from_base64(encoded: &str) -> Result<Self, SamlError> {
        let bytes = B64
            .decode(encoded)
            .map_err(|e| SamlError::Binding(format!("artifact base64: {e}")))?;
        Self::from_bytes(&bytes)
    }
}

/// A SAML 2.0 `<samlp:ArtifactResolve>` — the back-channel SOAP request
/// the SP sends to redeem an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactResolve {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    pub issuer: String,
    pub destination: String,
    /// Base64 artifact (the value of `SAMLart=`).
    pub artifact: String,
}

/// A SAML 2.0 `<samlp:ArtifactResponse>` — the IdP's reply wrapping a
/// full `<samlp:Response>` inside a SOAP envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactResponse {
    pub id: String,
    pub issue_instant: DateTime<Utc>,
    /// Matches the `ID` of the ArtifactResolve that produced this reply.
    pub in_response_to: String,
    pub issuer: String,
    pub status: StatusCode,
    /// Inner `<samlp:Response>` payload — `None` on resolution failure.
    pub inner_response: Option<Response>,
}

// ─── Writers ─────────────────────────────────────────────────────────────────

/// Wrap `ArtifactResolve` in a SOAP 1.1 envelope.
pub fn write_artifact_resolve(r: &ArtifactResolve) -> Result<Vec<u8>, SamlError> {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new(&mut buf);
    let issue = r.issue_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut env = BytesStart::new("soap:Envelope");
    env.push_attribute(("xmlns:soap", SOAP_NS));
    w.write_event(Event::Start(env)).map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("soap:Body")))
        .map_err(io_err)?;

    let mut req = BytesStart::new("samlp:ArtifactResolve");
    req.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
    req.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
    req.push_attribute(("ID", r.id.as_str()));
    req.push_attribute(("Version", "2.0"));
    req.push_attribute(("IssueInstant", issue.as_str()));
    req.push_attribute(("Destination", r.destination.as_str()));
    w.write_event(Event::Start(req)).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("saml:Issuer")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.issuer))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("saml:Issuer"))).map_err(io_err)?;

    w.write_event(Event::Start(BytesStart::new("samlp:Artifact")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&r.artifact))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("samlp:Artifact"))).map_err(io_err)?;

    w.write_event(Event::End(BytesEnd::new("samlp:ArtifactResolve"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("soap:Body"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("soap:Envelope"))).map_err(io_err)?;
    Ok(buf.into_inner())
}

/// Wrap `ArtifactResponse` (with optional inner `<samlp:Response>`) in
/// a SOAP 1.1 envelope.
pub fn write_artifact_response(r: &ArtifactResponse) -> Result<Vec<u8>, SamlError> {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new(&mut buf);
    let issue = r.issue_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut env = BytesStart::new("soap:Envelope");
    env.push_attribute(("xmlns:soap", SOAP_NS));
    w.write_event(Event::Start(env)).map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("soap:Body")))
        .map_err(io_err)?;

    let mut resp = BytesStart::new("samlp:ArtifactResponse");
    resp.push_attribute(("xmlns:samlp", ns::SAML_PROTOCOL));
    resp.push_attribute(("xmlns:saml", ns::SAML_ASSERTION));
    resp.push_attribute(("ID", r.id.as_str()));
    resp.push_attribute(("Version", "2.0"));
    resp.push_attribute(("IssueInstant", issue.as_str()));
    resp.push_attribute(("InResponseTo", r.in_response_to.as_str()));
    w.write_event(Event::Start(resp)).map_err(io_err)?;

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

    // Inline-write the inner Response by serialising then injecting its
    // bytes. We can't construct an arbitrary writer state from the inner
    // module here, so we splice the inner XML directly.
    if let Some(inner) = &r.inner_response {
        let inner_xml = inner.to_xml()?;
        // Strip any leading `<?xml ?>` preamble before splicing.
        let inner_str = std::str::from_utf8(&inner_xml)
            .map_err(|e| SamlError::Parse(format!("inner utf-8: {e}")))?;
        let inner_payload = strip_xml_prolog(inner_str);
        // raw() ensures the bytes are written verbatim (no escaping).
        w.write_event(Event::Text(quick_xml::events::BytesText::from_escaped(
            inner_payload,
        )))
        .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("samlp:ArtifactResponse"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("soap:Body"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("soap:Envelope"))).map_err(io_err)?;
    Ok(buf.into_inner())
}

fn strip_xml_prolog(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<?xml") {
        if let Some(idx) = rest.find("?>") {
            return rest[idx + 2..].trim_start();
        }
    }
    trimmed
}

// ─── Parsers ─────────────────────────────────────────────────────────────────

/// Parse a SOAP-wrapped `<samlp:ArtifactResolve>`.
pub fn parse_artifact_resolve(bytes: &[u8]) -> Result<ArtifactResolve, SamlError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut id = None;
    let mut issue_instant = None;
    let mut destination = None;
    let mut issuer = None;
    let mut artifact = None;
    let mut current: Option<&'static str> = None;
    let mut buf = Vec::new();
    let mut saw_resolve = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "ArtifactResolve" => {
                        saw_resolve = true;
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
                                _ => {}
                            }
                        }
                    }
                    "Issuer" => current = Some("issuer"),
                    "Artifact" => current = Some("artifact"),
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
                    Some("artifact") => artifact = Some(txt),
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == "Issuer" || name == "Artifact" {
                    current = None;
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if !saw_resolve {
        return Err(SamlError::MissingField("ArtifactResolve element".into()));
    }
    let id = id.ok_or_else(|| SamlError::MissingField("ArtifactResolve/ID".into()))?;
    let issue_instant = issue_instant
        .ok_or_else(|| SamlError::MissingField("ArtifactResolve/IssueInstant".into()))?;
    let destination = destination
        .ok_or_else(|| SamlError::MissingField("ArtifactResolve/Destination".into()))?;
    let issuer = issuer.ok_or_else(|| SamlError::MissingField("ArtifactResolve/Issuer".into()))?;
    let artifact =
        artifact.ok_or_else(|| SamlError::MissingField("ArtifactResolve/Artifact".into()))?;
    Ok(ArtifactResolve {
        id,
        issue_instant: DateTime::parse_from_rfc3339(&issue_instant)
            .map_err(|e| SamlError::Parse(format!("IssueInstant: {e}")))?
            .with_timezone(&Utc),
        destination,
        issuer,
        artifact,
    })
}

/// Parse a SOAP-wrapped `<samlp:ArtifactResponse>` and (if present) the
/// inner `<samlp:Response>`.
pub fn parse_artifact_response(bytes: &[u8]) -> Result<ArtifactResponse, SamlError> {
    // Two-pass parse: extract envelope attrs first, then re-scan the
    // buffer for an inner <samlp:Response> block to feed to the existing
    // Response parser. The depth of XML here is small (single SOAP body,
    // single ArtifactResponse, optional inner Response).
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut id = None;
    let mut issue_instant = None;
    let mut in_response_to = None;
    let mut issuer = None;
    let mut status = StatusCode::Responder;
    let mut current: Option<&'static str> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "ArtifactResponse" => {
                        for a in e.attributes().flatten() {
                            let k = local_name(a.key.as_ref());
                            let v = a
                                .unescape_value()
                                .map_err(|err| SamlError::Parse(err.to_string()))?
                                .into_owned();
                            match k.as_str() {
                                "ID" => id = Some(v),
                                "IssueInstant" => issue_instant = Some(v),
                                "InResponseTo" => in_response_to = Some(v),
                                _ => {}
                            }
                        }
                    }
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
                    "Issuer" => current = Some("issuer"),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t
                    .unescape()
                    .map_err(|err| SamlError::Parse(err.to_string()))?
                    .into_owned();
                if matches!(current, Some("issuer")) {
                    if issuer.is_none() {
                        issuer = Some(txt);
                    }
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

    // Inner Response: find the substring spanning `<samlp:Response …>` to
    // `</samlp:Response>` (or local-name `Response`) and hand it to the
    // existing parser. We tolerate either prefixed or unprefixed names.
    let s = std::str::from_utf8(bytes)
        .map_err(|e| SamlError::Parse(format!("utf-8: {e}")))?;
    let inner_response = extract_inner_response(s)?;

    let id = id.ok_or_else(|| SamlError::MissingField("ArtifactResponse/ID".into()))?;
    let issue_instant = issue_instant
        .ok_or_else(|| SamlError::MissingField("ArtifactResponse/IssueInstant".into()))?;
    let in_response_to = in_response_to
        .ok_or_else(|| SamlError::MissingField("ArtifactResponse/InResponseTo".into()))?;
    let issuer =
        issuer.ok_or_else(|| SamlError::MissingField("ArtifactResponse/Issuer".into()))?;

    Ok(ArtifactResponse {
        id,
        issue_instant: DateTime::parse_from_rfc3339(&issue_instant)
            .map_err(|e| SamlError::Parse(format!("IssueInstant: {e}")))?
            .with_timezone(&Utc),
        in_response_to,
        issuer,
        status,
        inner_response,
    })
}

fn extract_inner_response(s: &str) -> Result<Option<Response>, SamlError> {
    let candidates = ["<samlp:Response", "<Response"];
    let ends = ["</samlp:Response>", "</Response>"];
    for (open, close) in candidates.iter().zip(ends.iter()) {
        if let Some(o) = s.find(open) {
            if let Some(c) = s[o..].find(close) {
                let xml = &s[o..o + c + close.len()];
                return Ok(Some(Response::from_xml(xml.as_bytes())?));
            }
        }
    }
    Ok(None)
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

// ─── Store ───────────────────────────────────────────────────────────────────

/// Single-shot in-memory mapping of `SAMLart` → `<samlp:Response>` (per
/// SAML 2.0 §3.6.5 "Artifact Resolution"). The IdP `put`s a Response
/// when it mints the artifact; the SP `take`s it back-channel exactly
/// once. Re-`take` returns `None` to honor the spec's single-use rule.
#[derive(Clone, Default)]
pub struct ArtifactResolutionStore {
    inner: Arc<RwLock<HashMap<String, Response>>>,
}

impl ArtifactResolutionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, artifact_b64: String, response: Response) {
        self.inner
            .write()
            .expect("poisoned")
            .insert(artifact_b64, response);
    }

    pub fn take(&self, artifact_b64: &str) -> Option<Response> {
        self.inner.write().expect("poisoned").remove(artifact_b64)
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_endpoint_index_round_trips() {
        let mut a = Artifact::new_type4(&[1; 20], &[2; 20]);
        a.endpoint_index = 7;
        let s = a.to_base64();
        let b = Artifact::from_base64(&s).unwrap();
        assert_eq!(b.endpoint_index, 7);
    }

    #[test]
    fn artifact_rejects_wrong_type_code() {
        let mut bytes = [0u8; 44];
        bytes[0..2].copy_from_slice(&[0x00, 0x01]); // Type-1 (deprecated)
        let s = B64.encode(bytes);
        assert!(Artifact::from_base64(&s).is_err());
    }

    #[test]
    fn generate_produces_distinct_handles() {
        let a = Artifact::generate(&[0; 20]);
        let b = Artifact::generate(&[0; 20]);
        assert_ne!(a.message_handle, b.message_handle, "random fill");
    }

    #[test]
    fn strip_xml_prolog_drops_preamble() {
        assert_eq!(strip_xml_prolog("<?xml version=\"1.0\"?><x/>"), "<x/>");
        assert_eq!(strip_xml_prolog("<x/>"), "<x/>");
    }
}
