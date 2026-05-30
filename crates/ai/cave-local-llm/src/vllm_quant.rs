// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! vLLM weight-quantization config metadata — a pure-Rust port of vLLM's
//! AWQ / GPTQ / FP8 quantization configs
//! (vllm-project/vllm `vllm/model_executor/layers/quantization/`,
//! Apache-2.0).
//!
//! These describe how a linear layer's weights are packed: the bit width,
//! the int32 **pack factor** (`32 / bits` sub-byte weights per word), the
//! group size for per-group scales, and whether a zero-point is stored. The
//! sizing helpers compute the packed footprint and the compression ratio
//! against fp16 — the introspection a model loader needs without running any
//! GPU dequant kernel (kernels live in a GPU runtime, out of scope here).

use thiserror::Error;

/// Which quantization scheme a config describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantMethod {
    /// Activation-aware Weight Quantization (4-bit).
    Awq,
    /// GPTQ post-training quantization (2/3/4/8-bit).
    Gptq,
    /// 8-bit floating point.
    Fp8,
}

/// FP8 representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fp8Format {
    /// 4 exponent / 3 mantissa bits (higher precision, smaller range).
    E4M3,
    /// 5 exponent / 2 mantissa bits (larger range, lower precision).
    E5M2,
}

/// How FP8 activation scales are determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationScheme {
    /// Scales computed per-batch at runtime.
    Dynamic,
    /// Scales calibrated ahead of time.
    Static,
}

/// Quantization config errors mirroring vLLM's config-time `ValueError`s.
#[derive(Debug, Error, PartialEq)]
pub enum QuantError {
    /// Bit width unsupported by the method.
    #[error("{method:?} does not support {bits}-bit quantization")]
    UnsupportedBits {
        /// Offending method.
        method: QuantMethod,
        /// Requested bit width.
        bits: u32,
    },
    /// `group_size` is neither -1 (per-channel) nor a positive integer.
    #[error("group_size must be -1 or a positive integer, got {0}")]
    InvalidGroupSize(i32),
    /// `in_features` is not divisible by the group size.
    #[error("in_features {in_features} not divisible by group_size {group_size}")]
    GroupMismatch {
        /// Layer input dimension.
        in_features: usize,
        /// Configured group size.
        group_size: usize,
    },
    /// `in_features * out_features` is not divisible by the pack factor.
    #[error("weight count {count} not divisible by pack_factor {pack_factor}")]
    PackMismatch {
        /// `in_features * out_features`.
        count: usize,
        /// `32 / bits`.
        pack_factor: usize,
    },
}

/// A weight-quantization configuration for one linear-layer family.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantConfig {
    /// Scheme.
    pub method: QuantMethod,
    /// Weight bit width.
    pub weight_bits: u32,
    /// Group size for per-group scales (-1 = per output channel).
    pub group_size: i32,
    /// GPTQ symmetric quantization (no zero-point).
    pub sym: bool,
    /// GPTQ activation-order reordering.
    pub desc_act: bool,
    /// Stores an explicit per-group zero-point (AWQ / asymmetric GPTQ).
    pub zero_point: bool,
    /// FP8 representation, if `method == Fp8`.
    pub fp8_format: Option<Fp8Format>,
    /// FP8 activation scaling, if `method == Fp8`.
    pub activation_scheme: Option<ActivationScheme>,
}

fn check_group_size(group_size: i32) -> Result<(), QuantError> {
    if group_size == -1 || group_size > 0 {
        Ok(())
    } else {
        Err(QuantError::InvalidGroupSize(group_size))
    }
}

impl QuantConfig {
    /// Build an AWQ config (4-bit only; zero-point always stored).
    pub fn awq(weight_bits: u32, group_size: i32) -> Result<Self, QuantError> {
        if weight_bits != 4 {
            return Err(QuantError::UnsupportedBits {
                method: QuantMethod::Awq,
                bits: weight_bits,
            });
        }
        check_group_size(group_size)?;
        Ok(Self {
            method: QuantMethod::Awq,
            weight_bits,
            group_size,
            sym: false,
            desc_act: false,
            zero_point: true,
            fp8_format: None,
            activation_scheme: None,
        })
    }

