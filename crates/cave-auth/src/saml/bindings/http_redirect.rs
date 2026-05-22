// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/web/util/RedirectBindingUtil.java + saml-core/.../web/SAMLRedirectBinding.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! HTTP-Redirect binding — raw-deflate compress + base64. Used by
//! the IdP for AuthnRequests and small Responses that fit inside
//! a URL.
//!
//! Mirrors Keycloak's `RedirectBindingUtil`. The earlier
//! `saml::binding::redirect_*` free functions implement the same
//! pipeline; this module exposes them under the per-binding
//! module-name pattern upstream uses, plus a deflated-Signature
//! helper for the signed-Redirect profile.

use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::saml::SamlError;

/// Encode XML bytes for the HTTP-Redirect binding. Returns the
/// value to drop into the `SAMLRequest=` (or `SAMLResponse=`)
/// query param — caller is responsible for URL-encoding the
/// resulting string.
///
/// SAML 2.0 Bindings §3.4.4.1: raw DEFLATE (no zlib header) then
/// standard base64.
pub fn encode(xml: &[u8]) -> Result<String, SamlError> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(xml)
        .map_err(|err| SamlError::Binding(format!("deflate: {err}")))?;
    let deflated = e
        .finish()
        .map_err(|err| SamlError::Binding(format!("deflate finish: {err}")))?;
    Ok(B64.encode(deflated))
}

/// Decode an HTTP-Redirect-binding `SAMLRequest=` value back to
/// XML bytes. Inverse of [`encode`].
pub fn decode(encoded: &str) -> Result<Vec<u8>, SamlError> {
    let deflated = B64
        .decode(encoded)
        .map_err(|err| SamlError::Binding(format!("base64: {err}")))?;
    let mut d = DeflateDecoder::new(deflated.as_slice());
    let mut out = Vec::new();
    d.read_to_end(&mut out)
        .map_err(|err| SamlError::Binding(format!("inflate: {err}")))?;
    Ok(out)
}

/// Construct the signed-Redirect signing payload per SAML 2.0
/// Bindings §3.4.4.1 — the URL-encoded concatenation
/// `SAMLRequest=...&RelayState=...&SigAlg=...` *without* the
/// `Signature=` parameter. The IdP signs this byte sequence
/// directly; the receiver re-builds the same string and verifies.
///
/// This is the place real-world Redirect-binding signature bugs
/// land: parameter order matters, and `RelayState` is omitted if
/// absent (not included as an empty value).
pub fn signing_payload(
    saml_param_name: &str,
    saml_param_value_urlenc: &str,
    relay_state_urlenc: Option<&str>,
    sig_alg_urlenc: &str,
) -> String {
    let mut out = String::new();
    out.push_str(saml_param_name);
    out.push('=');
    out.push_str(saml_param_value_urlenc);
    if let Some(rs) = relay_state_urlenc {
        out.push_str("&RelayState=");
        out.push_str(rs);
    }
    out.push_str("&SigAlg=");
    out.push_str(sig_alg_urlenc);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_abc" Version="2.0" IssueInstant="2026-05-13T10:00:00Z" Destination="d"/>"#;

    #[test]
    fn encode_decode_round_trip() {
        let enc = encode(SAMPLE).unwrap();
        let dec = decode(&enc).unwrap();
        assert_eq!(dec, SAMPLE);
    }

    #[test]
    fn encoding_compresses_repetitive_xml() {
        // Repetitive payload — DEFLATE should win meaningfully
        // against the raw-base64 alternative.
        let repeated: Vec<u8> = SAMPLE
            .iter()
            .cycle()
            .take(SAMPLE.len() * 4)
            .copied()
            .collect();
        let enc = encode(&repeated).unwrap();
        let raw_b64 = B64.encode(&repeated);
        assert!(
            enc.len() < raw_b64.len(),
            "deflate: {} vs raw-b64: {}",
            enc.len(),
            raw_b64.len()
        );
    }

    #[test]
    fn decode_rejects_bad_base64() {
        assert!(decode("!not!base64!").is_err());
    }

    #[test]
    fn decode_rejects_garbage_inflate() {
        let bytes = B64.encode(b"this is not deflate output");
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn signing_payload_with_relay_state() {
        let p = signing_payload("SAMLRequest", "abc%3D", Some("rs%2F1"), "http%3A%2F%2Falg");
        assert_eq!(
            p,
            "SAMLRequest=abc%3D&RelayState=rs%2F1&SigAlg=http%3A%2F%2Falg"
        );
    }

    #[test]
    fn signing_payload_omits_relay_state_when_none() {
        let p = signing_payload("SAMLRequest", "abc%3D", None, "http%3A%2F%2Falg");
        assert_eq!(p, "SAMLRequest=abc%3D&SigAlg=http%3A%2F%2Falg");
        assert!(!p.contains("RelayState"));
    }

    #[test]
    fn signing_payload_uses_response_param_name_when_asked() {
        // §3.4.4.1: the same payload shape works for both
        // SAMLRequest and SAMLResponse — the param name is the
        // only thing that changes.
        let p = signing_payload("SAMLResponse", "xyz%3D", None, "http%3A%2F%2Frsa-sha256");
        assert!(p.starts_with("SAMLResponse="));
    }
}
