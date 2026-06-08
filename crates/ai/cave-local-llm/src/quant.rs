// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GGUF quantization file-type helpers.
//!
//! Cite ollama/ollama `fs/ggml/type.go` — the `FileType` enum (llama_ftype
//! equivalent), its `ParseFileType`/`String` round-trip, and the integer ids
//! assigned by iota. This module provides a pure-Rust mirror plus two derived
//! utilities the registry/daemon needs but upstream computes in the runtime:
//! approximate **bits-per-weight** and a model **size estimate**. It does not
//! perform quantization itself (that shells out to `llama-quantize` upstream —
//! an explicit scope-cut).

use std::fmt;
use std::str::FromStr;

/// A GGUF quantization file type. Cite fs/ggml/type.go `FileType`. Names and
/// integer ids mirror upstream; variants are limited to the formats Ollama
/// actually emits or loads.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuantType {
    F32,
    F16,
    BF16,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    Q2_K,
    Q3_K_S,
    Q3_K_M,
    Q3_K_L,
    Q4_K_S,
    Q4_K_M,
    Q5_K_S,
    Q5_K_M,
    Q6_K,
}

impl QuantType {
    /// Canonical upstream name (e.g. `"Q4_K_M"`).
    pub fn name(self) -> &'static str {
        match self {
            QuantType::F32 => "F32",
            QuantType::F16 => "F16",
            QuantType::BF16 => "BF16",
            QuantType::Q4_0 => "Q4_0",
            QuantType::Q4_1 => "Q4_1",
            QuantType::Q5_0 => "Q5_0",
            QuantType::Q5_1 => "Q5_1",
            QuantType::Q8_0 => "Q8_0",
            QuantType::Q2_K => "Q2_K",
            QuantType::Q3_K_S => "Q3_K_S",
            QuantType::Q3_K_M => "Q3_K_M",
            QuantType::Q3_K_L => "Q3_K_L",
            QuantType::Q4_K_S => "Q4_K_S",
            QuantType::Q4_K_M => "Q4_K_M",
            QuantType::Q5_K_S => "Q5_K_S",
            QuantType::Q5_K_M => "Q5_K_M",
            QuantType::Q6_K => "Q6_K",
        }
    }

    /// The upstream `FileType` integer id (iota order in fs/ggml/type.go).
    pub fn file_type_id(self) -> u32 {
        match self {
            QuantType::F32 => 0,
            QuantType::F16 => 1,
            QuantType::Q4_0 => 2,
            QuantType::Q4_1 => 3,
            QuantType::Q8_0 => 7,
            QuantType::Q5_0 => 8,
            QuantType::Q5_1 => 9,
            QuantType::Q2_K => 10,
            QuantType::Q3_K_S => 11,
            QuantType::Q3_K_M => 12,
            QuantType::Q3_K_L => 13,
            QuantType::Q4_K_S => 14,
            QuantType::Q4_K_M => 15,
            QuantType::Q5_K_S => 16,
            QuantType::Q5_K_M => 17,
            QuantType::Q6_K => 18,
            QuantType::BF16 => 32,
        }
    }

    /// Resolve from an upstream `FileType` integer id.
    pub fn from_file_type(id: u32) -> Option<QuantType> {
        Some(match id {
            0 => QuantType::F32,
            1 => QuantType::F16,
            2 => QuantType::Q4_0,
            3 => QuantType::Q4_1,
            7 => QuantType::Q8_0,
            8 => QuantType::Q5_0,
            9 => QuantType::Q5_1,
            10 => QuantType::Q2_K,
            11 => QuantType::Q3_K_S,
            12 => QuantType::Q3_K_M,
            13 => QuantType::Q3_K_L,
            14 => QuantType::Q4_K_S,
            15 => QuantType::Q4_K_M,
            16 => QuantType::Q5_K_S,
            17 => QuantType::Q5_K_M,
            18 => QuantType::Q6_K,
            32 => QuantType::BF16,
            _ => return None,
        })
    }

    /// Approximate **bits per weight** including block overhead. These are the
    /// standard llama.cpp effective rates used for size estimation; they are
    /// approximations, not exact per-tensor figures.
    pub fn bits_per_weight(self) -> f32 {
        match self {
            QuantType::F32 => 32.0,
            QuantType::F16 | QuantType::BF16 => 16.0,
            QuantType::Q8_0 => 8.5,
            QuantType::Q6_K => 6.5625,
            QuantType::Q5_1 => 6.0,
            QuantType::Q5_0 | QuantType::Q5_K_S | QuantType::Q5_K_M => 5.5,
            QuantType::Q4_1 => 5.0,
            QuantType::Q4_K_M => 4.85,
            QuantType::Q4_K_S => 4.58,
            QuantType::Q4_0 => 4.5,
            QuantType::Q3_K_L => 4.27,
            QuantType::Q3_K_M => 3.91,
            QuantType::Q3_K_S => 3.5,
            QuantType::Q2_K => 2.63,
        }
    }

    /// Estimate the on-disk size in bytes for a model with `num_params`
    /// weights at this quantization (`params * bits_per_weight / 8`).
    pub fn estimate_bytes(self, num_params: u64) -> u64 {
        (num_params as f64 * self.bits_per_weight() as f64 / 8.0) as u64
    }
}

