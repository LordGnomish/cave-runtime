// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding quantization — IEEE-754 half (fp16) and per-vector int8.
//!
//! Quantized embeddings cut payload size: fp16 halves it, int8 quarters it.
//! fp16 is a lossy float conversion; int8 is symmetric per-vector scalar
//! quantization (`scale = max|x| / 127`) that approximately preserves cosine
//! ranking, which is all retrieval needs. Both are pure-Rust and dependency-free.

/// Convert an `f32` to its IEEE-754 binary16 bit pattern (round-to-nearest-even,
/// with overflow saturating to ±inf and subnormal handling).
pub fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    // Unbias the f32 exponent (bias 127) to compare against the f16 range.
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    let mantissa = bits & 0x007f_ffff;

    if exp == 128 {
        // inf / NaN
        if mantissa != 0 {
            return sign | 0x7e00; // canonical NaN
        }
        return sign | 0x7c00; // inf
    }
    if exp > 15 {
        return sign | 0x7c00; // overflow -> inf
    }
    if exp < -14 {
        // Subnormal or underflow to zero in f16.
        if exp < -24 {
            return sign; // too small -> signed zero
        }
        // Build the subnormal mantissa (implicit leading 1 included).
        let mant = (mantissa | 0x0080_0000) >> (-exp - 14 + 1) as u32;
        // Round to nearest even using the bit just below the kept range.
        let rounded = ((mant >> 13) + ((mant >> 12) & 1)) as u16;
        return sign | rounded;
    }
    // Normal f16.
    let exp16 = ((exp + 15) as u16) << 10;
    // Round mantissa from 23 -> 10 bits, nearest-even.
    let mant16 = ((mantissa >> 13) + ((mantissa >> 12) & 1)) as u16;
    // Mantissa rounding may overflow into the exponent — u16 add handles carry.
    sign | (exp16 + mant16)
}

/// Convert an IEEE-754 binary16 bit pattern back to `f32`.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x03ff) as u32;

    let bits = if exp == 0 {
        if mant == 0 {
            sign // signed zero
        } else {
            // Subnormal: normalize.
            let mut e = -1i32;
            let mut m = mant;
            while m & 0x0400 == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x03ff;
            let exp32 = (127 - 15 + 1 + e) as u32;
            sign | (exp32 << 23) | (m << 13)
        }
    } else if exp == 0x1f {
        // inf / NaN
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        let exp32 = exp + (127 - 15);
        sign | (exp32 << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

/// A per-vector int8-quantized embedding (symmetric scalar quantization).
#[derive(Debug, Clone, PartialEq)]
pub struct Int8Vector {
    /// Per-element scale: `dequantized = q * scale`.
    pub scale: f32,
    /// Quantized components in `[-127, 127]`.
    pub data: Vec<i8>,
}

impl Int8Vector {
    /// Quantize an `f32` vector. The scale is `max|x| / 127`; a zero vector
    /// quantizes to all zeros with scale 0.
    pub fn quantize(v: &[f32]) -> Self {
        let max_abs = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        if max_abs == 0.0 {
            return Int8Vector {
                scale: 0.0,
                data: vec![0i8; v.len()],
            };
        }
        let scale = max_abs / 127.0;
        let data = v
            .iter()
            .map(|x| {
                let q = (x / scale).round();
                q.clamp(-127.0, 127.0) as i8
            })
            .collect();
        Int8Vector { scale, data }
    }

    /// Reconstruct the approximate `f32` vector.
    pub fn dequantize(&self) -> Vec<f32> {
        self.data.iter().map(|&q| q as f32 * self.scale).collect()
    }
}
