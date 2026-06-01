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

use std::collections::BTreeMap;
use thiserror::Error;

/// GGUF little-endian magic — ASCII `GGUF`. Cite fs/ggml/ggml.go
/// `FILE_MAGIC_GGUF_LE = 0x46554747`.
pub const GGUF_MAGIC: u32 = 0x4655_4747;
/// Big-endian variant — recognised only to emit a clear "unsupported" error.
pub const GGUF_MAGIC_BE: u32 = 0x4747_5546;

/// Errors from [`GgufFile::parse`].
#[derive(Debug, Error)]
pub enum GgufError {
    #[error("not a GGUF file: bad magic {0:#010x}")]
    BadMagic(u32),
    #[error("big-endian GGUF is not supported")]
    BigEndianUnsupported,
    #[error("unsupported GGUF version {0} (only v2/v3 supported)")]
    UnsupportedVersion(u32),
    #[error("unexpected end of data while reading GGUF")]
    UnexpectedEof,
    #[error("invalid UTF-8 in GGUF string")]
    InvalidUtf8,
    #[error("unknown GGUF metadata value type {0}")]
    UnknownValueType(u32),
}

/// A typed GGUF metadata value. Cite fs/ggml/gguf.go value-type ids 0–12.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
    Array(Vec<MetaValue>),
}

/// One tensor descriptor from the GGUF tensor-info block. Cite fs/ggml
/// gguf.go `Tensor{Name, Kind, Offset, Shape}`. `kind` is the ggml type id
/// (see [`crate::quant`]); `offset` is relative to the aligned start of the
/// tensor-data section. This reader records the descriptors only — it never
/// loads tensor data (that is the llama.cpp runtime scope-cut).
#[derive(Debug, Clone, PartialEq)]
pub struct TensorInfo {
    pub name: String,
    pub kind: u32,
    pub offset: u64,
    pub shape: Vec<u64>,
}

impl TensorInfo {
    /// Total element count = product of the shape dimensions (1 for a scalar,
    /// matching Go's `slices` product over an empty shape).
    pub fn num_elements(&self) -> u64 {
        self.shape.iter().product()
    }
}

/// A parsed GGUF container: header fields, the metadata key/value block, and
/// the tensor-info descriptors that follow it.
#[derive(Debug, Clone)]
pub struct GgufFile {
    pub version: u32,
    pub tensor_count: u64,
    pub metadata: BTreeMap<String, MetaValue>,
    pub tensors: Vec<TensorInfo>,
}

impl GgufFile {
    /// Parse a GGUF header + metadata block from an in-memory byte slice.
    pub fn parse(bytes: &[u8]) -> Result<Self, GgufError> {
        let mut cur = Cursor::new(bytes);

        let magic = cur.read_u32()?;
        if magic == GGUF_MAGIC_BE {
            return Err(GgufError::BigEndianUnsupported);
        }
        if magic != GGUF_MAGIC {
            return Err(GgufError::BadMagic(magic));
        }

        let version = cur.read_u32()?;
        if version < 2 {
            return Err(GgufError::UnsupportedVersion(version));
        }
        if version > 3 {
            return Err(GgufError::UnsupportedVersion(version));
        }

        // v2/v3 use u64 counts.
        let tensor_count = cur.read_u64()?;
        let kv_count = cur.read_u64()?;

        let mut metadata = BTreeMap::new();
        for _ in 0..kv_count {
            let key = cur.read_string()?;
            let vtype = cur.read_u32()?;
            let value = cur.read_value(vtype)?;
            metadata.insert(key, value);
        }

        // Tensor-info block follows the KV block: tensor_count descriptors,
        // each name / n_dims(u32) / shape(n_dims × u64) / kind(u32) / offset(u64).
        let mut tensors = Vec::with_capacity(tensor_count.min(65_536) as usize);
        for _ in 0..tensor_count {
            let name = cur.read_string()?;
            let n_dims = cur.read_u32()?;
            let mut shape = Vec::with_capacity(n_dims.min(8) as usize);
            for _ in 0..n_dims {
                shape.push(cur.read_u64()?);
            }
            let kind = cur.read_u32()?;
            let offset = cur.read_u64()?;
            tensors.push(TensorInfo {
                name,
                kind,
                offset,
                shape,
            });
        }

        Ok(GgufFile {
            version,
            tensor_count,
            metadata,
            tensors,
        })
    }

    /// Borrow a metadata value by key.
    pub fn get(&self, key: &str) -> Option<&MetaValue> {
        self.metadata.get(key)
    }

