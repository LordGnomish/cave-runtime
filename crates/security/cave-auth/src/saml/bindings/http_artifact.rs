// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/web/util/ArtifactBindingUtil.java + saml-core-api/.../v2/protocol/ArtifactType.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! HTTP-Artifact binding — opaque 44-byte handle the receiver
//! resolves via the back-channel SOAP ArtifactResolve call.
//!
//! SAML 2.0 Bindings §3.6.4 defines the artifact format
//! (type 0x0004) as a 44-byte concatenation:
//!
//! ```text
//!   TypeCode      (2 bytes,  big-endian)  — 0x0004
//!   EndpointIndex (2 bytes,  big-endian)  — which ArtifactResolutionService endpoint to hit
//!   SourceID      (20 bytes)              — SHA-1 of the IdP entity ID
//!   MessageHandle (20 bytes)              — random per-message handle
//! ```
//!
//! The handle is base64-encoded into the `SAMLart=` query/form
//! parameter. The receiver POSTs an `<ArtifactResolve>` SOAP
//! envelope to the issuer's ArtifactResolutionService endpoint,
//! which returns the original SAML message wrapped in an
//! `<ArtifactResponse>`. Decoding here is wire-format only; the
//! SOAP resolution step lives in the broker.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use crate::saml::SamlError;
use std::collections::HashMap;

/// Length of a SAML 2.0 type-0x0004 artifact in bytes.
pub const ARTIFACT_LEN: usize = 44;
/// SAML 2.0 type-0x0004 artifact type code.
pub const ARTIFACT_TYPE_0004: u16 = 0x0004;

/// A SAML 2.0 type-0x0004 artifact — the only artifact type the
/// spec actually defines. `type_code` is always `0x0004` in
/// practice; we keep it as a field so the parser can reject any
/// future type the IdP federates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Artifact {
    pub type_code: u16,
    pub endpoint_index: u16,
    pub source_id: [u8; 20],
    pub message_handle: [u8; 20],
}

impl Artifact {
    /// Build a fresh artifact. `source_id` is typically the SHA-1
    /// hash of the issuer entity ID; `message_handle` is 20 bytes
    /// of fresh randomness.
    pub fn new(
        type_code: u16,
        endpoint_index: u16,
        source_id: [u8; 20],
        message_handle: [u8; 20],
    ) -> Self {
        Artifact {
            type_code,
            endpoint_index,
            source_id,
            message_handle,
        }
    }

    /// Encode as the 44-byte wire form (big-endian).
    pub fn to_bytes(&self) -> [u8; ARTIFACT_LEN] {
        let mut out = [0u8; ARTIFACT_LEN];
        out[0..2].copy_from_slice(&self.type_code.to_be_bytes());
        out[2..4].copy_from_slice(&self.endpoint_index.to_be_bytes());
        out[4..24].copy_from_slice(&self.source_id);
        out[24..44].copy_from_slice(&self.message_handle);
        out
    }

    /// Base64-encode the wire form — the `SAMLart=` query param
    /// value.
    pub fn to_base64(&self) -> String {
        B64.encode(self.to_bytes())
    }

    /// Parse the 44-byte wire form.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SamlError> {
        if bytes.len() != ARTIFACT_LEN {
            return Err(SamlError::Binding(format!(
                "artifact length {} != {}",
                bytes.len(),
                ARTIFACT_LEN
            )));
        }
        let type_code = u16::from_be_bytes([bytes[0], bytes[1]]);
        let endpoint_index = u16::from_be_bytes([bytes[2], bytes[3]]);
        let mut source_id = [0u8; 20];
        source_id.copy_from_slice(&bytes[4..24]);
        let mut message_handle = [0u8; 20];
        message_handle.copy_from_slice(&bytes[24..44]);
        Ok(Artifact {
            type_code,
            endpoint_index,
            source_id,
            message_handle,
        })
    }

    /// Parse from the base64-encoded `SAMLart=` query parameter.
    pub fn from_base64(encoded: &str) -> Result<Self, SamlError> {
        let bytes = B64
            .decode(encoded)
            .map_err(|err| SamlError::Binding(format!("base64: {err}")))?;
        Self::from_bytes(&bytes)
    }

    /// Is this a recognised type-0x0004 artifact? cave-auth only
    /// supports the one type the spec defines; this helper exists
    /// so the receiver can reject future types politely.
    pub fn is_type_0004(&self) -> bool {
        self.type_code == ARTIFACT_TYPE_0004
    }
}

