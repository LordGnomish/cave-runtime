// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pure-Rust GGUF header + metadata reader.
//!
//! Cite ollama/ollama `fs/ggml/ggml.go` (magic constants) and `fs/ggml/gguf.go`
//! (decode loop). This reader parses the GGUF container header — magic, version,
//! tensor count, KV count — and the key/value metadata block into a typed map.
//! It deliberately does **not** read tensor data or run inference (that is the
//! llama.cpp runtime, an explicit scope-cut delegated to `cave-llm-gateway`); it
//! gives the registry/daemon a dependency-free way to introspect a `.gguf`
//! file's architecture, context length, and quantization.
//!
//! Only little-endian GGUF v2/v3 (the format every modern Ollama model ships)
//! is supported; v1 and big-endian files return a clear error.

#[cfg(test)]
mod tests {
    use super::*;

    // GGUF metadata value type ids (cite fs/ggml/gguf.go).
    const T_UINT32: u32 = 4;
    const T_STRING: u32 = 8;
    const T_ARRAY: u32 = 9;

    fn put_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    fn header(version: u32, tensor_count: u64, kv_count: u64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&tensor_count.to_le_bytes());
        b.extend_from_slice(&kv_count.to_le_bytes());
        b
    }

    #[test]
    fn parses_minimal_header() {
        let buf = header(3, 0, 0);
        let g = GgufFile::parse(&buf).expect("parse ok");
        assert_eq!(g.version, 3);
        assert_eq!(g.tensor_count, 0);
        assert!(g.metadata.is_empty());
    }

    #[test]
    fn parses_string_kv_and_architecture_helper() {
        let mut buf = header(3, 0, 1);
        put_str(&mut buf, "general.architecture");
        buf.extend_from_slice(&T_STRING.to_le_bytes());
        put_str(&mut buf, "llama");

        let g = GgufFile::parse(&buf).expect("parse ok");
        assert_eq!(
            g.get("general.architecture"),
            Some(&MetaValue::String("llama".to_string()))
        );
        assert_eq!(g.architecture(), Some("llama"));
    }

    #[test]
    fn parses_u32_kv() {
        let mut buf = header(3, 0, 1);
        put_str(&mut buf, "llama.context_length");
        buf.extend_from_slice(&T_UINT32.to_le_bytes());
        buf.extend_from_slice(&4096u32.to_le_bytes());

        let g = GgufFile::parse(&buf).expect("parse ok");
        assert_eq!(g.get("llama.context_length"), Some(&MetaValue::U32(4096)));
    }

    #[test]
    fn parses_string_array_kv() {
        let mut buf = header(3, 0, 1);
        put_str(&mut buf, "tokenizer.ggml.tokens");
        buf.extend_from_slice(&T_ARRAY.to_le_bytes());
        buf.extend_from_slice(&T_STRING.to_le_bytes()); // element type
        buf.extend_from_slice(&2u64.to_le_bytes()); // count
        put_str(&mut buf, "<s>");
        put_str(&mut buf, "</s>");

        let g = GgufFile::parse(&buf).expect("parse ok");
        match g.get("tokenizer.ggml.tokens") {
            Some(MetaValue::Array(items)) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], MetaValue::String("<s>".into()));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let mut buf = vec![0u8; 24];
        buf[0] = b'N';
        let err = GgufFile::parse(&buf).unwrap_err();
        assert!(matches!(err, GgufError::BadMagic(_)));
    }

    #[test]
    fn rejects_truncated_data() {
        let buf = vec![0x47, 0x47, 0x55, 0x46]; // magic only, no version
        let err = GgufFile::parse(&buf).unwrap_err();
        assert!(matches!(err, GgufError::UnexpectedEof));
    }

    #[test]
    fn rejects_v1() {
        let buf = header(1, 0, 0);
        let err = GgufFile::parse(&buf).unwrap_err();
        assert!(matches!(err, GgufError::UnsupportedVersion(1)));
    }
}
