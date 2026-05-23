// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Decoder chains — port of `pkg/decoders/`. Each decoder takes a chunk
//! payload and returns zero or more decoded byte buffers that are then
//! re-scanned by the detector engine. Mirrors upstream's
//! `Decoders []Decoder` registration order so the workspace produces the
//! same Finding stream upstream would produce on the same input.

pub mod base64;
pub mod escaped_unicode;
pub mod html;
pub mod utf16;
pub mod utf8;

/// Output of a decoder pass — payload + the upstream name (for telemetry +
/// `Result.ExtraData["decoder"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedChunk {
    pub decoder: &'static str,
    pub payload: Vec<u8>,
}

/// Trait implemented by every decoder in the registry.
pub trait Decoder: Send + Sync {
    fn name(&self) -> &'static str;
    fn from_chunk(&self, input: &[u8]) -> Vec<DecodedChunk>;
}

/// Registry — fed by `engine::Engine::run` in the upstream `for _, dec :=
/// range decoders { … }` loop.
pub struct DecoderRegistry {
    pub decoders: Vec<Box<dyn Decoder>>,
}

impl Default for DecoderRegistry {
    fn default() -> Self {
        Self {
            decoders: vec![
                Box::new(utf8::Utf8Decoder),
                Box::new(base64::Base64Decoder),
                Box::new(utf16::Utf16Decoder),
                Box::new(escaped_unicode::EscapedUnicodeDecoder),
                Box::new(html::HtmlDecoder),
            ],
        }
    }
}

impl DecoderRegistry {
    pub fn decode_all(&self, input: &[u8]) -> Vec<DecodedChunk> {
        let mut out = Vec::new();
        for d in &self.decoders {
            out.extend(d.from_chunk(input));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_five_decoders() {
        let r = DecoderRegistry::default();
        let names: Vec<_> = r.decoders.iter().map(|d| d.name()).collect();
        for n in ["utf8", "base64", "utf16", "escaped_unicode", "html"] {
            assert!(names.contains(&n), "missing {n}");
        }
    }

    #[test]
    fn registry_decodes_to_concatenated_set() {
        let r = DecoderRegistry::default();
        let payload = b"hello \\u0041 &amp;";
        let out = r.decode_all(payload);
        let decoders: Vec<_> = out.iter().map(|c| c.decoder).collect();
        assert!(decoders.iter().any(|d| d == &"utf8"));
        assert!(decoders.iter().any(|d| d == &"escaped_unicode"));
        assert!(decoders.iter().any(|d| d == &"html"));
    }
}