/// Build the `<samlp:ArtifactResolve>` SOAP body the receiver
/// POSTs back to the issuer's ArtifactResolutionService endpoint.
/// XML emission only — does not perform the network call.
///
/// Mirrors `ArtifactResolveType` from `saml-core-api`.
pub fn build_artifact_resolve(
    id: &str,
    issue_instant: &str,
    issuer: &str,
    artifact: &str,
) -> String {
    format!(
        r#"<samlp:ArtifactResolve xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="{issue_instant}"><saml:Issuer>{issuer}</saml:Issuer><samlp:Artifact>{artifact}</samlp:Artifact></samlp:ArtifactResolve>"#,
        id = id,
        issue_instant = issue_instant,
        issuer = issuer,
        artifact = artifact,
    )
}

/// Wrap an `<ArtifactResolve>` (or any SAML element) in the SOAP
/// 1.1 envelope Keycloak's back-channel ArtifactBinding uses.
pub fn wrap_soap(saml_body_xml: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>{}</soap:Body></soap:Envelope>"#,
        saml_body_xml
    )
}

// ===========================================================================
// Back-channel resolver layer — <samlp:ArtifactResolve> / <ArtifactResponse>
// parsing + the source-side single-use artifact store. Mirrors
// keycloak `ArtifactResolveType` / `ArtifactResponseType` and the
// `SamlService.artifactResolution` source store (SAML Bindings §3.6.3-3.6.4).
// XML/format only — no network, no SOAP transport beyond [`wrap_soap`].
// ===========================================================================

/// SAML 2.0 top-level `Success` status URI.
pub const STATUS_SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";

/// A parsed `<samlp:ArtifactResolve>` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactResolve {
    pub id: String,
    pub issuer: String,
    /// Base64 `SAMLart` value to resolve.
    pub artifact: String,
}

/// A parsed `<samlp:ArtifactResponse>` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactResponse {
    pub id: String,
    pub in_response_to: String,
    pub issuer: String,
    /// Top-level status URI (e.g. [`STATUS_SUCCESS`]).
    pub status: String,
    /// Raw XML of the resolved protocol message (e.g. a `<samlp:Response>`).
    pub payload: String,
}

/// Parse a `<samlp:ArtifactResolve>` (bare or SOAP-wrapped).
pub fn parse_artifact_resolve(xml: &str) -> Result<ArtifactResolve, SamlError> {
    if open_tag_lt(xml, "ArtifactResolve", 0).is_none() {
        return Err(SamlError::Parse("not an ArtifactResolve".into()));
    }
    let id = attr(xml, "ArtifactResolve", "ID")
        .ok_or_else(|| SamlError::MissingField("ArtifactResolve@ID".into()))?;
    let issuer =
        elem_text(xml, "Issuer").ok_or_else(|| SamlError::MissingField("Issuer".into()))?;
    let artifact =
        elem_text(xml, "Artifact").ok_or_else(|| SamlError::MissingField("Artifact".into()))?;
    Ok(ArtifactResolve { id, issuer, artifact })
}

/// Build a `<samlp:ArtifactResponse>` carrying `payload` (raw XML). XML
/// emission only; wrap with [`wrap_soap`] for the back-channel POST.
/// Mirrors `ArtifactResponseType` from `saml-core-api`.
pub fn build_artifact_response(
    id: &str,
    issue_instant: &str,
    in_response_to: &str,
    issuer: &str,
    status: &str,
    payload: &str,
) -> String {
    format!(
        r#"<samlp:ArtifactResponse xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="{issue_instant}" InResponseTo="{in_response_to}"><saml:Issuer>{issuer}</saml:Issuer><samlp:Status><samlp:StatusCode Value="{status}"/></samlp:Status>{payload}</samlp:ArtifactResponse>"#,
        id = id,
        issue_instant = issue_instant,
        in_response_to = in_response_to,
        issuer = issuer,
        status = status,
        payload = payload.trim(),
    )
}

