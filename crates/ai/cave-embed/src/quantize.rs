// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding quantization — sentence-transformers `quantize_embeddings`.
//!
//! Upstream: `sentence_transformers/quantization.py`. Compresses float32
//! embeddings to int8/uint8 (scalar, per-dimension calibration ranges) or to
//! binary/ubinary (sign threshold + `np.packbits`, big-endian bit order).

use crate::error::{EmbedError, EmbedResult};

/// Target precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// No quantization.
    Float32,
    /// Signed 8-bit scalar quantization, `[-128, 127]`.
    Int8,
    /// Unsigned 8-bit scalar quantization, `[0, 255]`.
    Uint8,
    /// Sign-threshold bits packed big-endian, stored as `i8`.
    Binary,
    /// Sign-threshold bits packed big-endian, stored as `u8`.
    Ubinary,
}

/// Per-dimension calibration range (min/max) used by int8/uint8.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationRange {
    /// Per-dimension minimum.
    pub min: Vec<f32>,
    /// Per-dimension maximum.
    pub max: Vec<f32>,
}

impl CalibrationRange {
    /// Derive per-dimension min/max from a batch of embeddings.
    pub fn from_batch(embeddings: &[Vec<f32>]) -> EmbedResult<Self> {
        let first = embeddings.first().ok_or(EmbedError::EmptyInput)?;
        let dims = first.len();
        let mut min = vec![f32::INFINITY; dims];
        let mut max = vec![f32::NEG_INFINITY; dims];
        for e in embeddings {
            if e.len() != dims {
                return Err(EmbedError::ShapeMismatch {
                    tokens: e.len(),
                    mask: dims,
                });
            }
            for i in 0..dims {
                min[i] = min[i].min(e[i]);
                max[i] = max[i].max(e[i]);
            }
        }
        Ok(Self { min, max })
    }
}

/// A quantized embedding.
#[derive(Debug, Clone, PartialEq)]
pub enum QuantizedEmbedding {
    /// Unchanged float32.
    Float32(Vec<f32>),
    /// Signed 8-bit.
    Int8(Vec<i8>),
    /// Unsigned 8-bit.
    Uint8(Vec<u8>),
    /// Packed binary bits as i8.
    Binary(Vec<i8>),
    /// Packed binary bits as u8.
    Ubinary(Vec<u8>),
}

/// Pack sign bits big-endian: bit = (x > 0), MSB first, 8 dims per byte.
fn packbits(emb: &[f32]) -> Vec<u8> {
    let n_bytes = emb.len().div_ceil(8);
    let mut out = vec![0u8; n_bytes];
    for (i, &x) in emb.iter().enumerate() {
        if x > 0.0 {
            out[i / 8] |= 1 << (7 - (i % 8));
        }
    }
    out
}

/// Quantize one embedding.
pub fn quantize(
    emb: &[f32],
    precision: Precision,
    ranges: Option<&CalibrationRange>,
) -> EmbedResult<QuantizedEmbedding> {
    match precision {
        Precision::Float32 => Ok(QuantizedEmbedding::Float32(emb.to_vec())),
        Precision::Binary => Ok(QuantizedEmbedding::Binary(
            packbits(emb).into_iter().map(|b| b as i8).collect(),
        )),
        Precision::Ubinary => Ok(QuantizedEmbedding::Ubinary(packbits(emb))),
        Precision::Int8 | Precision::Uint8 => {
            let r = ranges.ok_or_else(|| {
                EmbedError::InvalidArgument(
                    "int8/uint8 quantization requires calibration ranges".into(),
                )
            })?;
            if r.min.len() != emb.len() || r.max.len() != emb.len() {
                return Err(EmbedError::ShapeMismatch {
                    tokens: emb.len(),
                    mask: r.min.len(),
                });
            }
            let mut int8 = Vec::with_capacity(emb.len());
            let mut uint8 = Vec::with_capacity(emb.len());
            for i in 0..emb.len() {
                let span = r.max[i] - r.min[i];
                // 256 buckets across [min, max]: step = span / 255.
                let bucket = if span <= 0.0 {
                    0i32
                } else {
                    let step = span / 255.0;
                    (((emb[i] - r.min[i]) / step).round() as i32).clamp(0, 255)
                };
                int8.push((bucket - 128) as i8);
                uint8.push(bucket as u8);
            }
            match precision {
                Precision::Int8 => Ok(QuantizedEmbedding::Int8(int8)),
                _ => Ok(QuantizedEmbedding::Uint8(uint8)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float32_passthrough() {
        let q = quantize(&[1.0, 2.0], Precision::Float32, None).unwrap();
        assert_eq!(q, QuantizedEmbedding::Float32(vec![1.0, 2.0]));
    }

    #[test]
    fn int8_maps_endpoints() {
        let r = CalibrationRange {
            min: vec![0.0],
            max: vec![10.0],
        };
        assert_eq!(
            quantize(&[0.0], Precision::Int8, Some(&r)).unwrap(),
            QuantizedEmbedding::Int8(vec![-128])
        );
        assert_eq!(
            quantize(&[10.0], Precision::Int8, Some(&r)).unwrap(),
            QuantizedEmbedding::Int8(vec![127])
        );
    }

    #[test]
    fn uint8_maps_endpoints() {
        let r = CalibrationRange {
            min: vec![0.0],
            max: vec![10.0],
        };
        assert_eq!(
            quantize(&[0.0], Precision::Uint8, Some(&r)).unwrap(),
            QuantizedEmbedding::Uint8(vec![0])
        );
        assert_eq!(
            quantize(&[10.0], Precision::Uint8, Some(&r)).unwrap(),
            QuantizedEmbedding::Uint8(vec![255])
        );
    }

    #[test]
    fn int8_requires_ranges() {
        assert!(matches!(
            quantize(&[1.0], Precision::Int8, None),
            Err(EmbedError::InvalidArgument(_))
        ));
    }

    #[test]
    fn binary_packs_big_endian() {
        // signs: + - + - + - + -  => bits 1010_1010 = 0xAA = 170 => i8 -86
        let v = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        assert_eq!(
            quantize(&v, Precision::Binary, None).unwrap(),
            QuantizedEmbedding::Binary(vec![-86])
        );
        assert_eq!(
            quantize(&v, Precision::Ubinary, None).unwrap(),
            QuantizedEmbedding::Ubinary(vec![170])
        );
    }

    #[test]
    fn binary_length_is_ceil_dims_over_8() {
        let v = vec![0.5f32; 16];
        match quantize(&v, Precision::Ubinary, None).unwrap() {
            QuantizedEmbedding::Ubinary(b) => assert_eq!(b.len(), 2),
            _ => panic!(),
        }
        let v = vec![0.5f32; 10];
        match quantize(&v, Precision::Ubinary, None).unwrap() {
            QuantizedEmbedding::Ubinary(b) => assert_eq!(b.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn int8_preserves_dimension_count() {
        let r = CalibrationRange::from_batch(&[vec![0.0, -1.0, 2.0], vec![4.0, 1.0, -2.0]]).unwrap();
        match quantize(&[1.0, 0.0, 0.0], Precision::Int8, Some(&r)).unwrap() {
            QuantizedEmbedding::Int8(v) => assert_eq!(v.len(), 3),
            _ => panic!(),
        }
    }

    #[test]
    fn calibration_from_batch_per_dim() {
        let r = CalibrationRange::from_batch(&[vec![1.0, 5.0], vec![-3.0, 2.0]]).unwrap();
        assert_eq!(r.min, vec![-3.0, 2.0]);
        assert_eq!(r.max, vec![1.0, 5.0]);
    }
}