impl fmt::Display for QuantType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error returned when a string is not a recognised quantization name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseQuantError(pub String);

impl fmt::Display for ParseQuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown quantization type: {}", self.0)
    }
}

impl std::error::Error for ParseQuantError {}

impl FromStr for QuantType {
    type Err = ParseQuantError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept the common "Q4_K" alias for Q4_K_M (cite ParseFileType).
        let normalized = s.trim();
        let q = match normalized {
            "F32" => QuantType::F32,
            "F16" => QuantType::F16,
            "BF16" => QuantType::BF16,
            "Q4_0" => QuantType::Q4_0,
            "Q4_1" => QuantType::Q4_1,
            "Q5_0" => QuantType::Q5_0,
            "Q5_1" => QuantType::Q5_1,
            "Q8_0" => QuantType::Q8_0,
            "Q2_K" => QuantType::Q2_K,
            "Q3_K_S" => QuantType::Q3_K_S,
            "Q3_K_M" => QuantType::Q3_K_M,
            "Q3_K_L" => QuantType::Q3_K_L,
            "Q4_K_S" => QuantType::Q4_K_S,
            "Q4_K_M" | "Q4_K" => QuantType::Q4_K_M,
            "Q5_K_S" => QuantType::Q5_K_S,
            "Q5_K_M" => QuantType::Q5_K_M,
            "Q6_K" => QuantType::Q6_K,
            other => return Err(ParseQuantError(other.to_string())),
        };
        Ok(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_display_roundtrip() {
        for name in ["F32", "F16", "Q4_0", "Q4_K_M", "Q5_K_M", "Q6_K", "Q8_0", "BF16"] {
            let q: QuantType = name.parse().unwrap();
            assert_eq!(q.to_string(), name, "roundtrip {name}");
        }
    }

    #[test]
    fn parse_unknown_errors() {
        assert!("totally-not-a-quant".parse::<QuantType>().is_err());
    }

    #[test]
    fn from_file_type_maps_known_ids() {
        // Cite fs/ggml/type.go iota: F32=0, F16=1, Q8_0=7, Q4_K_S=14, Q4_K_M=15.
        assert_eq!(QuantType::from_file_type(0), Some(QuantType::F32));
        assert_eq!(QuantType::from_file_type(1), Some(QuantType::F16));
        assert_eq!(QuantType::from_file_type(7), Some(QuantType::Q8_0));
        assert_eq!(QuantType::from_file_type(15), Some(QuantType::Q4_K_M));
    }

    #[test]
    fn file_type_id_roundtrips() {
        assert_eq!(QuantType::Q4_K_M.file_type_id(), 15);
        assert_eq!(
            QuantType::from_file_type(QuantType::Q8_0.file_type_id()),
            Some(QuantType::Q8_0)
        );
    }

    #[test]
    fn from_file_type_unknown_is_none() {
        assert_eq!(QuantType::from_file_type(9999), None);
    }

    #[test]
    fn bits_per_weight_orders_by_precision() {
        // Lower-precision quants must use fewer bits/weight than higher ones.
        let q4 = QuantType::Q4_K_M.bits_per_weight();
        let q5 = QuantType::Q5_K_M.bits_per_weight();
        let q6 = QuantType::Q6_K.bits_per_weight();
        let q8 = QuantType::Q8_0.bits_per_weight();
        let f16 = QuantType::F16.bits_per_weight();
        assert!(q4 < q5, "q4<q5");
        assert!(q5 < q6, "q5<q6");
        assert!(q6 < q8, "q6<q8");
        assert!(q8 < f16, "q8<f16");
        assert_eq!(f16, 16.0);
    }

    #[test]
    fn estimate_bytes_for_7b_q4_k_m() {
        // 7B params at ~4.85 bits/weight ≈ 4.24 GiB. Allow a generous band.
        let bytes = QuantType::Q4_K_M.estimate_bytes(7_000_000_000);
        let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        assert!((3.8..4.7).contains(&gib), "got {gib} GiB ({bytes} bytes)");
    }

    #[test]
    fn estimate_bytes_q8_larger_than_q4() {
        let n = 1_000_000_000;
        assert!(QuantType::Q8_0.estimate_bytes(n) > QuantType::Q4_K_M.estimate_bytes(n));
    }
}