/// Parse a `<samlp:ArtifactResponse>` (bare or SOAP-wrapped), recovering the
/// inner protocol message.
pub fn parse_artifact_response(xml: &str) -> Result<ArtifactResponse, SamlError> {
    let ar_lt = open_tag_lt(xml, "ArtifactResponse", 0)
        .ok_or_else(|| SamlError::Parse("not an ArtifactResponse".into()))?;
    let id = attr(xml, "ArtifactResponse", "ID")
        .ok_or_else(|| SamlError::MissingField("ArtifactResponse@ID".into()))?;
    let in_response_to = attr(xml, "ArtifactResponse", "InResponseTo").unwrap_or_default();
    let issuer =
        elem_text(xml, "Issuer").ok_or_else(|| SamlError::MissingField("Issuer".into()))?;
    let status = attr(xml, "StatusCode", "Value")
        .ok_or_else(|| SamlError::MissingField("StatusCode@Value".into()))?;

    // Payload = everything between the end of <Status> and the
    // </ArtifactResponse> close tag.
    let ar_gt = start_tag_gt(xml, ar_lt)
        .ok_or_else(|| SamlError::Parse("unterminated ArtifactResponse start tag".into()))?;
    let inner_start = ar_gt + 1;
    let status_end = close_tag_end(xml, "Status", inner_start)
        .ok_or_else(|| SamlError::Parse("unterminated Status".into()))?;
    let ar_close_end = close_tag_end(xml, "ArtifactResponse", inner_start)
        .ok_or_else(|| SamlError::Parse("unterminated ArtifactResponse".into()))?;
    let ar_close_lt = xml[..ar_close_end]
        .rfind("</")
        .ok_or_else(|| SamlError::Parse("bad ArtifactResponse close".into()))?;
    let payload = xml[status_end..ar_close_lt].trim().to_string();

    Ok(ArtifactResponse {
        id,
        in_response_to,
        issuer,
        status,
        payload,
    })
}

/// Source-side single-use artifact store (SAML Bindings §3.6.3). The issuer
/// stores a protocol message keyed by the artifact it hands out, then
/// resolves-and-consumes it exactly once when the peer presents a matching
/// `<ArtifactResolve>`. Mirrors the transient artifact map Keycloak's
/// `SamlService` keeps for the source-site role.
#[derive(Debug, Default, Clone)]
pub struct ArtifactResolver {
    map: HashMap<String, String>,
}

impl ArtifactResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store `message` keyed by the artifact's base64 form; returns the
    /// `SAMLart` the issuer hands to the peer.
    pub fn store(&mut self, artifact: &Artifact, message: &str) -> String {
        let key = artifact.to_base64();
        self.map.insert(key.clone(), message.to_string());
        key
    }

    /// Resolve and **consume** (single-use). Errors if the artifact was never
    /// issued or has already been resolved — artifacts are one-time-use per
    /// the spec, which thwarts replay.
    pub fn resolve(&mut self, artifact_b64: &str) -> Result<String, SamlError> {
        self.map.remove(artifact_b64.trim()).ok_or_else(|| {
            SamlError::Other("artifact not found (already resolved or never issued)".into())
        })
    }

    /// Number of artifacts still awaiting resolution.
    pub fn pending(&self) -> usize {
        self.map.len()
    }
}

// ---- minimal namespace-insensitive XML helpers (hand-rolled, matching the
//      string-extraction idiom this module already uses) ----

/// Byte index of `<` for the opening tag of the element with local name
/// `local`, searching from `from`. Namespace-prefix insensitive; skips
/// `</`, `<!`, `<?`.
fn open_tag_lt(xml: &str, local: &str, from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(rel) = xml[i..].find('<') {
        let lt = i + rel;
        let after = &xml[lt + 1..];
        if after.starts_with('/') || after.starts_with('!') || after.starts_with('?') {
            i = lt + 1;
            continue;
        }
        let name_end = after
            .find(|c: char| {
                c == ' ' || c == '>' || c == '/' || c == '\t' || c == '\n' || c == '\r'
            })
            .unwrap_or(after.len());
        let name = &after[..name_end];
        let local_part = name.rsplit(':').next().unwrap_or(name);
        if local_part == local {
            return Some(lt);
        }
        i = lt + 1;
    }
    None
}

/// Byte index of the `>` ending the start tag that begins at `lt`.
fn start_tag_gt(xml: &str, lt: usize) -> Option<usize> {
    xml[lt..].find('>').map(|r| lt + r)
}

/// Byte index just after the `>` of the first matching close tag
/// `</...local>` at/after `from`. Namespace-prefix insensitive.
fn close_tag_end(xml: &str, local: &str, from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(rel) = xml[i..].find("</") {
        let lt = i + rel;
        let after = &xml[lt + 2..];
        let name_end = after
            .find(|c: char| c == ' ' || c == '>' || c == '\t' || c == '\n' || c == '\r')
            .unwrap_or(after.len());
        let name = &after[..name_end];
        let local_part = name.rsplit(':').next().unwrap_or(name);
        if local_part == local {
            let gt = xml[lt..].find('>')? + lt;
            return Some(gt + 1);
        }
        i = lt + 2;
    }
    None
}

