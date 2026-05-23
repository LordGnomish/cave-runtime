// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Port of `pkg/sources/chunker.go`. Splits a `Read` stream into bounded
//! chunks with a configurable peek window so multi-part credentials at the
//! boundary still survive detection.

use std::io::Read;

/// Upstream default: 10 KiB chunk with 512 B peek.
pub const DEFAULT_CHUNK_SIZE: usize = 10 * 1024;
pub const DEFAULT_PEEK_SIZE: usize = 512;

#[derive(Debug, Clone)]
pub struct Chunker {
    pub chunk_size: usize,
    pub peek_size: usize,
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            peek_size: DEFAULT_PEEK_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBytes {
    pub data: Vec<u8>,
    pub offset: usize,
}

impl Chunker {
    pub fn new(chunk_size: usize, peek_size: usize) -> Self {
        assert!(chunk_size > 0);
        Self {
            chunk_size,
            peek_size,
        }
    }

    /// Slice an in-memory buffer using the chunker boundaries. Mirrors the
    /// upstream `Read` loop with `bufio.NewReaderSize` + `Peek(peekSize)`.
    pub fn chunk_bytes(&self, mut input: &[u8]) -> Vec<ChunkBytes> {
        let mut out = Vec::new();
        let mut offset = 0usize;
        while !input.is_empty() {
            let take = self.chunk_size.min(input.len());
            let peek_extra = (input.len() - take).min(self.peek_size);
            let total = take + peek_extra;
            out.push(ChunkBytes {
                data: input[..total].to_vec(),
                offset,
            });
            offset += take;
            input = &input[take..];
        }
        out
    }

    /// Drain a `Read` source into chunks. Used by stdin / filesystem sources.
    pub fn chunk_reader<R: Read>(&self, mut r: R) -> std::io::Result<Vec<ChunkBytes>> {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf)?;
        Ok(self.chunk_bytes(&buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_chunks() {
        let c = Chunker::default();
        assert!(c.chunk_bytes(b"").is_empty());
    }

    #[test]
    fn single_short_chunk_is_one_band() {
        let c = Chunker::default();
        let r = c.chunk_bytes(b"hello world");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].data, b"hello world");
        assert_eq!(r[0].offset, 0);
    }

    #[test]
    fn boundary_chunk_includes_peek() {
        let c = Chunker::new(10, 4);
        let r = c.chunk_bytes(b"0123456789ABCDE");
        // 15-byte input, chunk=10, peek=4 -> first window is 14 bytes
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].data.len(), 14);
        assert_eq!(r[0].offset, 0);
        assert_eq!(r[1].offset, 10);
        assert_eq!(r[1].data, b"ABCDE");
    }

    #[test]
    fn many_chunks_have_monotonic_offsets() {
        let c = Chunker::new(8, 2);
        let data: Vec<u8> = (0..40u8).collect();
        let r = c.chunk_bytes(&data);
        let mut prev = 0;
        for (i, c) in r.iter().enumerate() {
            if i > 0 {
                assert!(c.offset >= prev);
            }
            prev = c.offset;
        }
    }

    #[test]
    fn reader_path_matches_byte_path() {
        let c = Chunker::default();
        let payload: Vec<u8> = (0..20_000u32).map(|x| (x & 0xff) as u8).collect();
        let a = c.chunk_bytes(&payload);
        let b = c.chunk_reader(payload.as_slice()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn peek_does_not_advance_offset() {
        let c = Chunker::new(4, 4);
        let r = c.chunk_bytes(b"ABCDEFGH");
        assert_eq!(r[0].data, b"ABCDEFGH");
        assert_eq!(r[0].offset, 0);
        assert_eq!(r[1].data, b"EFGH");
        assert_eq!(r[1].offset, 4);
    }
}
