// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Base64 encoding/decoding helpers for etcd v3 API compatibility.
//!
//! etcd v3 JSON API requires keys and values to be base64-encoded
//! because they are arbitrary byte arrays (not necessarily valid UTF-8).

use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Encode bytes to base64 string.
pub fn encode(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Decode base64 string to bytes. Returns empty vec on invalid input.
pub fn decode(s: &str) -> Vec<u8> {
    STANDARD.decode(s).unwrap_or_else(|_| {
        // Fallback: treat as plain text (for backward compat)
        s.as_bytes().to_vec()
    })
}

/// Decode base64 string, returning None for empty/missing input.
pub fn decode_opt(s: &Option<String>) -> Option<Vec<u8>> {
    s.as_ref().map(|s| decode(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let data = b"hello world";
        let encoded = encode(data);
        assert_eq!(encoded, "aGVsbG8gd29ybGQ=");
        let decoded = decode(&encoded);
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_binary_data() {
        let data = vec![0x00, 0xFF, 0x42, 0x13];
        let encoded = encode(&data);
        let decoded = decode(&encoded);
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_empty() {
        assert_eq!(encode(b""), "");
        assert_eq!(decode(""), b"");
    }

    #[test]
    fn test_plain_text_fallback() {
        // Invalid base64 falls back to treating as plain text
        let result = decode("not-valid-base64!!!");
        assert_eq!(result, b"not-valid-base64!!!");
    }

    #[test]
    fn test_etcd_compat_key() {
        // etcdctl put foo bar -> key="Zm9v" value="YmFy"
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"bar"), "YmFy");
        assert_eq!(decode("Zm9v"), b"foo");
        assert_eq!(decode("YmFy"), b"bar");
    }

    #[test]
    fn test_prefix_range_end() {
        // etcd prefix query: key="/a/" range_end="/a0"
        // base64("/a/") = "L2Ev", base64("/a0") = "L2Ew"
        assert_eq!(encode(b"/a/"), "L2Ev");
        assert_eq!(encode(b"/a0"), "L2Ew");
    }
}
