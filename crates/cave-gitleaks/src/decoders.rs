// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Decoder chains — base64 + gzip auto-decode of in-line payloads.
//!
//! Mirrors `detect/decoders/` upstream (`v8.29.1`). The upstream package
//! provides ordered detectors keyed on byte-prefix recognition (base64
//! letters, gzip magic, hex). cave-gitleaks ships the two highest-signal
//! decoders — base64 (RFC 4648) and gzip — chained byte-aware so a
//! base64-wrapped gzip blob is unwrapped in two passes.
//!
//! Out-of-scope: hex decoding (low signal; many false positives), URL
//! decoding (already part of regex space), AES/CBC obfuscation chains.

use std::io::Read;

use flate2::read::GzDecoder;

use crate::detect::Detector;
use crate::finding::Finding;

/// One decoder step.
#[derive(Debug, Clone, Copy)]
pub enum Decoder {
    Base64,
    Gzip,
}

impl Decoder {
    /// Attempt to decode the input. `None` if the input doesn't look
    /// decodable into something different from `input`. Returns raw bytes
    /// so the caller can chain into the next decoder without forcing
    /// utf-8 in the middle of a base64-wrapped gzip payload.
    pub fn try_decode_bytes(&self, input: &[u8]) -> Option<Vec<u8>> {
        match self {
            Decoder::Base64 => decode_base64_runs_bytes(input),
            Decoder::Gzip => decode_gzip_bytes(input),
        }
    }

    /// String convenience wrapper — returns Some only when the decoded
    /// bytes are valid UTF-8.
    pub fn try_decode(&self, input: &str) -> Option<String> {
        let bytes = self.try_decode_bytes(input.as_bytes())?;
        String::from_utf8(bytes).ok()
    }
}

/// A chain of decoders applied breadth-first. Matches upstream
/// `Decoder.Decode(..., depth)` semantics.
#[derive(Debug, Clone)]
pub struct DecoderChain {
    pub steps: Vec<Decoder>,
}

impl DecoderChain {
    /// Default chain: base64 then gzip (gzip is usually wrapped in base64
    /// for transport, so checking base64 first is correct).
    pub fn default_chain() -> Self {
        Self {
            steps: vec![Decoder::Base64, Decoder::Gzip],
        }
    }
}

/// Scan `content` for secrets, including up to `max_depth` decoded forms.
/// At each depth, every decoder in the chain is tried against the bytes
/// from the previous round; whenever a decoded blob is valid UTF-8, the
/// detector runs against it.
pub fn detect_with_decoders(
    detector: &Detector,
    path: &str,
    content: &str,
    chain: &DecoderChain,
    max_depth: u8,
) -> Vec<Finding> {
    let mut out = detector.scan_str(path, content);

    if max_depth == 0 {
        return out;
    }

    let mut frontier: Vec<Vec<u8>> = vec![content.as_bytes().to_vec()];
    for _depth in 0..max_depth {
        let mut next_frontier = Vec::new();
        for blob in frontier.drain(..) {
            for step in &chain.steps {
                if let Some(decoded_bytes) = step.try_decode_bytes(&blob)
                    && decoded_bytes != blob
                {
                    if let Ok(decoded_str) = std::str::from_utf8(&decoded_bytes) {
                        out.extend(detector.scan_str(path, decoded_str));
                    }
                    next_frontier.push(decoded_bytes);
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }
    out
}

/// Locate base64 runs in `input` (anything that looks like RFC 4648
/// alphabet, >= 16 chars) and decode each. Concatenates the decoded
/// byte strings.
pub fn decode_base64_runs_bytes(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < input.len() {
        if is_b64_alpha(input[i]) {
            let start = i;
            while i < input.len() && (is_b64_alpha(input[i]) || input[i] == b'=') {
                i += 1;
            }
            let run = &input[start..i];
            if run.len() >= 16
                && let Some(dec) = base64_decode_bytes(run)
            {
                out.extend(dec);
            }
        } else {
            i += 1;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// String convenience wrapper around [`decode_base64_runs_bytes`].
pub fn decode_base64_runs(input: &str) -> Option<String> {
    let bytes = decode_base64_runs_bytes(input.as_bytes())?;
    String::from_utf8(bytes).ok()
}

/// Decode `input` as gzip — `None` if it isn't gzip-compressed.
pub fn decode_gzip(input: &str) -> Option<String> {
    let bytes = decode_gzip_bytes(input.as_bytes())?;
    String::from_utf8(bytes).ok()
}

/// Byte-level gzip decompression — `None` if `input` doesn't begin with
/// the gzip magic `0x1f 0x8b`.
pub fn decode_gzip_bytes(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() < 2 || !(input[0] == 0x1f && input[1] == 0x8b) {
        return None;
    }
    let mut dec = GzDecoder::new(input);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).ok()?;
    Some(out)
}

fn is_b64_alpha(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

/// Pure base64 decode (RFC 4648). Returns None on invalid input.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    base64_decode_bytes(input.as_bytes())
}

fn base64_decode_bytes(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            // Padding — stop reading.
            break;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(input: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut i = 0;
        while i + 3 <= input.len() {
            let n = ((input[i] as u32) << 16)
                | ((input[i + 1] as u32) << 8)
                | (input[i + 2] as u32);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push(TABLE[(n & 0x3f) as usize] as char);
            i += 3;
        }
        let rem = input.len() - i;
        if rem == 1 {
            let n = (input[i] as u32) << 16;
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push_str("==");
        } else if rem == 2 {
            let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        out
    }

    #[test]
    fn base64_decode_roundtrips() {
        let original = b"hello, gitleaks!";
        let encoded = b64(original);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_base64_runs_finds_payload_inside_text() {
        let secret = b"AKIAIOSFODNN7EXAMPLE";
        let encoded = b64(secret);
        let text = format!("config = {{ blob = \"{}\" }}", encoded);
        let decoded = decode_base64_runs(&text).unwrap();
        assert!(decoded.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn decode_base64_runs_skips_short_runs() {
        // "abc" is too short to be considered a payload.
        assert!(decode_base64_runs("abc").is_none());
    }

    #[test]
    fn decode_gzip_rejects_non_gzip_input() {
        assert!(decode_gzip("not gzip").is_none());
    }

    #[test]
    fn detect_with_decoders_zero_depth_is_passthrough() {
        let d = Detector::with_builtins();
        let chain = DecoderChain::default_chain();
        let plain = "AKIAIOSFODNN7EXAMPLE";
        let f = detect_with_decoders(&d, "x", plain, &chain, 0);
        assert_eq!(f.len(), 1);
    }
}
