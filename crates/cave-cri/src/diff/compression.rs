// SPDX-License-Identifier: AGPL-3.0-or-later
//! gzip compression + diff-id computation.
//!
//! Mirrors `core/diff/diff.go`'s helpers. The "diff id" of a layer is
//! the SHA-256 of its *uncompressed* tarball — distinct from the
//! "layer digest" which is the SHA-256 of the *compressed* form. Both
//! end up in the manifest; this module produces the diff id, the
//! content store keeps the layer digest.

use crate::content::digest::{Digest, DigestAlgorithm};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};

#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

/// gzip-encode `bytes` at default compression.
pub fn compress_gzip(bytes: &[u8]) -> Result<Vec<u8>, CompressionError> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

/// gzip-decode `bytes`.
pub fn decompress_gzip(bytes: &[u8]) -> Result<Vec<u8>, CompressionError> {
    let mut dec = GzDecoder::new(bytes);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

/// Compute the "diff id" of a layer: SHA-256 of the *uncompressed*
/// tarball. Caller passes already-decompressed bytes.
pub fn compute_diff_id(uncompressed_tar: &[u8]) -> Digest {
    Digest::compute(DigestAlgorithm::Sha256, uncompressed_tar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_gzip_preserves_bytes() {
        let original = b"hello layer content".repeat(16);
        let compressed = compress_gzip(&original).unwrap();
        // Default compression usually shrinks repeated content.
        assert!(compressed.len() < original.len());
        let restored = decompress_gzip(&compressed).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn empty_bytes_round_trip() {
        let compressed = compress_gzip(b"").unwrap();
        let restored = decompress_gzip(&compressed).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn compute_diff_id_uses_sha256_of_uncompressed() {
        let payload = b"layer-bytes";
        let id = compute_diff_id(payload);
        assert_eq!(id.algorithm(), DigestAlgorithm::Sha256);
        // The diff id is the sha256 of the uncompressed bytes.
        let expected = Digest::compute(DigestAlgorithm::Sha256, payload);
        assert_eq!(id, expected);
    }

    #[test]
    fn diff_id_distinct_from_layer_digest() {
        let payload = b"x".repeat(2048);
        let compressed = compress_gzip(&payload).unwrap();
        let diff_id = compute_diff_id(&payload);
        let layer_digest = Digest::compute(DigestAlgorithm::Sha256, &compressed);
        assert_ne!(diff_id, layer_digest);
    }

    #[test]
    fn corrupted_gzip_rejected() {
        let mut bad = compress_gzip(b"ok").unwrap();
        // Flip a bit in the deflate stream.
        let mid = bad.len() / 2;
        bad[mid] ^= 0xff;
        assert!(decompress_gzip(&bad).is_err());
    }
}
