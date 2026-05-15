// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/web/util/PostBindingUtil.java + saml-core/.../web/SAMLPostBinding.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! HTTP-POST binding — base64 only. Used for SAML Responses that
//! carry an Assertion + signature; the body exceeds the URL
//! length limit Redirect binds against.
//!
//! Mirrors Keycloak's `PostBindingUtil`. SAML 2.0 Bindings §3.5.4
//! mandates standard base64 (RFC 4648 §4), no DEFLATE,
//! line-breaks-permitted in transport.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::saml::SamlError;

/// Encode XML for the HTTP-POST binding — base64, no DEFLATE.
pub fn encode(xml: &[u8]) -> String {
    B64.encode(xml)
}

/// Decode an HTTP-POST-binding form value back to XML bytes.
/// Tolerates `\r\n` line wrapping the way SAML deployments
/// historically emit (browsers occasionally insert it).
pub fn decode(encoded: &str) -> Result<Vec<u8>, SamlError> {
    // Strip whitespace first — base64 itself rejects bare
    // whitespace, but the SAML wire format permits line-wrapping.
    let cleaned: String = encoded.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    B64.decode(cleaned.as_bytes())
        .map_err(|err| SamlError::Binding(format!("base64: {err}")))
}

/// Build the auto-submit HTML form Keycloak's
/// `SAMLPostBinding.java` emits — the IdP returns this and the
/// browser POSTs it to the SP's ACS URL on body-load.
///
/// The returned HTML embeds `saml_param_value` already-base64-encoded
/// and (if present) `relay_state`. Caller is responsible for
/// HTML-escaping the destination URL.
pub fn auto_submit_form(
    destination: &str,
    saml_param_name: &str,
    saml_param_value_b64: &str,
    relay_state: Option<&str>,
) -> String {
    let mut form = String::new();
    form.push_str("<!DOCTYPE html><html><body onload=\"document.forms[0].submit()\">");
    form.push_str(&format!(
        "<form method=\"POST\" action=\"{}\">",
        html_escape(destination)
    ));
    form.push_str(&format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\"/>",
        html_escape(saml_param_name),
        html_escape(saml_param_value_b64)
    ));
    if let Some(rs) = relay_state {
        form.push_str(&format!(
            "<input type=\"hidden\" name=\"RelayState\" value=\"{}\"/>",
            html_escape(rs)
        ));
    }
    form.push_str("<noscript><button type=\"submit\">Continue</button></noscript>");
    form.push_str("</form></body></html>");
    form
}

/// Minimal HTML attribute escaper — five reserved chars.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] =
        br#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_r" Version="2.0"/>"#;

    #[test]
    fn encode_decode_round_trip() {
        let enc = encode(SAMPLE);
        let dec = decode(&enc).unwrap();
        assert_eq!(dec, SAMPLE);
    }

    #[test]
    fn decode_rejects_bad_base64() {
        assert!(decode("!@#$").is_err());
    }

    #[test]
    fn decode_strips_whitespace_in_wrapped_lines() {
        // Wrap to 60 chars per line — real-world IdPs do this.
        let enc = encode(SAMPLE);
        let wrapped: String = enc
            .as_bytes()
            .chunks(60)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n");
        let dec = decode(&wrapped).unwrap();
        assert_eq!(dec, SAMPLE);
    }

    #[test]
    fn auto_submit_form_has_destination_and_response_field() {
        let form = auto_submit_form(
            "https://sp.example.com/saml/acs",
            "SAMLResponse",
            "BASE64==",
            Some("relay-state-1"),
        );
        assert!(form.contains("action=\"https://sp.example.com/saml/acs\""));
        assert!(form.contains("name=\"SAMLResponse\""));
        assert!(form.contains("value=\"BASE64==\""));
        assert!(form.contains("value=\"relay-state-1\""));
        assert!(form.contains("onload=\"document.forms[0].submit()\""));
    }

    #[test]
    fn auto_submit_form_omits_relay_state_when_none() {
        let form =
            auto_submit_form("https://sp.example.com/acs", "SAMLResponse", "B64=", None);
        assert!(!form.contains("RelayState"));
    }

    #[test]
    fn auto_submit_form_html_escapes_destination() {
        let form = auto_submit_form(
            "https://sp.example.com/?q=<x>&y=1",
            "SAMLResponse",
            "B64=",
            None,
        );
        assert!(form.contains("&lt;x&gt;"));
        assert!(form.contains("&amp;y=1"));
    }

    #[test]
    fn auto_submit_form_has_noscript_fallback() {
        let form = auto_submit_form("https://sp/acs", "SAMLResponse", "B64=", None);
        assert!(form.contains("<noscript>"));
        assert!(form.contains("Continue"));
    }
}
