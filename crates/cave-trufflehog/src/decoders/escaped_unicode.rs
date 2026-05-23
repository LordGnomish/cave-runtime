// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Escaped-Unicode decoder — `\uXXXX` and `\u{XXXXXX}` sequences. Port of
//! `pkg/decoders/escaped_unicode.go`. Used when source files embed secrets
//! inside JS / JSON / YAML where strings are routinely escaped.

use super::{DecodedChunk, Decoder};
use regex::Regex;
use std::sync::OnceLock;

pub struct EscapedUnicodeDecoder;

static RE_FOUR: OnceLock<Regex> = OnceLock::new();
static RE_BRACED: OnceLock<Regex> = OnceLock::new();

fn re_four() -> &'static Regex {
    RE_FOUR.get_or_init(|| Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap())
}
fn re_braced() -> &'static Regex {
    RE_BRACED.get_or_init(|| Regex::new(r"\\u\{([0-9a-fA-F]{1,6})\}").unwrap())
}

impl Decoder for EscapedUnicodeDecoder {
    fn name(&self) -> &'static str {
        "escaped_unicode"
    }

    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk> {
        let Ok(s) = std::str::from_utf8(input) else {
            return Vec::new();
        };
        if !s.contains("\\u") {
            return Vec::new();
        }
        let mut out = String::with_capacity(s.len());
        let mut idx = 0usize;
        // Combined sweep: braced first, then four-hex; falls back to literal.
        while idx < s.len() {
            if let Some(m) = re_braced().find_at(s, idx)
                && m.start() == idx
            {
                let hex = &s[m.start() + 3..m.end() - 1];
                if let Ok(cp) = u32::from_str_radix(hex, 16)
                    && let Some(c) = char::from_u32(cp)
                {
                    out.push(c);
                }
                idx = m.end();
                continue;
            }
            if let Some(m) = re_four().find_at(s, idx)
                && m.start() == idx
            {
                let hex = &s[m.start() + 2..m.end()];
                if let Ok(cp) = u32::from_str_radix(hex, 16)
                    && let Some(c) = char::from_u32(cp)
                {
                    out.push(c);
                }
                idx = m.end();
                continue;
            }
            let ch = s[idx..].chars().next().unwrap();
            out.push(ch);
            idx += ch.len_utf8();
        }
        if out == s {
            Vec::new()
        } else {
            vec![DecodedChunk {
                decoder: "escaped_unicode",
                payload: out.into_bytes(),
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_hex_decoded() {
        let r = EscapedUnicodeDecoder.from_chunk(b"hello \\u0041\\u0042");
        assert_eq!(r[0].payload, b"hello AB");
    }

    #[test]
    fn braced_decoded() {
        let r = EscapedUnicodeDecoder.from_chunk(b"\\u{1F600}");
        let s = String::from_utf8(r[0].payload.clone()).unwrap();
        assert_eq!(s, "😀");
    }

    #[test]
    fn passthrough_when_no_escape() {
        let r = EscapedUnicodeDecoder.from_chunk(b"plain text");
        assert!(r.is_empty());
    }

    #[test]
    fn mixed_passthrough_preserved() {
        let r = EscapedUnicodeDecoder.from_chunk(b"foo\\u0021bar");
        assert_eq!(r[0].payload, b"foo!bar");
    }

    #[test]
    fn invalid_codepoint_dropped() {
        let r = EscapedUnicodeDecoder.from_chunk(b"\\uD800x");
        let s = String::from_utf8(r[0].payload.clone()).unwrap();
        assert!(s.contains('x'));
        // U+D800 is a lone surrogate — char::from_u32 returns None so it's dropped.
    }
}
