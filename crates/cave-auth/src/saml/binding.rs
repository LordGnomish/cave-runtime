// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SAML transport bindings — how messages travel between SP and
//! IdP over HTTP. cave-auth implements the two front-channel
//! bindings every real-world IdP supports:
//!
//! * **HTTP-Redirect** — raw-deflate compress + base64 + URL.
//!   Used for `AuthnRequest`s and small responses that fit
//!   inside a URL.
//! * **HTTP-POST** — base64 only. Used for `Response`s, which
//!   are too big for a URL and carry the assertion plus its
//!   signature.
//!
//! Mirrors `org.keycloak.saml.processing.web.util.PostBindingUtil`
//! + `RedirectBindingUtil` from upstream.

use std::io::{Read, Write};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

use super::SamlError;

/// Spec URN for HTTP-Redirect.
pub const BINDING_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
/// Spec URN for HTTP-POST.
pub const BINDING_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";

/// Encode XML bytes for the HTTP-Redirect binding. Returns the
/// value to drop into the `SAMLRequest=` (or `SAMLResponse=`)
/// query param — caller is responsible for URL-encoding the
/// resulting string.
pub fn redirect_encode(xml: &[u8]) -> Result<String, SamlError> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(xml)
        .map_err(|err| SamlError::Binding(format!("deflate: {err}")))?;
    let deflated = e
        .finish()
        .map_err(|err| SamlError::Binding(format!("deflate finish: {err}")))?;
    Ok(B64.encode(deflated))
}

/// Decode an HTTP-Redirect-binding `SAMLRequest=` value back to
/// XML bytes. Inverse of [`redirect_encode`].
pub fn redirect_decode(encoded: &str) -> Result<Vec<u8>, SamlError> {
    let deflated = B64
        .decode(encoded)
        .map_err(|err| SamlError::Binding(format!("base64: {err}")))?;
    let mut d = DeflateDecoder::new(deflated.as_slice());
    let mut out = Vec::new();
    d.read_to_end(&mut out)
        .map_err(|err| SamlError::Binding(format!("inflate: {err}")))?;
    Ok(out)
}

/// Encode XML for the HTTP-POST binding — just base64.
pub fn post_encode(xml: &[u8]) -> String {
    B64.encode(xml)
}

/// Decode an HTTP-POST-binding form value back to XML bytes.
pub fn post_decode(encoded: &str) -> Result<Vec<u8>, SamlError> {
    B64.decode(encoded)
        .map_err(|err| SamlError::Binding(format!("base64: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
        ID="_abc" Version="2.0" IssueInstant="2026-05-13T10:00:00Z" Destination="d"/>"#;

    #[test]
    fn redirect_encode_decode_round_trips() {
        let enc = redirect_encode(SAMPLE).unwrap();
        let dec = redirect_decode(&enc).unwrap();
        assert_eq!(dec, SAMPLE);
    }

    #[test]
    fn redirect_encoding_is_compact_for_xml() {
        let enc = redirect_encode(SAMPLE).unwrap();
        // Deflate + base64 of a small XML doc should still be
        // a meaningful compression vs raw base64.
        let raw_b64 = post_encode(SAMPLE);
        assert!(enc.len() < raw_b64.len(), "redirect: {}  post: {}", enc.len(), raw_b64.len());
    }

    #[test]
    fn redirect_decode_rejects_bad_base64() {
        assert!(redirect_decode("!not!base64!").is_err());
    }

    #[test]
    fn redirect_decode_rejects_garbage_inflate() {
        let bytes = B64.encode(b"this is not deflate output");
        assert!(redirect_decode(&bytes).is_err());
    }

    #[test]
    fn post_encode_decode_round_trips() {
        let enc = post_encode(SAMPLE);
        let dec = post_decode(&enc).unwrap();
        assert_eq!(dec, SAMPLE);
    }

    #[test]
    fn post_decode_rejects_bad_base64() {
        assert!(post_decode("!@#$").is_err());
    }
}
