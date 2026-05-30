// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XML canonicalization — exclusive c14n (`exc-c14n`, rfc3741)
//! subset for SAML 2.0 messages.
//!
//! Mirrors the canonicalization step of upstream Keycloak's
//! `SAML2Signature.canonicalize` (which delegates to Apache
//! Santuario's `Canonicalizer.canonicalizeSubtree`). cave-auth
//! ships a *pure-Rust* subset sufficient for round-tripping the
//! messages we produce ourselves — see the limitations note
//! below.
//!
//! ## What's covered
//!
//! * No XML declaration in the output (per exc-c14n §2.1).
//! * Comments dropped (we don't sign over them).
//! * Self-closing tags expanded to `<tag></tag>` (the canonical
//!   shape).
//! * Attributes serialised in lexicographic order by qualified
//!   name.
//! * Namespace declarations emitted in lexicographic order by
//!   prefix; only declarations that are *visibly utilized* by
//!   the current element or its attributes are emitted (that is
//!   the "exclusive" in exc-c14n).
//! * Text nodes preserved verbatim (no whitespace normalization
//!   inside `<saml:Issuer>` etc.).
//! * Output is UTF-8.
//!
//! * Attribute values are canonicalized per Canonical XML 1.0
//!   §Processing Model — `&` `<` `"` escaped, `>` left literal,
//!   and the whitespace characters #x9 / #xA / #xD emitted as
//!   `&#x9;` / `&#xA;` / `&#xD;` character references.
//! * Text content escapes `&` `<` `>` and emits a carriage
//!   return (#xD) as `&#xD;`.
//! * CDATA sections are reserialised as escaped character data
//!   (Canonical XML 1.0 §3.4).
//!
//! ## Known limitations
//!
//! * No DTD / entity resolution. SAML messages don't carry
//!   either.
//! * `InclusiveNamespaces`-prefix lists are not yet parsed; the
//!   plain visibly-utilised algorithm is used.

use std::collections::BTreeMap;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::Reader;

use super::SamlError;

/// Canonicalize `xml` with the built-in exc-c14n subset. The
/// shape used by [`super::signature::SignedDocument`] —
/// `fn(&[u8]) -> Result<Vec<u8>, SamlError>`.
pub fn exc_c14n(xml: &[u8]) -> Result<Vec<u8>, SamlError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().expand_empty_elements = true;

    let mut out: Vec<u8> = Vec::new();

    let mut ns_stack: Vec<Vec<NsDeclLocal>> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| SamlError::Parse(format!("c14n read: {e}")))?
        {
            Event::Eof => break,
            Event::Decl(_) | Event::DocType(_) | Event::PI(_) => {
                // Drop per exc-c14n.
            }
            Event::Comment(_) => {
                // Without-comments profile.
            }
            Event::Start(e) => {
                let frame = build_frame(&e, &ns_stack)?;
                emit_start(&mut out, &e, &frame)?;
                ns_stack.push(frame.declared);
            }
            Event::End(e) => {
                emit_end(&mut out, &e)?;
                ns_stack.pop();
            }
            Event::Empty(e) => {
                // expand_empty_elements is set, so this should
                // not fire — but if quick-xml ever yields one
                // (eg. inside a CDATA edge case) we expand it
                // manually.
                let frame = build_frame(&e, &ns_stack)?;
                let name_bytes = e.name().as_ref().to_vec();
                let name_str = std::str::from_utf8(&name_bytes)
                    .map_err(|err| SamlError::Parse(format!("c14n empty name: {err}")))?
                    .to_string();
                emit_start(&mut out, &e, &frame)?;
                emit_end(&mut out, &BytesEnd::new(name_str.as_str()))?;
            }
            Event::Text(t) => {
                let txt = t
                    .unescape()
                    .map_err(|e| SamlError::Parse(format!("c14n text: {e}")))?
                    .into_owned();
                escape_text_into(&mut out, &txt);
            }
            Event::CData(c) => {
                // A CDATA section's content is *character data* —
                // canonical XML reserialises it as escaped text
                // (Canonical XML 1.0 §3.4).
                let txt = std::str::from_utf8(&c)
                    .map_err(|e| SamlError::Parse(format!("c14n cdata: {e}")))?
                    .to_string();
                escape_text_into(&mut out, &txt);
            } // Some quick-xml builds emit GeneralRef for `&amp;`
              // etc. — our setup decodes those into Text. Any
              // remaining variants drop through.
        }
        buf.clear();
    }

    Ok(out)
}