/// Text content of the first element with local name `local`.
fn elem_text(xml: &str, local: &str) -> Option<String> {
    let lt = open_tag_lt(xml, local, 0)?;
    let gt = start_tag_gt(xml, lt)?;
    if xml.as_bytes().get(gt.wrapping_sub(1)) == Some(&b'/') {
        return Some(String::new()); // self-closing
    }
    let inner_start = gt + 1;
    let close_end = close_tag_end(xml, local, inner_start)?;
    let close_lt = xml[inner_start..close_end]
        .rfind('<')
        .map(|r| inner_start + r)?;
    Some(xunesc(xml[inner_start..close_lt].trim()))
}

/// Value of attribute `name` on the first element with local name `local`.
fn attr(xml: &str, local: &str, name: &str) -> Option<String> {
    let lt = open_tag_lt(xml, local, 0)?;
    let gt = start_tag_gt(xml, lt)?;
    let tag = &xml[lt..=gt];
    let needle = format!("{name}=\"");
    let p = tag.find(&needle)? + needle.len();
    let rest = &tag[p..];
    let end = rest.find('"')?;
    Some(xunesc(&rest[..end]))
}

/// Decode the five predefined XML entities. `&amp;` is decoded last so an
/// already-escaped `&amp;lt;` does not collapse to `<`.
fn xunesc(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_bytes_round_trip() {
        let src = [0xaau8; 20];
        let msg = [0xbbu8; 20];
        let art = Artifact::new(ARTIFACT_TYPE_0004, 7, src, msg);
        let bytes = art.to_bytes();
        let decoded = Artifact::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, art);
    }

    #[test]
    fn artifact_wire_format_is_44_bytes() {
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [0u8; 20], [0u8; 20]);
        assert_eq!(art.to_bytes().len(), ARTIFACT_LEN);
    }

    #[test]
    fn artifact_base64_round_trip() {
        let src = [0x01u8; 20];
        let msg = [0x02u8; 20];
        let art = Artifact::new(ARTIFACT_TYPE_0004, 1, src, msg);
        let b64 = art.to_base64();
        let decoded = Artifact::from_base64(&b64).unwrap();
        assert_eq!(decoded.type_code, ARTIFACT_TYPE_0004);
        assert_eq!(decoded.endpoint_index, 1);
        assert_eq!(decoded.source_id, src);
        assert_eq!(decoded.message_handle, msg);
    }

    #[test]
    fn artifact_endpoint_index_big_endian() {
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0x1234, [0u8; 20], [0u8; 20]);
        let bytes = art.to_bytes();
        // Bytes 2..4 are the EndpointIndex, big-endian.
        assert_eq!(bytes[2], 0x12);
        assert_eq!(bytes[3], 0x34);
    }

    #[test]
    fn artifact_type_code_big_endian() {
        let art = Artifact::new(0x00f0, 0, [0u8; 20], [0u8; 20]);
        let bytes = art.to_bytes();
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], 0xf0);
    }

    #[test]
    fn artifact_from_bytes_rejects_wrong_length() {
        assert!(Artifact::from_bytes(&[0u8; 43]).is_err());
        assert!(Artifact::from_bytes(&[0u8; 45]).is_err());
        assert!(Artifact::from_bytes(&[]).is_err());
    }

    #[test]
    fn artifact_from_base64_rejects_bad_base64() {
        assert!(Artifact::from_base64("!@#$").is_err());
    }

    #[test]
    fn artifact_from_base64_rejects_short_decoded() {
        // 4 base64 chars decode to 3 bytes — way short of 44.
        let short = B64.encode(b"hi!");
        assert!(Artifact::from_base64(&short).is_err());
    }

    #[test]
    fn artifact_is_type_0004_returns_true_for_canonical() {
        let a = Artifact::new(ARTIFACT_TYPE_0004, 0, [0u8; 20], [0u8; 20]);
        assert!(a.is_type_0004());
    }

    #[test]
    fn artifact_is_type_0004_returns_false_for_other_types() {
        let a = Artifact::new(0x0005, 0, [0u8; 20], [0u8; 20]);
        assert!(!a.is_type_0004());
    }

    #[test]
    fn artifact_source_id_and_message_handle_distinct() {
        let src = [0x33u8; 20];
        let msg = [0x44u8; 20];
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, src, msg);
        let bytes = art.to_bytes();
        // Bytes 4..24 are SourceID, 24..44 are MessageHandle.
        assert_eq!(&bytes[4..24], &src);
        assert_eq!(&bytes[24..44], &msg);
    }

    #[test]
    fn build_artifact_resolve_has_all_required_fields() {
        let resolve = build_artifact_resolve(
            "_resolve-1",
            "2026-05-15T10:00:00Z",
            "https://sp.example.com",
            "AAQAAA==",
        );
        assert!(resolve.contains("ID=\"_resolve-1\""));
        assert!(resolve.contains("Version=\"2.0\""));
        assert!(resolve.contains("IssueInstant=\"2026-05-15T10:00:00Z\""));
        assert!(resolve.contains("<saml:Issuer>https://sp.example.com</saml:Issuer>"));
        assert!(resolve.contains("<samlp:Artifact>AAQAAA==</samlp:Artifact>"));
    }

    #[test]
    fn wrap_soap_envelope_is_well_formed() {
        let envelope = wrap_soap("<dummy/>");
        assert!(envelope.starts_with(r#"<?xml version="1.0""#));
        assert!(envelope.contains("<soap:Envelope"));
        assert!(envelope.contains("<soap:Body><dummy/></soap:Body>"));
        assert!(envelope.ends_with("</soap:Envelope>"));
    }

    // ---- Back-channel resolver layer (cont3 depth) ----

    #[test]
    fn artifact_resolve_parses_back() {
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [1u8; 20], [2u8; 20]);
        let xml = build_artifact_resolve(
            "_q1",
            "2026-05-31T00:00:00Z",
            "https://sp.example.com",
            &art.to_base64(),
        );
        let parsed = parse_artifact_resolve(&xml).unwrap();
        assert_eq!(parsed.id, "_q1");
        assert_eq!(parsed.issuer, "https://sp.example.com");
        assert_eq!(parsed.artifact, art.to_base64());
    }

    #[test]
    fn artifact_resolve_parses_inside_soap_envelope() {
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [9u8; 20], [8u8; 20]);
        let inner = build_artifact_resolve(
            "_q2",
            "2026-05-31T00:00:00Z",
            "https://sp.example.com",
            &art.to_base64(),
        );
        let soap = wrap_soap(&inner);
        let parsed = parse_artifact_resolve(&soap).unwrap();
        assert_eq!(parsed.id, "_q2");
        assert_eq!(parsed.artifact, art.to_base64());
    }

    #[test]
    fn artifact_response_round_trips_with_payload() {
        let payload = r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_r1" Version="2.0"><saml:Issuer xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">https://idp.example.com</saml:Issuer></samlp:Response>"#;
        let xml = build_artifact_response(
            "_a1",
            "2026-05-31T00:00:00Z",
            "_q1",
            "https://idp.example.com",
            STATUS_SUCCESS,
            payload,
        );
        let parsed = parse_artifact_response(&xml).unwrap();
        assert_eq!(parsed.id, "_a1");
        assert_eq!(parsed.in_response_to, "_q1");
        assert_eq!(parsed.issuer, "https://idp.example.com");
        assert_eq!(parsed.status, STATUS_SUCCESS);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn resolver_is_single_use() {
        let mut r = ArtifactResolver::new();
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [4u8; 20], [5u8; 20]);
        let samlart = r.store(&art, "<msg/>");
        assert_eq!(r.pending(), 1);
        assert_eq!(r.resolve(&samlart).unwrap(), "<msg/>");
        assert_eq!(r.pending(), 0);
        assert!(r.resolve(&samlart).is_err()); // consumed
    }

    #[test]
    fn resolver_full_back_channel_flow() {
        let mut idp = ArtifactResolver::new();
        let response_msg = r#"<samlp:Response ID="_x"/>"#;
        let art = Artifact::new(ARTIFACT_TYPE_0004, 0, [3u8; 20], [7u8; 20]);
        let samlart = idp.store(&art, response_msg);
        // SP side: build the resolve from the artifact it received.
        let resolve_xml = build_artifact_resolve(
            "_q",
            "2026-05-31T00:00:00Z",
            "https://sp.example.com",
            &samlart,
        );
        // IdP side: parse, consume the stored message, build the response.
        let req = parse_artifact_resolve(&resolve_xml).unwrap();
        let msg = idp.resolve(&req.artifact).unwrap();
        let resp_xml = build_artifact_response(
            "_a",
            "2026-05-31T00:00:00Z",
            &req.id,
            "https://idp.example.com",
            STATUS_SUCCESS,
            &msg,
        );
        // SP side: parse the response, recover the original payload.
        let resp = parse_artifact_response(&resp_xml).unwrap();
        assert_eq!(resp.in_response_to, "_q");
        assert_eq!(resp.payload, response_msg);
    }

    #[test]
    fn parse_artifact_resolve_rejects_non_resolve() {
        assert!(parse_artifact_resolve("<soap:Envelope/>").is_err());
    }
}
