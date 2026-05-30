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
