// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Chunked, length-delimited streaming framing for Prometheus remote-read —
//! line-by-line port of prometheus/prometheus `storage/remote/chunked.go`
//! (v3.12.0, source_sha a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! This is the transport that backs the `STREAMED_XOR_CHUNKS` remote-read
//! response: instead of one buffered protobuf, the server streams a sequence
//! of self-delimited frames, each `uvarint(len) ++ CRC32C(len-be? no: data) ++
//! payload`. The CRC is Castagnoli (CRC32C), matching upstream's
//! `crc32.MakeTable(crc32.Castagnoli)`. Porting the framing closes the
//! "block-streaming deferred" gap on the remote-read path.

use crate::error::{MetricsError, Result};

/// Writes length-delimited, CRC32C-protected frames into an in-memory buffer.
#[derive(Default)]
pub struct ChunkedWriter {
    buf: Vec<u8>,
}

impl ChunkedWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append one frame: `uvarint(len(data))` then the 4-byte big-endian
    /// CRC32C of `data`, then `data` itself (upstream `ChunkedWriter.Write`).
    pub fn write(&mut self, data: &[u8]) {
        put_uvarint(&mut self.buf, data.len() as u64);
        let crc = crc32c(data);
        self.buf.extend_from_slice(&crc.to_be_bytes());
        self.buf.extend_from_slice(data);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Reads frames produced by [`ChunkedWriter`], verifying each CRC32C.
pub struct ChunkedReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ChunkedReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Return the next frame's payload, `None` at clean end-of-stream, or an
    /// error on truncation / CRC mismatch (upstream `ChunkedReader.Next`).
    pub fn next(&mut self) -> Result<Option<Vec<u8>>> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let (size, consumed) = read_uvarint(&self.data[self.pos..])
            .ok_or_else(|| MetricsError::Ingestion("chunked: truncated uvarint size".into()))?;
        self.pos += consumed;

        if self.pos + 4 > self.data.len() {
            return Err(MetricsError::Ingestion("chunked: truncated CRC".into()));
        }
        let mut crc_bytes = [0u8; 4];
        crc_bytes.copy_from_slice(&self.data[self.pos..self.pos + 4]);
        let expected = u32::from_be_bytes(crc_bytes);
        self.pos += 4;

        let size = size as usize;
        if self.pos + size > self.data.len() {
            return Err(MetricsError::Ingestion("chunked: truncated payload".into()));
        }
        let payload = self.data[self.pos..self.pos + size].to_vec();
        self.pos += size;

        let actual = crc32c(&payload);
        if actual != expected {
            return Err(MetricsError::Ingestion(format!(
                "chunked: CRC mismatch (expected {expected:#010x}, got {actual:#010x})"
            )));
        }
        Ok(Some(payload))
    }

    /// Drain all remaining frames.
    pub fn read_all(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        while let Some(frame) = self.next()? {
            out.push(frame);
        }
        Ok(out)
    }
}

/// Append `v` to `buf` as an unsigned LEB128 varint (Go `binary.PutUvarint`).
fn put_uvarint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Read an unsigned LEB128 varint; returns `(value, bytes_consumed)`.
fn read_uvarint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate() {
        if shift >= 64 {
            return None;
        }
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None
}

/// CRC-32C (Castagnoli), reflected polynomial 0x82F63B78 — bit-for-bit
/// identical to Go's `crc32.Checksum(b, crc32.MakeTable(crc32.Castagnoli))`.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x82F6_3B78
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_known_vector() {
        // CRC32C of the ASCII string "123456789" is 0xE3069283 (Castagnoli check value).
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn uvarint_roundtrip() {
        for v in [0u64, 1, 127, 128, 300, 16384, 1_000_000, u32::MAX as u64] {
            let mut b = Vec::new();
            put_uvarint(&mut b, v);
            let (got, n) = read_uvarint(&b).unwrap();
            assert_eq!(got, v);
            assert_eq!(n, b.len());
        }
    }
}