    /// Build a GPTQ config (2/3/4/8-bit). Symmetric drops the zero-point.
    pub fn gptq(
        weight_bits: u32,
        group_size: i32,
        desc_act: bool,
        sym: bool,
    ) -> Result<Self, QuantError> {
        if !matches!(weight_bits, 2 | 3 | 4 | 8) {
            return Err(QuantError::UnsupportedBits {
                method: QuantMethod::Gptq,
                bits: weight_bits,
            });
        }
        check_group_size(group_size)?;
        Ok(Self {
            method: QuantMethod::Gptq,
            weight_bits,
            group_size,
            sym,
            desc_act,
            zero_point: !sym,
            fp8_format: None,
            activation_scheme: None,
        })
    }

    /// Build an FP8 config (8-bit; per-channel weight scale, no zero-point).
    pub fn fp8(format: Fp8Format, activation_scheme: ActivationScheme) -> Self {
        Self {
            method: QuantMethod::Fp8,
            weight_bits: 8,
            group_size: -1,
            sym: true,
            desc_act: false,
            zero_point: false,
            fp8_format: Some(format),
            activation_scheme: Some(activation_scheme),
        }
    }

    /// Sub-byte weights packed per int32 word (`32 / weight_bits`).
    pub fn pack_factor(&self) -> u32 {
        32 / self.weight_bits
    }

    /// Number of scale groups along `in_features` (1 for per-channel).
    pub fn num_groups(&self, in_features: usize) -> usize {
        if self.group_size == -1 {
            1
        } else {
            in_features / self.group_size as usize
        }
    }

    /// Validate a concrete layer shape against this config.
    pub fn validate_shape(&self, in_features: usize, out_features: usize) -> Result<(), QuantError> {
        if self.group_size != -1 {
            let gs = self.group_size as usize;
            if in_features % gs != 0 {
                return Err(QuantError::GroupMismatch {
                    in_features,
                    group_size: gs,
                });
            }
        }
        let pack = self.pack_factor() as usize;
        let count = in_features * out_features;
        if count % pack != 0 {
            return Err(QuantError::PackMismatch {
                count,
                pack_factor: pack,
            });
        }
        Ok(())
    }

    /// Packed-weight size in int32 words (`in*out / pack_factor`).
    pub fn packed_weight_words(
        &self,
        in_features: usize,
        out_features: usize,
    ) -> Result<usize, QuantError> {
        self.validate_shape(in_features, out_features)?;
        Ok(in_features * out_features / self.pack_factor() as usize)
    }

    /// Total quantized footprint in bytes: packed weights (int32) + fp16
    /// per-group scales + packed zero-points (when stored).
    pub fn quantized_bytes(&self, in_features: usize, out_features: usize) -> usize {
        let pack = self.pack_factor() as usize;
        let weight_words = in_features * out_features / pack;
        let weight_bytes = weight_words * 4;
        let groups = self.num_groups(in_features);
        let scale_bytes = groups * out_features * 2; // fp16 scales
        let zero_bytes = if self.zero_point {
            (groups * out_features / pack) * 4 // packed int32 zero-points
        } else {
            0
        };
        weight_bytes + scale_bytes + zero_bytes
    }

    /// Bytes an fp16 (unquantized) weight of this shape would occupy.
    pub fn fp16_bytes(in_features: usize, out_features: usize) -> usize {
        in_features * out_features * 2
    }

    /// Quantized footprint as a fraction of the fp16 footprint (< 1.0).
    pub fn compression_ratio(&self, in_features: usize, out_features: usize) -> f64 {
        let q = self.quantized_bytes(in_features, out_features) as f64;
        let f = Self::fp16_bytes(in_features, out_features) as f64;
        q / f
    }
}