/// Append `s` as canonical XML *character data* to `out`.
///
/// Canonical XML 1.0 §Processing Model: in text content `&`,
/// `<`, and `>` are escaped, and a carriage return (#xD) is
/// emitted as the character reference `&#xD;` so signed bytes
/// survive line-ending normalization. Tab (#x9) and line-feed
/// (#xA) are left literal in text.
fn escape_text_into(out: &mut Vec<u8>, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '\r' => out.extend_from_slice(b"&#xD;"),
            _ => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
}

/// Append `s` as a canonical XML *attribute value* to `out`.
///
/// Canonical XML 1.0 §Processing Model: attribute values escape
/// `&`, `<`, and `"`; `>` is left literal. The whitespace
/// characters #x9 (tab), #xA (LF), and #xD (CR) are emitted as
/// character references so attribute-value normalization cannot
/// collapse them after signing.
fn escape_attr_into(out: &mut Vec<u8>, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '"' => out.extend_from_slice(b"&quot;"),
            '\t' => out.extend_from_slice(b"&#x9;"),
            '\n' => out.extend_from_slice(b"&#xA;"),
            '\r' => out.extend_from_slice(b"&#xD;"),
            _ => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
}

/// Per-element data computed from the raw start tag — used by
/// `emit_start` to produce a deterministic output.
struct ElementFrame {
    /// New namespace declarations to emit (lexicographic by
    /// prefix). Default-ns prefix is "".
    namespaces: Vec<(String, String)>,
    /// Attributes (lexicographic by qualified name) — already
    /// stripped of `xmlns:*` and `xmlns`.
    attributes: Vec<(String, String)>,
    /// Newly declared namespaces from this element (for the
    /// stack).
    declared: Vec<NsDeclLocal>,
}

#[derive(Debug, Clone)]
struct NsDeclLocal {
    prefix: String,
    uri: String,
}

fn build_frame(
    e: &BytesStart<'_>,
    ns_stack: &[Vec<NsDeclLocal>],
) -> Result<ElementFrame, SamlError> {
    let elem_name = std::str::from_utf8(e.name().as_ref())
        .map_err(|err| SamlError::Parse(format!("c14n elem: {err}")))?
        .to_string();
    let elem_prefix = elem_name.split(':').next().unwrap_or("").to_string();
    let elem_local_prefix = if elem_name.contains(':') {
        elem_prefix
    } else {
        String::new()
    };

    let mut declared: Vec<NsDeclLocal> = Vec::new();
    let mut attrs_sorted: BTreeMap<String, String> = BTreeMap::new();
    let mut used_prefixes = std::collections::BTreeSet::new();
    used_prefixes.insert(elem_local_prefix);

    for a in e.attributes() {
        let a: Attribute = a.map_err(|err| SamlError::Parse(format!("c14n attr: {err}")))?;
        let key = std::str::from_utf8(a.key.as_ref())
            .map_err(|err| SamlError::Parse(format!("c14n key: {err}")))?
            .to_string();
        let val = a
            .unescape_value()
            .map_err(|err| SamlError::Parse(format!("c14n val: {err}")))?
            .into_owned();
        if key == "xmlns" {
            declared.push(NsDeclLocal {
                prefix: String::new(),
                uri: val,
            });
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            declared.push(NsDeclLocal {
                prefix: prefix.to_string(),
                uri: val,
            });
        } else {
            // Track attribute prefix as "used".
            if let Some((p, _)) = key.split_once(':') {
                used_prefixes.insert(p.to_string());
            }
            attrs_sorted.insert(key, val);
        }
    }

    // Determine which declared namespaces are *visibly utilized*
    // by this element. Exclusive c14n: only those used by the
    // element's own prefix or attribute prefixes are emitted —
    // and only if they are not already in effect from a parent
    // frame.
    let mut namespaces_out: Vec<(String, String)> = Vec::new();
    for d in &declared {
        if !used_prefixes.contains(&d.prefix) {
            continue;
        }
        // Inherited?
        let inherited = ns_stack.iter().rev().find_map(|frame| {
            frame
                .iter()
                .find(|p| p.prefix == d.prefix)
                .map(|p| p.uri.clone())
        });
        if inherited.as_deref() == Some(d.uri.as_str()) {
            continue;
        }
        namespaces_out.push((d.prefix.clone(), d.uri.clone()));
    }
    // Also: if the element's prefix is in use but no declaration
    // for it exists on this element AND no parent declared it,
    // that's malformed input — pass through; quick-xml would
    // already have errored. We honor whatever parent declared.
    namespaces_out.sort_by(|a, b| a.0.cmp(&b.0));

    let attributes: Vec<(String, String)> = attrs_sorted.into_iter().collect();

    Ok(ElementFrame {
        namespaces: namespaces_out,
        attributes,
        declared,
    })
}

