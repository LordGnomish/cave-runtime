//! Message compression codecs: gzip, snappy, lz4, zstd, none.

use crate::error::{StreamsError, StreamsResult};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Compression codec identifier — matches Kafka's attributes bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    #[default]
    None = 0,
    Gzip = 1,
    Snappy = 2,
    Lz4 = 3,
    Zstd = 4,
}

impl Codec {
    pub fn from_i8(v: i8) -> Self {
        match v & 0x07 {
            1 => Self::Gzip,
            2 => Self::Snappy,
            3 => Self::Lz4,
            4 => Self::Zstd,
            _ => Self::None,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "gzip" => Self::Gzip,
            "snappy" => Self::Snappy,
            "lz4" => Self::Lz4,
            "zstd" => Self::Zstd,
            _ => Self::None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Snappy => "snappy",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
        }
    }
}

/// Compress `data` with the given codec.
pub fn compress(codec: Codec, data: &[u8]) -> StreamsResult<Bytes> {
    match codec {
        Codec::None => Ok(Bytes::copy_from_slice(data)),
        Codec::Gzip => compress_gzip(data),
        Codec::Snappy => compress_snappy(data),
        Codec::Lz4 => compress_lz4(data),
        Codec::Zstd => compress_zstd(data),
    }
}

/// Decompress `data` with the given codec.
pub fn decompress(codec: Codec, data: &[u8]) -> StreamsResult<Bytes> {
    match codec {
        Codec::None => Ok(Bytes::copy_from_slice(data)),
        Codec::Gzip => decompress_gzip(data),
        Codec::Snappy => decompress_snappy(data),
        Codec::Lz4 => decompress_lz4(data),
        Codec::Zstd => decompress_zstd(data),
    }
}

// ── gzip ──────────────────────────────────────────────────────────────────────

fn compress_gzip(data: &[u8]) -> StreamsResult<Bytes> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).map_err(|e| StreamsError::Compression {
        codec: "gzip".into(),
        message: e.to_string(),
    })?;
    let out = enc.finish().map_err(|e| StreamsError::Compression {
        codec: "gzip".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

fn decompress_gzip(data: &[u8]) -> StreamsResult<Bytes> {
    use flate2::read::GzDecoder;
    let mut dec = GzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|e| StreamsError::Compression {
        codec: "gzip".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

// ── snappy ────────────────────────────────────────────────────────────────────

fn compress_snappy(data: &[u8]) -> StreamsResult<Bytes> {
    let mut enc = snap::write::FrameEncoder::new(Vec::new());
    enc.write_all(data).map_err(|e| StreamsError::Compression {
        codec: "snappy".into(),
        message: e.to_string(),
    })?;
    let out = enc.into_inner().map_err(|e| StreamsError::Compression {
        codec: "snappy".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

fn decompress_snappy(data: &[u8]) -> StreamsResult<Bytes> {
    let mut dec = snap::read::FrameDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|e| StreamsError::Compression {
        codec: "snappy".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

// ── lz4 ───────────────────────────────────────────────────────────────────────

fn compress_lz4(data: &[u8]) -> StreamsResult<Bytes> {
    let out = lz4_flex::compress_prepend_size(data);
    Ok(Bytes::from(out))
}

fn decompress_lz4(data: &[u8]) -> StreamsResult<Bytes> {
    let out = lz4_flex::decompress_size_prepended(data).map_err(|e| {
        StreamsError::Compression {
            codec: "lz4".into(),
            message: e.to_string(),
        }
    })?;
    Ok(Bytes::from(out))
}

// ── zstd ──────────────────────────────────────────────────────────────────────

fn compress_zstd(data: &[u8]) -> StreamsResult<Bytes> {
    let out = zstd::encode_all(data, 3).map_err(|e| StreamsError::Compression {
        codec: "zstd".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

fn decompress_zstd(data: &[u8]) -> StreamsResult<Bytes> {
    let out = zstd::decode_all(data).map_err(|e| StreamsError::Compression {
        codec: "zstd".into(),
        message: e.to_string(),
    })?;
    Ok(Bytes::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &[u8] = b"The quick brown fox jumps over the lazy dog. Kafka message compression test.";

    fn roundtrip(codec: Codec) {
        let compressed = compress(codec, PAYLOAD).unwrap();
        if codec != Codec::None {
            // Compressed should generally differ from original (for non-trivial payloads)
            assert!(!compressed.is_empty());
        }
        let decompressed = decompress(codec, &compressed).unwrap();
        assert_eq!(&decompressed[..], PAYLOAD);
    }

    #[test]
    fn roundtrip_none() {
        roundtrip(Codec::None);
    }

    #[test]
    fn roundtrip_gzip() {
        roundtrip(Codec::Gzip);
    }

    #[test]
    fn roundtrip_snappy() {
        roundtrip(Codec::Snappy);
    }

    #[test]
    fn roundtrip_lz4() {
        roundtrip(Codec::Lz4);
    }

    #[test]
    fn roundtrip_zstd() {
        roundtrip(Codec::Zstd);
    }

    #[test]
    fn codec_from_name() {
        assert_eq!(Codec::from_name("gzip"), Codec::Gzip);
        assert_eq!(Codec::from_name("SNAPPY"), Codec::Snappy);
        assert_eq!(Codec::from_name("lz4"), Codec::Lz4);
        assert_eq!(Codec::from_name("zstd"), Codec::Zstd);
        assert_eq!(Codec::from_name("none"), Codec::None);
        assert_eq!(Codec::from_name("unknown"), Codec::None);
    }
}
