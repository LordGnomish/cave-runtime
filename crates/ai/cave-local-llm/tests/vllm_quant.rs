// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's weight-quantization config metadata
// (vllm-project/vllm `vllm/model_executor/layers/quantization/{awq,gptq,fp8}.py`,
// Apache-2.0): AWQ / GPTQ / FP8 parameter validation, the int32 pack factor,
// per-group scale layout, packed-weight sizing and the fp16 compression ratio.

use cave_local_llm::vllm_quant::{
    ActivationScheme, Fp8Format, QuantConfig, QuantError, QuantMethod,
};

#[test]
fn awq_is_4bit_with_zero_point() {
    let c = QuantConfig::awq(4, 128).unwrap();
    assert_eq!(c.method, QuantMethod::Awq);
    assert_eq!(c.weight_bits, 4);
    assert_eq!(c.pack_factor(), 8); // 32 / 4
    assert!(c.zero_point);
}

#[test]
fn awq_rejects_non_4bit() {
    assert!(matches!(
        QuantConfig::awq(3, 128),
        Err(QuantError::UnsupportedBits { .. })
    ));
}

#[test]
fn gptq_supports_2_3_4_8_bits_only() {
    for b in [2, 3, 4, 8] {
        QuantConfig::gptq(b, 128, false, true).unwrap_or_else(|_| panic!("{b} bits valid"));
    }
    assert!(QuantConfig::gptq(5, 128, false, true).is_err());
}

#[test]
fn gptq_symmetric_drops_zero_point() {
    let sym = QuantConfig::gptq(4, 128, false, true).unwrap();
    assert!(!sym.zero_point);
    let asym = QuantConfig::gptq(4, 128, false, false).unwrap();
    assert!(asym.zero_point);
}

#[test]
fn pack_factor_is_32_over_bits() {
    assert_eq!(QuantConfig::gptq(4, 128, false, true).unwrap().pack_factor(), 8);
    assert_eq!(QuantConfig::gptq(8, -1, false, true).unwrap().pack_factor(), 4);
    assert_eq!(QuantConfig::gptq(2, 128, false, true).unwrap().pack_factor(), 16);
}

#[test]
fn invalid_group_size_rejected() {
    assert!(matches!(
        QuantConfig::gptq(4, 0, false, true),
        Err(QuantError::InvalidGroupSize(0))
    ));
    // -1 (per-channel) is valid.
    QuantConfig::gptq(4, -1, false, true).unwrap();
}

#[test]
fn num_groups_per_channel_vs_grouped() {
    let per_channel = QuantConfig::gptq(4, -1, false, true).unwrap();
    assert_eq!(per_channel.num_groups(4096), 1);
    let grouped = QuantConfig::gptq(4, 128, false, true).unwrap();
    assert_eq!(grouped.num_groups(4096), 32); // 4096 / 128
}

#[test]
fn validate_shape_requires_group_divisibility() {
    let c = QuantConfig::awq(4, 128).unwrap();
    assert!(matches!(
        c.validate_shape(100, 256),
        Err(QuantError::GroupMismatch { .. })
    ));
    c.validate_shape(4096, 4096).unwrap();
}

#[test]
fn packed_weight_words_matches_pack_factor() {
    let c = QuantConfig::awq(4, 128).unwrap();
    // qweight packs 8 4-bit weights per int32 -> in*out/8 words.
    assert_eq!(c.packed_weight_words(4096, 4096).unwrap(), 4096 * 4096 / 8);
}

#[test]
fn awq_4bit_compresses_to_about_a_quarter_of_fp16() {
    let c = QuantConfig::awq(4, 128).unwrap();
    let r = c.compression_ratio(4096, 4096);
    // 4-bit weights + fp16 group scales + packed zeros -> ~0.26 of fp16.
    assert!(r > 0.20 && r < 0.30, "awq ratio {r}");
}

#[test]
fn fp8_is_8bit_and_halves_fp16() {
    let c = QuantConfig::fp8(Fp8Format::E4M3, ActivationScheme::Dynamic);
    assert_eq!(c.method, QuantMethod::Fp8);
    assert_eq!(c.weight_bits, 8);
    assert_eq!(c.fp8_format, Some(Fp8Format::E4M3));
    assert_eq!(c.activation_scheme, Some(ActivationScheme::Dynamic));
    let r = c.compression_ratio(4096, 4096);
    assert!(r > 0.49 && r < 0.51, "fp8 ratio {r}");
}
