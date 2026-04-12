//! Chunk-based log storage with pluggable compression.
//!
//! A chunk holds a contiguous, sorted slice of log entries for a single stream.
//! On flush it is compressed and written to the chunk store; the in-memory
//! "head chunk" accumulates writes until it reaches a size/age threshold.

use std::io::{Read, Write};
use serde::{Deserialize, Serialize};

use crate::models::{Chunk, Codec, LogEntry, TimestampNs};

/// Maximum uncompressed size before a head chunk is flushed (default 256 KiB).
pub const DEFAULT_CHUNK_TARGET_SIZE: usize = 256 * 1024;
/// Maximum age of a head chunk before forced flush (seconds).
pub const DEFAULT_CHUNK_MAX_AGE_SECS: u64 = 60;

/// Serialisable representation of a single entry inside a chunk.
#[derive(Debug, Serialize, Deserialize)]
struct WireEntry {
    ts: TimestampNs,
    line: String,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    meta: std::collections::HashMap<String, String>,
}

// ── Encoding / decoding ──────────────────────────────────────────────────────

/// Encode `entries` to a newline-delimited JSON byte buffer, then compress.
pub fn encode_chunk(entries: &[LogEntry], codec: Codec) -> anyhow::Result<Vec<u8>> {
    let mut raw: Vec<u8> = Vec::with_capacity(entries.len() * 128);
    for e in entries {
        let wire = WireEntry { ts: e.ts, line: e.line.clone(), meta: e.metadata.clone() };
        serde_json::to_writer(&mut raw, &wire)?;
        raw.push(b'\n');
    }
    compress(&raw, codec)
}

/// Decode a chunk back into log entries.
pub fn decode_chunk(chunk: &Chunk) -> anyhow::Result<Vec<LogEntry>> {
    let raw = decompress(&chunk.data, chunk.codec)?;
    let mut entries = Vec::with_capacity(chunk.num_entries as usize);
    for line in raw.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let wire: WireEntry = serde_json::from_slice(line)?;
        entries.push(LogEntry { ts: wire.ts, line: wire.line, metadata: wire.meta });
    }
    Ok(entries)
}

// ── Compression ──────────────────────────────────────────────────────────────

pub fn compress(data: &[u8], codec: Codec) -> anyhow::Result<Vec<u8>> {
    match codec {
        Codec::None => Ok(data.to_vec()),
        Codec::Gzip => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data)?;
            Ok(enc.finish()?)
        }
        Codec::Snappy => {
            let mut wtr = snap::write::FrameEncoder::new(Vec::new());
            wtr.write_all(data)?;
            Ok(wtr.into_inner().map_err(|e| anyhow::anyhow!("{}", e))?)
        }
        Codec::Lz4 => Ok(lz4_flex::compress_prepend_size(data)),
        Codec::Zstd => {
            let mut enc = zstd::Encoder::new(Vec::new(), 3)?;
            enc.write_all(data)?;
            Ok(enc.finish()?)
        }
    }
}

