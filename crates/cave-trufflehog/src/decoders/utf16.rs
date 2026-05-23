// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! UTF-16 LE/BE decoder. Mirrors `pkg/decoders/utf16.go` — detects BOM or
//! repeated zero-byte heuristic, decodes both endiannesses and returns the
//! UTF-8 transcription.

use super::{DecodedChunk, Decoder};

pub struct Utf16Decoder;

impl Decoder for Utf16Decoder {
    fn name(&self) -> &'static str {
        "utf16"
    }

    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk> {
        if input.len() < 4 {
            return Vec::new();
        }
        let mut out = Vec::new();
        // BOM-driven path
        if input.starts_with(&[0xff, 0xfe]) {
            if let Some(s) = decode_le(&input[2..]) {
                out.push(DecodedChunk {
                    decoder: "utf16",
                    payload: s.into_bytes(),
                });
            }
        } else if input.starts_with(&[0xfe, 0xff]) {
            if let Some(s) = decode_be(&input[2..]) {
                out.push(DecodedChunk {
                    decoder: "utf16",
                    payload: s.into_bytes(),
                });
            }
        } else if looks_like_utf16le(input) {
            if let Some(s) = decode_le(input) {
                out.push(DecodedChunk {
                    decoder: "utf16",
                    payload: s.into_bytes(),
                });
            }
        } else if looks_like_utf16be(input) {
            if let Some(s) = decode_be(input) {
                out.push(DecodedChunk {
                    decoder: "utf16",
                    payload: s.into_bytes(),
                });
            }
        }
        out
    }
}

fn decode_le(b: &[u8]) -> Option<String> {
    let words: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let s: String = char::decode_utf16(words)
        .filter_map(|c| c.ok())
        .filter(|c| !c.is_control() || c == &'\n' || c == &'\t' || c == &'\r')
        .collect();
    if s.is_empty() { None } else { Some(s) }
}

fn decode_be(b: &[u8]) -> Option<String> {
    let words: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    let s: String = char::decode_utf16(words)
        .filter_map(|c| c.ok())
        .filter(|c| !c.is_control() || c == &'\n' || c == &'\t' || c == &'\r')
        .collect();
    if s.is_empty() { None } else { Some(s) }
}

fn looks_like_utf16le(b: &[u8]) -> bool {
    // Every other byte is 0 — ASCII-in-UTF-16-LE heuristic.
    if b.len() < 4 || b.len() % 2 != 0 {
        return false;
    }
    let half = b.len() / 2;
    let zeros = (0..half).filter(|i| b[i * 2 + 1] == 0).count();
    zeros * 2 >= half // >= 50% odd-byte zeros
}

fn looks_like_utf16be(b: &[u8]) -> bool {
    if b.len() < 4 || b.len() % 2 != 0 {
        return false;
    }
    let half = b.len() / 2;
    let zeros = (0..half).filter(|i| b[i * 2] == 0).count();
    zeros * 2 >= half
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_le_decodes() {
        let mut buf = vec![0xff, 0xfe];
        for c in b"hi" {
            buf.push(*c);
            buf.push(0);
        }
        let r = Utf16Decoder.from_chunk(&buf);
        assert_eq!(r[0].payload, b"hi");
    }

    #[test]
    fn bom_be_decodes() {
        let mut buf = vec![0xfe, 0xff];
        for c in b"hi" {
            buf.push(0);
            buf.push(*c);
        }
        let r = Utf16Decoder.from_chunk(&buf);
        assert_eq!(r[0].payload, b"hi");
    }

    #[test]
    fn no_bom_le_heuristic_works() {
        let mut buf = Vec::new();
        for c in b"sk_live_xyz" {
            buf.push(*c);
            buf.push(0);
        }
        let r = Utf16Decoder.from_chunk(&buf);
        assert!(!r.is_empty());
        assert_eq!(r[0].payload, b"sk_live_xyz");
    }

    #[test]
    fn no_bom_be_heuristic_works() {
        let mut buf = Vec::new();
        for c in b"AKIAIOSFODNN7EXAMPLE" {
            buf.push(0);
            buf.push(*c);
        }
        let r = Utf16Decoder.from_chunk(&buf);
        assert!(!r.is_empty());
        assert_eq!(r[0].payload, b"AKIAIOSFODNN7EXAMPLE");
    }

    #[test]
    fn pure_ascii_is_not_decoded() {
        let r = Utf16Decoder.from_chunk(b"plain ascii content");
        assert!(r.is_empty());
    }

    #[test]
    fn short_input_skipped() {
        assert!(Utf16Decoder.from_chunk(b"ab").is_empty());
    }
}
