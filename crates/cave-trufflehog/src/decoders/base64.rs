// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Base64 decoder — port of `pkg/decoders/base64.go`. Finds candidate
//! base64 blobs in the input (standard + URL-safe alphabets), decodes
//! each, and returns the inner bytes for re-scanning.

use super::{DecodedChunk, Decoder};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use regex::Regex;
use std::sync::OnceLock;

pub struct Base64Decoder;

static B64_RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    // Length threshold mirrors upstream's minLen=20 — short tokens are
    // discarded to keep noise below the 1% false-positive budget.
    B64_RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/_\-]{20,}={0,2}").unwrap())
}

impl Decoder for Base64Decoder {
    fn name(&self) -> &'static str {
        "base64"
    }

    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk> {
        let Ok(s) = std::str::from_utf8(input) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for m in re().find_iter(s) {
            let cand = m.as_str();
            for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
                if let Ok(bytes) = engine.decode(cand)
                    && !bytes.is_empty()
                    && is_mostly_printable(&bytes)
                {
                    out.push(DecodedChunk {
                        decoder: "base64",
                        payload: bytes,
                    });
                    break;
                }
            }
        }
        out
    }
}

fn is_mostly_printable(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let printable = bytes
        .iter()
        .filter(|b| (0x20u8..=0x7eu8).contains(b) || matches!(**b, 9 | 10 | 13))
        .count();
    (printable * 100) / bytes.len() >= 90
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_alphabet_round_trip() {
        let payload = "github_pat_11ABCDEFG_aBc".to_string();
        let enc = STANDARD.encode(payload.as_bytes());
        let d = Base64Decoder;
        let out = d.from_chunk(enc.as_bytes());
        assert!(out.iter().any(|c| c.payload == payload.as_bytes()));
    }

    #[test]
    fn url_safe_alphabet_decodes() {
        let payload = "this-is-a-long-secret-payload-token-1234567890";
        let enc = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let d = Base64Decoder;
        let out = d.from_chunk(enc.as_bytes());
        assert!(out.iter().any(|c| c.payload == payload.as_bytes()));
    }

    #[test]
    fn short_candidates_are_rejected() {
        // Below 20 chars
        let d = Base64Decoder;
        assert!(d.from_chunk(b"YWI=").is_empty());
    }

    #[test]
    fn non_utf8_input_yields_nothing() {
        let d = Base64Decoder;
        assert!(d.from_chunk(&[0xff, 0xfe, 0xfd]).is_empty());
    }

    #[test]
    fn mostly_binary_payload_skipped() {
        // Encode random binary — printable filter rejects it.
        let bin: Vec<u8> = (0..40u8).collect();
        let enc = STANDARD.encode(&bin);
        let d = Base64Decoder;
        let out = d.from_chunk(enc.as_bytes());
        for c in out {
            assert!(is_mostly_printable(&c.payload), "non-printable leaked");
        }
    }
}
