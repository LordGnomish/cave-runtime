// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! UTF-8 pass-through decoder — strips invalid bytes and re-emits a clean
//! UTF-8 buffer so detectors that rely on `String` semantics see consistent
//! input. Mirrors `pkg/decoders/utf8.go`.

use super::{DecodedChunk, Decoder};

pub struct Utf8Decoder;

impl Decoder for Utf8Decoder {
    fn name(&self) -> &'static str {
        "utf8"
    }

    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk> {
        if input.is_empty() {
            return Vec::new();
        }
        let s = String::from_utf8_lossy(input).into_owned();
        vec![DecodedChunk {
            decoder: "utf8",
            payload: s.into_bytes(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trips() {
        let d = Utf8Decoder;
        let r = d.from_chunk(b"plain");
        assert_eq!(r[0].payload, b"plain");
    }

    #[test]
    fn invalid_bytes_are_replaced() {
        let d = Utf8Decoder;
        // 0xFF is not valid UTF-8 — String::from_utf8_lossy yields U+FFFD.
        let r = d.from_chunk(&[0x68, 0xFF, 0x69]);
        let s = String::from_utf8(r[0].payload.clone()).unwrap();
        assert!(s.contains('h') && s.contains('i'));
    }

    #[test]
    fn empty_yields_nothing() {
        let d = Utf8Decoder;
        assert!(d.from_chunk(b"").is_empty());
    }

    #[test]
    fn name_is_utf8() {
        assert_eq!(Utf8Decoder.name(), "utf8");
    }
}