fn emit_start(
    out: &mut Vec<u8>,
    e: &BytesStart<'_>,
    frame: &ElementFrame,
) -> Result<(), SamlError> {
    let name_bytes = e.name().as_ref().to_vec();
    let name = std::str::from_utf8(&name_bytes)
        .map_err(|err| SamlError::Parse(format!("c14n name: {err}")))?
        .to_string();
    out.push(b'<');
    out.extend_from_slice(name.as_bytes());
    // Emit namespaces (default first if present, then by prefix).
    let mut ns_default: Option<&(String, String)> = None;
    let mut ns_prefixed: Vec<&(String, String)> = Vec::new();
    for n in &frame.namespaces {
        if n.0.is_empty() {
            ns_default = Some(n);
        } else {
            ns_prefixed.push(n);
        }
    }
    if let Some((_, uri)) = ns_default {
        out.extend_from_slice(b" xmlns=\"");
        escape_attr_into(out, uri);
        out.push(b'"');
    }
    for (p, uri) in &ns_prefixed {
        out.extend_from_slice(b" xmlns:");
        out.extend_from_slice(p.as_bytes());
        out.extend_from_slice(b"=\"");
        escape_attr_into(out, uri);
        out.push(b'"');
    }
    for (k, v) in &frame.attributes {
        out.push(b' ');
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b"=\"");
        escape_attr_into(out, v);
        out.push(b'"');
    }
    out.push(b'>');
    Ok(())
}

