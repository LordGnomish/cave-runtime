// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Encoded-payload decoders — base64 and gzip.
//!
//! TruffleHog's `pkg/decoders/` walks each line/blob and re-scans whatever
//! decoded bytes fall out of base64/hex/gzip envelopes. cave-secrets ports the
//! same idea but constrained to the two encodings that actually hide secrets
//! in source repos today: base64 and gzip.

use crate::detector::{scan, Finding, SecretDetector};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use flate2::read::GzDecoder;
use std::io::Read;

const MIN_BASE64_LEN: usize = 24;
const MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Attempt to decode `s` as standard base64. Returns `None` if the input is
/// too short, not valid base64, or the decoded bytes are not UTF-8.
pub fn decode_base64_to_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.len() < MIN_BASE64_LEN {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    {
        return None;
    }
    let bytes = B64.decode(trimmed).ok()?;
    if bytes.len() > MAX_DECODED_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Decompress gzip-framed bytes back into a `Vec<u8>`. Returns `None` if the
/// header is not gzip-magic or decompression fails. Caps output at 4 MiB to
/// keep mis-identified inputs from blowing memory.
pub fn decode_gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 2 || bytes[..2] != GZIP_MAGIC {
        return None;
    }
    let mut decoder = GzDecoder::new(bytes);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while out.len() <= MAX_DECODED_BYTES {
        match decoder.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => return None,
        }
    }
    None
}

/// Scan `content` directly *and* attempt to decode embedded base64 blobs on a
/// per-token basis. Findings from decoded content carry the original line
/// number but the synthetic filename suffix `:base64`.
pub fn scan_with_base64_decoder(
    content: &str,
    filename: &str,
    detectors: &[SecretDetector],
) -> Vec<Finding> {
    let mut findings = scan(content, filename, detectors);
    for (line_idx, line) in content.lines().enumerate() {
        for token in tokenize_for_base64(line) {
            if let Some(decoded) = decode_base64_to_string(token) {
                let virt = format!("{}:base64", filename);
                let mut sub = scan(&decoded, &virt, detectors);
                for f in &mut sub {
                    f.line = line_idx + 1;
                }
                findings.extend(sub);
            }
        }
    }
    findings
}

/// Decompress `bytes` as gzip and scan the resulting payload. If decompression
/// fails, returns an empty vector.
pub fn scan_gzip_blob(bytes: &[u8], filename: &str, detectors: &[SecretDetector]) -> Vec<Finding> {
    let Some(decoded) = decode_gzip(bytes) else {
        return Vec::new();
    };
    let Ok(text) = String::from_utf8(decoded) else {
        return Vec::new();
    };
    let virt = format!("{}:gzip", filename);
    scan(&text, &virt, detectors)
}

fn tokenize_for_base64(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in line.char_indices() {
        let ok = c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=';
        match (ok, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let end = i;
                if end - s >= MIN_BASE64_LEN {
                    out.push(&line[s..end]);
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if line.len() - s >= MIN_BASE64_LEN {
            out.push(&line[s..]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::builtin_detectors;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    #[test]
    fn base64_round_trip() {
        let original = "the quick brown fox jumps over the lazy dog";
        let encoded = B64.encode(original.as_bytes());
        let decoded = decode_base64_to_string(&encoded).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn base64_too_short_rejected() {
        let encoded = B64.encode(b"short");
        assert!(decode_base64_to_string(&encoded).is_none());
    }

    #[test]
    fn base64_non_alphabet_rejected() {
        assert!(decode_base64_to_string("this string has spaces!!!!!!!!!").is_none());
    }

    #[test]
    fn base64_non_utf8_rejected() {
        let encoded = B64.encode([0xff_u8, 0xfe, 0xfd, 0xfc, 0xfb, 0xfa, 0xf9, 0xf8, 0xf7, 0xf6, 0xf5, 0xf4, 0xf3, 0xf2, 0xf1, 0xf0, 0xef, 0xee, 0xed, 0xec, 0xeb, 0xea, 0xe9, 0xe8]);
        assert!(decode_base64_to_string(&encoded).is_none());
    }

    #[test]
    fn gzip_round_trip() {
        let payload = b"hello cave-secrets";
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(payload).unwrap();
        let zipped = enc.finish().unwrap();
        let decoded = decode_gzip(&zipped).expect("decode");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn gzip_rejects_non_gzip() {
        assert!(decode_gzip(b"not gzip").is_none());
    }

    #[test]
    fn gzip_rejects_truncated_magic() {
        assert!(decode_gzip(&[0x1f]).is_none());
    }

    #[test]
    fn base64_decoder_finds_secret_in_encoded_blob() {
        let secret_payload = "AWS_KEY=AKIAIOSFODNN7EXAMPLE";
        let encoded = B64.encode(secret_payload.as_bytes());
        let content = format!("data: {}", encoded);
        let det = builtin_detectors();
        let findings = scan_with_base64_decoder(&content, "cfg.env", &det);
        assert!(
            findings.iter().any(|f| f.detector == "aws-access-key"),
            "expected AWS key to surface after base64 decode"
        );
    }

    #[test]
    fn gzip_decoder_finds_secret_in_gzip_blob() {
        let payload = "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n";
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(payload.as_bytes()).unwrap();
        let bytes = enc.finish().unwrap();
        let det = builtin_detectors();
        let findings = scan_gzip_blob(&bytes, "blob.gz", &det);
        assert!(
            findings.iter().any(|f| f.detector == "aws-access-key"),
            "expected AWS key in gzip-encoded blob"
        );
    }

    #[test]
    fn tokenize_finds_substrings_above_min_len() {
        let line = "prefix VGhpcyBpcyBhIGxvbmctZW5vdWdoIHN0cmluZw== suffix";
        let toks = tokenize_for_base64(line);
        assert!(!toks.is_empty());
        let any_decodes = toks.iter().any(|t| decode_base64_to_string(t).is_some());
        assert!(any_decodes);
    }

    #[test]
    fn empty_line_yields_no_tokens() {
        assert!(tokenize_for_base64("").is_empty());
    }

    #[test]
    fn very_short_segment_skipped() {
        let toks = tokenize_for_base64("hi");
        assert!(toks.is_empty());
    }
}