    /// Convenience: `general.architecture` as a string (e.g. "llama").
    pub fn architecture(&self) -> Option<&str> {
        match self.metadata.get("general.architecture") {
            Some(MetaValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Tensor-data alignment. Cite fs/ggml gguf.go: read from
    /// `general.alignment` (any unsigned-int metadata type), defaulting to 32
    /// when absent. Used to pad the start of the tensor-data section.
    pub fn alignment(&self) -> u64 {
        match self.metadata.get("general.alignment") {
            Some(MetaValue::U8(v)) => *v as u64,
            Some(MetaValue::U16(v)) => *v as u64,
            Some(MetaValue::U32(v)) => *v as u64,
            Some(MetaValue::U64(v)) => *v,
            _ => 32,
        }
    }
}

/// Minimal little-endian byte cursor with bounds checking.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], GgufError> {
        let end = self.pos.checked_add(n).ok_or(GgufError::UnexpectedEof)?;
        let slice = self.buf.get(self.pos..end).ok_or(GgufError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, GgufError> {
        Ok(self.take(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, GgufError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn read_u32(&mut self) -> Result<u32, GgufError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64, GgufError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn read_string(&mut self) -> Result<String, GgufError> {
        let len = self.read_u64()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| GgufError::InvalidUtf8)
    }

    /// Read a single metadata value of the given GGUF type id.
    fn read_value(&mut self, vtype: u32) -> Result<MetaValue, GgufError> {
        Ok(match vtype {
            0 => MetaValue::U8(self.read_u8()?),
            1 => MetaValue::I8(self.read_u8()? as i8),
            2 => MetaValue::U16(self.read_u16()?),
            3 => MetaValue::I16(self.read_u16()? as i16),
            4 => MetaValue::U32(self.read_u32()?),
            5 => MetaValue::I32(self.read_u32()? as i32),
            6 => MetaValue::F32(f32::from_bits(self.read_u32()?)),
            7 => MetaValue::Bool(self.read_u8()? != 0),
            8 => MetaValue::String(self.read_string()?),
            9 => {
                let elem_type = self.read_u32()?;
                let count = self.read_u64()?;
                let mut items = Vec::with_capacity(count.min(4096) as usize);
                for _ in 0..count {
                    items.push(self.read_value(elem_type)?);
                }
                MetaValue::Array(items)
            }
            10 => MetaValue::U64(self.read_u64()?),
            11 => MetaValue::I64(self.read_u64()? as i64),
            12 => MetaValue::F64(f64::from_bits(self.read_u64()?)),
            other => return Err(GgufError::UnknownValueType(other)),
        })
    }
}

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

    /// Encode a tensor descriptor: name, n_dims (u32), shape (u64 each),
    /// kind (u32 ggml type), offset (u64). Cite fs/ggml gguf.go decode loop.
    fn put_tensor(buf: &mut Vec<u8>, name: &str, shape: &[u64], kind: u32, offset: u64) {
        put_str(buf, name);
        buf.extend_from_slice(&(shape.len() as u32).to_le_bytes());
        for d in shape {
            buf.extend_from_slice(&d.to_le_bytes());
        }
        buf.extend_from_slice(&kind.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
    }

    #[test]
    fn parses_tensor_info_after_kv_block() {
        // tensor_count=1, kv_count=1 (general.alignment = 16)
        let mut buf = header(3, 1, 1);
        put_str(&mut buf, "general.alignment");
        buf.extend_from_slice(&T_UINT32.to_le_bytes());
        buf.extend_from_slice(&16u32.to_le_bytes());
        // one tensor: token_embd.weight, [4096, 32000], kind 12 (Q4_K), offset 0
        put_tensor(&mut buf, "token_embd.weight", &[4096, 32000], 12, 0);

        let g = GgufFile::parse(&buf).expect("parse ok");
        assert_eq!(g.tensors.len(), 1);
        let t = &g.tensors[0];
        assert_eq!(t.name, "token_embd.weight");
        assert_eq!(t.shape, vec![4096u64, 32000]);
        assert_eq!(t.kind, 12);
        assert_eq!(t.offset, 0);
        assert_eq!(t.num_elements(), 4096 * 32000);
        assert_eq!(g.alignment(), 16);
    }

    #[test]
    fn tensor_kind_type_and_size_bytes() {
        // tensor_count=1, kv_count=0; one F32 (kind 0) tensor of [4096, 4096].
        let mut buf = header(3, 1, 0);
        put_tensor(&mut buf, "blk.0.attn_q.weight", &[4096, 4096], 0, 0);
        let g = GgufFile::parse(&buf).expect("parse ok");
        let t = &g.tensors[0];
        assert_eq!(t.kind_type(), crate::quant::TensorType(0));
        assert_eq!(t.kind_type().name(), "F32");
        // F32: (4096*4096 / 1) * 4 bytes = 67_108_864.
        assert_eq!(t.size_bytes(), 4096u64 * 4096 * 4);
    }

    #[test]
    fn alignment_defaults_to_32_when_absent() {
        let buf = header(3, 0, 0);
        let g = GgufFile::parse(&buf).expect("parse ok");
        assert_eq!(g.alignment(), 32);
        assert!(g.tensors.is_empty());
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