fn emit_end(out: &mut Vec<u8>, e: &BytesEnd<'_>) -> Result<(), SamlError> {
    let name_bytes = e.name().as_ref().to_vec();
    let name = std::str::from_utf8(&name_bytes)
        .map_err(|err| SamlError::Parse(format!("c14n end name: {err}")))?;
    out.extend_from_slice(b"</");
    out.extend_from_slice(name.as_bytes());
    out.push(b'>');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_xml_declaration() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><a/>"#;
        let out = exc_c14n(xml).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains("<?xml"), "got: {s}");
    }

    #[test]
    fn expands_self_closing() {
        let out = exc_c14n(b"<a/>").unwrap();
        assert_eq!(out, b"<a></a>");
    }

    #[test]
    fn sorts_attributes_lexicographically() {
        let out = exc_c14n(br#"<a c="3" a="1" b="2"></a>"#).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        let i_a = s.find("a=").unwrap();
        let i_b = s.find("b=").unwrap();
        let i_c = s.find("c=").unwrap();
        assert!(i_a < i_b && i_b < i_c, "order wrong: {s}");
    }

    #[test]
    fn drops_comments() {
        let out = exc_c14n(b"<a><!-- skip --><b/></a>").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains("skip"));
        assert!(s.contains("<b></b>"));
    }

    #[test]
    fn preserves_text() {
        let out = exc_c14n(b"<a>hello world</a>").unwrap();
        assert_eq!(out, b"<a>hello world</a>");
    }

    #[test]
    fn idempotent_on_canonical_input() {
        let input = br#"<a xmlns="urn:x" attr="v"><b>text</b></a>"#;
        let once = exc_c14n(input).unwrap();
        let twice = exc_c14n(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn keeps_visibly_utilized_namespaces() {
        // The root declares two namespaces; only `samlp:` is
        // used on the element, so only `samlp:` survives.
        let xml = br#"<samlp:Req xmlns:samlp="urn:p" xmlns:other="urn:o"/>"#;
        let out = exc_c14n(xml).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("xmlns:samlp"));
        assert!(!s.contains("xmlns:other"), "got: {s}");
    }

    #[test]
    fn child_inherits_parent_namespace() {
        // Parent declares samlp:; child uses samlp: but should
        // not re-emit the xmlns declaration (exclusive rule).
        let xml = br#"<samlp:Root xmlns:samlp="urn:p"><samlp:Child/></samlp:Root>"#;
        let out = exc_c14n(xml).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        let xmlns_count = s.matches("xmlns:samlp").count();
        assert_eq!(xmlns_count, 1, "got: {s}");
    }

    #[test]
    fn child_redeclaration_with_different_uri_kept() {
        // Parent says samlp = urn:p1, child shadows with urn:p2.
        let xml =
            br#"<samlp:Root xmlns:samlp="urn:p1"><samlp:Child xmlns:samlp="urn:p2"/></samlp:Root>"#;
        let out = exc_c14n(xml).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("urn:p2"));
    }

    #[test]
    fn nested_round_trip_with_saml_shape() {
        let xml = br#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_abc" Version="2.0" IssueInstant="2026-05-13T10:00:00Z"><saml:Issuer>sp</saml:Issuer></samlp:AuthnRequest>"#;
        let out = exc_c14n(xml).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        // attributes lex-sorted: ID, IssueInstant, Version
        let i_id = s.find("ID=").unwrap();
        let i_iss = s.find("IssueInstant=").unwrap();
        let i_ver = s.find("Version=").unwrap();
        assert!(i_id < i_iss && i_iss < i_ver, "{s}");
        // Inner element keeps content.
        assert!(s.contains("<saml:Issuer>sp</saml:Issuer>"));
    }

    #[test]
    fn sign_and_verify_with_canonicalize_fn_round_trips() {
        use crate::saml::signature::{SignedDocument, sign_rsa_sha256, verify_signature};
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD as B64;

        // Same test key as signature.rs.
        const KEY_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
        const PUB_B64: &str = "MIIBCgKCAQEAj3oMzGZftYx1TE9D1uvCv6nCGKmwZJDuqKFVJsbgyxatDSEJrrXkN9ETNOrcW86SXbQTv7wMTn3FAUPo5isIsRwtCVHFuo4sRX0zsBSiHXgcsyETaItYv2W64YZJyymd1NiEM+FcK6Mimy9phYOXTwwNaI9XCRuEDg1i6m3I54EZhMXTPK3pqvnOhxoiNk0tGpXYN/8BgSOQviMjnp4lY82hTf2dxgqqvbguyI/LKQ6BgWT6bJB/HoRS2+kbKMb1ViSiZgey0jZGm1l1Sw/mvqKlTvaMzJVdUeqmWeCFFe/pTUuAUKeFZWt0SCmo449mCsxiJfVFBMh/wN0jpZKq6wIDAQAB";
        let key = B64.decode(KEY_B64).unwrap();
        let pubk = B64.decode(PUB_B64).unwrap();
        let doc_xml = br#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_a" Version="2.0"><saml:Issuer>i</saml:Issuer></saml:Assertion>"#;
        let doc = SignedDocument {
            xml: doc_xml,
            canonicalize_fn: Some(exc_c14n),
        };
        let sig = sign_rsa_sha256(&doc, &key).unwrap();
        verify_signature(&doc, &sig, &pubk).unwrap();
    }

    #[test]
    fn sign_with_c14n_rejects_unequal_byte_form_of_same_doc() {
        // Two byte-different but c14n-equivalent inputs MUST
        // verify against the same signature.
        use crate::saml::signature::{SignedDocument, sign_rsa_sha256, verify_signature};
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD as B64;
        const KEY_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
        const PUB_B64: &str = "MIIBCgKCAQEAj3oMzGZftYx1TE9D1uvCv6nCGKmwZJDuqKFVJsbgyxatDSEJrrXkN9ETNOrcW86SXbQTv7wMTn3FAUPo5isIsRwtCVHFuo4sRX0zsBSiHXgcsyETaItYv2W64YZJyymd1NiEM+FcK6Mimy9phYOXTwwNaI9XCRuEDg1i6m3I54EZhMXTPK3pqvnOhxoiNk0tGpXYN/8BgSOQviMjnp4lY82hTf2dxgqqvbguyI/LKQ6BgWT6bJB/HoRS2+kbKMb1ViSiZgey0jZGm1l1Sw/mvqKlTvaMzJVdUeqmWeCFFe/pTUuAUKeFZWt0SCmo449mCsxiJfVFBMh/wN0jpZKq6wIDAQAB";
        let key = B64.decode(KEY_B64).unwrap();
        let pubk = B64.decode(PUB_B64).unwrap();

        // Form A: attributes in one order, self-closing inner.
        let xml_a = br#"<saml:R xmlns:saml="urn:s" b="2" a="1"><saml:Inner/></saml:R>"#;
        // Form B: same logical doc, attributes in different
        // order, expanded inner. After c14n both reduce to the
        // same byte sequence.
        let xml_b = br#"<saml:R xmlns:saml="urn:s" a="1" b="2"><saml:Inner></saml:Inner></saml:R>"#;

        let c_a = exc_c14n(xml_a).unwrap();
        let c_b = exc_c14n(xml_b).unwrap();
        assert_eq!(
            c_a,
            c_b,
            "c14n forms diverged: {} vs {}",
            std::str::from_utf8(&c_a).unwrap(),
            std::str::from_utf8(&c_b).unwrap()
        );

        let sig = sign_rsa_sha256(
            &SignedDocument {
                xml: xml_a,
                canonicalize_fn: Some(exc_c14n),
            },
            &key,
        )
        .unwrap();
        verify_signature(
            &SignedDocument {
                xml: xml_b,
                canonicalize_fn: Some(exc_c14n),
            },
            &sig,
            &pubk,
        )
        .unwrap();
    }

    #[test]
    fn rejects_garbage_input() {
        assert!(exc_c14n(b"<not xml>").is_err());
    }

    #[test]
    fn unicode_text_round_trips() {
        let out = exc_c14n("<a>héllo wörld 你好</a>".as_bytes()).unwrap();
        assert_eq!(out, "<a>héllo wörld 你好</a>".as_bytes());
    }

    // ── Cycle 1: W3C Canonical XML escaping (Canonical XML 1.0
    // §Processing Model, inherited by exc-c14n). ──────────────

    #[test]
    fn text_escapes_amp_lt_gt() {
        // In character data, `&`, `<`, and `>` are escaped; `>`
        // is escaped even though XML only *requires* it after
        // `]]`. (Canonical XML always escapes it.)
        let out = exc_c14n(b"<a>1 &lt; 2 &amp; 3 &gt; 0</a>").unwrap();
        assert_eq!(out, b"<a>1 &lt; 2 &amp; 3 &gt; 0</a>");
    }

    #[test]
    fn text_carriage_return_becomes_char_ref() {
        // A literal CR (#xD) in text content is emitted as
        // `&#xD;` so the signed bytes survive line-ending
        // mangling. Tab and LF in text are NOT escaped.
        let out = exc_c14n(b"<a>x\r\n\ty</a>").unwrap();
        assert_eq!(out, "<a>x&#xD;\n\ty</a>".as_bytes());
    }

    #[test]
    fn attr_value_escapes_quote_amp_lt_but_not_gt() {
        // Attribute values escape `"`, `&`, `<`; `>` is left
        // literal in attribute values (Canonical XML §2.3).
        let out = exc_c14n(br#"<a x="&quot;&amp;&lt;>" />"#).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s, r#"<a x="&quot;&amp;&lt;>"></a>"#, "got: {s}");
    }

    #[test]
    fn attr_value_whitespace_chars_become_char_refs() {
        // #x9 (tab), #xA (LF), #xD (CR) inside an attribute value
        // are emitted as character references — the canonical
        // form prevents whitespace-collapsing attribute-value
        // normalization from changing the signed bytes.
        let out = exc_c14n("<a x=\"p\tq\nr\rs\" />".as_bytes()).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s, "<a x=\"p&#x9;q&#xA;r&#xD;s\"></a>", "got: {s}");
    }
}
