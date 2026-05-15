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

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::saml::SamlError;

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
}