pub fn decompress(data: &[u8], codec: Codec) -> anyhow::Result<Vec<u8>> {
    match codec {
        Codec::None => Ok(data.to_vec()),
        Codec::Gzip => {
            let mut dec = flate2::read::GzDecoder::new(data);
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
        Codec::Snappy => {
            let mut dec = snap::read::FrameDecoder::new(data);
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
        Codec::Lz4 => Ok(lz4_flex::decompress_size_prepended(data)?),
        Codec::Zstd => {
            let mut dec = zstd::Decoder::new(data)?;
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
    }
}

/// Protobuf+snappy (Loki wire format for /loki/api/v1/push).
/// Snappy raw encoding (not framing) used by Loki.
pub fn snappy_raw_decompress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let len = snap::raw::decompress_len(data)?;
    let mut out = vec![0u8; len];
    snap::raw::Decoder::new().decompress(data, &mut out)?;
    Ok(out)
}

pub fn snappy_raw_compress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let max_len = snap::raw::max_compress_len(data.len());
    let mut out = vec![0u8; max_len];
    let written = snap::raw::Encoder::new().compress(data, &mut out)?;
    out.truncate(written);
    Ok(out)
}

// ── Head chunk (mutable accumulator) ─────────────────────────────────────────

/// In-memory head chunk that accumulates new entries for one stream.
#[derive(Debug)]
pub struct HeadChunk {
    pub stream_fp: u64,
    pub tenant: String,
    pub entries: Vec<LogEntry>,
    pub created_at: std::time::Instant,
    pub uncompressed_size: usize,
}

impl HeadChunk {
    pub fn new(stream_fp: u64, tenant: impl Into<String>) -> Self {
        Self {
            stream_fp,
            tenant: tenant.into(),
            entries: Vec::new(),
            created_at: std::time::Instant::now(),
            uncompressed_size: 0,
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        self.uncompressed_size += entry.size_bytes();
        self.entries.push(entry);
    }

    pub fn should_flush(&self, target: usize, max_age_secs: u64) -> bool {
        self.uncompressed_size >= target
            || self.created_at.elapsed().as_secs() >= max_age_secs
    }

    pub fn min_ts(&self) -> Option<TimestampNs> {
        self.entries.first().map(|e| e.ts)
    }

    pub fn max_ts(&self) -> Option<TimestampNs> {
        self.entries.last().map(|e| e.ts)
    }

    /// Seal this head chunk into a compressed `Chunk`.
    pub fn flush(self, codec: Codec) -> anyhow::Result<Chunk> {
        let min_ts = self.min_ts().unwrap_or(0);
        let max_ts = self.max_ts().unwrap_or(0);
        let num_entries = self.entries.len() as u64;
        let uncompressed_size = self.uncompressed_size as u64;
        let data = encode_chunk(&self.entries, codec)?;
        Ok(Chunk {
            stream_fp: self.stream_fp,
            tenant: self.tenant,
            min_ts,
            max_ts,
            codec,
            data,
            num_entries,
            uncompressed_size,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LogEntry;

    fn sample_entries(n: usize) -> Vec<LogEntry> {
        (0..n).map(|i| LogEntry::new(i as i64 * 1_000_000_000, format!("log line {}", i))).collect()
    }

    #[test]
    fn roundtrip_none() {
        let entries = sample_entries(10);
        let chunk = HeadChunk { stream_fp: 1, tenant: "t".into(), entries: entries.clone(),
            created_at: std::time::Instant::now(), uncompressed_size: 100 };
        let sealed = chunk.flush(Codec::None).unwrap();
        let decoded = decode_chunk(&sealed).unwrap();
        assert_eq!(decoded.len(), 10);
        assert_eq!(decoded[0].line, "log line 0");
    }

    #[test]
    fn roundtrip_snappy() {
        let entries = sample_entries(100);
        let chunk = HeadChunk { stream_fp: 2, tenant: "t".into(), entries: entries.clone(),
            created_at: std::time::Instant::now(), uncompressed_size: 1000 };
        let sealed = chunk.flush(Codec::Snappy).unwrap();
        assert!(sealed.data.len() < sealed.uncompressed_size as usize);
        let decoded = decode_chunk(&sealed).unwrap();
        assert_eq!(decoded.len(), 100);
    }

    #[test]
    fn roundtrip_gzip() {
        let entries = sample_entries(50);
        let sz: usize = entries.iter().map(|e| e.size_bytes()).sum();
        let chunk = HeadChunk { stream_fp: 3, tenant: "t".into(), entries,
            created_at: std::time::Instant::now(), uncompressed_size: sz };
        let sealed = chunk.flush(Codec::Gzip).unwrap();
        let decoded = decode_chunk(&sealed).unwrap();
        assert_eq!(decoded.len(), 50);
    }

    #[test]
    fn roundtrip_lz4() {
        let entries = sample_entries(50);
        let sz: usize = entries.iter().map(|e| e.size_bytes()).sum();
        let chunk = HeadChunk { stream_fp: 4, tenant: "t".into(), entries,
            created_at: std::time::Instant::now(), uncompressed_size: sz };
        let sealed = chunk.flush(Codec::Lz4).unwrap();
        let decoded = decode_chunk(&sealed).unwrap();
        assert_eq!(decoded.len(), 50);
    }

    #[test]
    fn roundtrip_zstd() {
        let entries = sample_entries(50);
        let sz: usize = entries.iter().map(|e| e.size_bytes()).sum();
        let chunk = HeadChunk { stream_fp: 5, tenant: "t".into(), entries,
            created_at: std::time::Instant::now(), uncompressed_size: sz };
        let sealed = chunk.flush(Codec::Zstd).unwrap();
        let decoded = decode_chunk(&sealed).unwrap();
        assert_eq!(decoded.len(), 50);
    }

    #[test]
    fn snappy_raw_roundtrip() {
        let data = b"hello world this is some log data that needs compression";
        let compressed = snappy_raw_compress(data).unwrap();
        let decompressed = snappy_raw_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn head_chunk_flush_threshold() {
        let mut hc = HeadChunk::new(1, "tenant");
        assert!(!hc.should_flush(1000, 300));
        for i in 0..100 {
            hc.push(LogEntry::new(i, "x".repeat(20)));
        }
        assert!(hc.should_flush(1000, 300));
    }
}
