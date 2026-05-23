// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTML entity decoder. Mirrors `pkg/decoders/html.go` — recognises the
//! common named entities + numeric forms (`&#NN;`, `&#xHH;`). Secrets
//! embedded in HTML pages, error responses, and rendered docs become
//! detectable after this pass.

use super::{DecodedChunk, Decoder};

pub struct HtmlDecoder;

const NAMED: &[(&str, char)] = &[
    ("&amp;", '&'),
    ("&lt;", '<'),
    ("&gt;", '>'),
    ("&quot;", '"'),
    ("&apos;", '\''),
    ("&nbsp;", '\u{00A0}'),
    ("&copy;", '©'),
    ("&reg;", '®'),
    ("&trade;", '™'),
];

impl Decoder for HtmlDecoder {
    fn name(&self) -> &'static str {
        "html"
    }

    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk> {
        let Ok(s) = std::str::from_utf8(input) else {
            return Vec::new();
        };
        if !s.contains('&') {
            return Vec::new();
        }
        let mut out = decode_named(s);
        out = decode_numeric(&out);
        if out == s {
            Vec::new()
        } else {
            vec![DecodedChunk {
                decoder: "html",
                payload: out.into_bytes(),
            }]
        }
    }
}

fn decode_named(s: &str) -> String {
    let mut out = s.to_string();
    for (k, v) in NAMED {
        if out.contains(k) {
            out = out.replace(k, &v.to_string());
        }
    }
    out
}

fn decode_numeric(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 2 < bytes.len() && bytes[i + 1] == b'#' {
            // &#x...; or &#NN;
            let hex = bytes[i + 2] == b'x' || bytes[i + 2] == b'X';
            let start = if hex { i + 3 } else { i + 2 };
            let mut end = start;
            while end < bytes.len() && bytes[end] != b';' && end - start < 8 {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b';' {
                let token = &input[start..end];
                let cp = if hex {
                    u32::from_str_radix(token, 16).ok()
                } else {
                    token.parse::<u32>().ok()
                };
                if let Some(cp) = cp
                    && let Some(c) = char::from_u32(cp)
                {
                    out.push(c);
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_entities_decoded() {
        let r = HtmlDecoder.from_chunk(b"a &amp; b &lt; c");
        assert_eq!(r[0].payload, b"a & b < c");
    }

    #[test]
    fn numeric_decimal_decoded() {
        let r = HtmlDecoder.from_chunk(b"&#65;&#66;");
        assert_eq!(r[0].payload, b"AB");
    }

    #[test]
    fn numeric_hex_decoded() {
        let r = HtmlDecoder.from_chunk(b"&#x41;&#x42;");
        assert_eq!(r[0].payload, b"AB");
    }

    #[test]
    fn plain_text_skipped() {
        assert!(HtmlDecoder.from_chunk(b"plain text").is_empty());
    }

    #[test]
    fn malformed_entity_passes_through() {
        let r = HtmlDecoder.from_chunk(b"& notanentity");
        assert!(r.is_empty());
    }
}
