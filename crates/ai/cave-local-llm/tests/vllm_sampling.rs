// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's SamplingParams (vllm-project/vllm
// `vllm/sampling_params.py`, Apache-2.0): the request-level sampling
// contract (temperature / top_p / top_k / min_p / penalties / n / best_of /
// stop / logprobs), its `_verify_args` validation, greedy-vs-random sampling
// classification, and the OpenAI-compat request -> SamplingParams mapping.

use cave_local_llm::vllm_sampling::{OpenAiSampling, SamplingError, SamplingParams, SamplingType};

#[test]
fn default_params_are_valid_random_sampling() {
    let p = SamplingParams::default();
    assert_eq!(p.temperature, 1.0);
    assert_eq!(p.top_p, 1.0);
    assert_eq!(p.top_k, -1);
    assert_eq!(p.min_p, 0.0);
    assert_eq!(p.n, 1);
    assert_eq!(p.best_of, 1);
    assert_eq!(p.repetition_penalty, 1.0);
    p.validate().expect("defaults must validate");
    assert_eq!(p.sampling_type(), SamplingType::Random);
}

#[test]
fn zero_temperature_is_greedy() {
    let p = SamplingParams {
        temperature: 0.0,
        ..Default::default()
    };
    assert_eq!(p.sampling_type(), SamplingType::Greedy);
}

#[test]
fn seeded_sampling_is_random_seed() {
    let p = SamplingParams {
        seed: Some(42),
        ..Default::default()
    };
    assert_eq!(p.sampling_type(), SamplingType::RandomSeed);
}

#[test]
fn greedy_with_best_of_gt_one_is_rejected() {
    let p = SamplingParams {
        temperature: 0.0,
        best_of: 2,
        n: 1,
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::GreedyBestOf)));
}

#[test]
fn negative_temperature_rejected() {
    let p = SamplingParams {
        temperature: -0.5,
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::Temperature(_))));
}

#[test]
fn top_p_out_of_range_rejected() {
    for bad in [0.0_f32, 1.5_f32] {
        let p = SamplingParams {
            top_p: bad,
            ..Default::default()
        };
        assert!(matches!(p.validate(), Err(SamplingError::TopP(_))), "top_p {bad}");
    }
}

#[test]
fn top_k_zero_rejected_but_minus_one_disables() {
    let bad = SamplingParams {
        top_k: 0,
        ..Default::default()
    };
    assert!(matches!(bad.validate(), Err(SamplingError::TopK(_))));
    let ok = SamplingParams {
        top_k: -1,
        ..Default::default()
    };
    ok.validate().expect("top_k = -1 disables the filter");
    let ok2 = SamplingParams {
        top_k: 50,
        ..Default::default()
    };
    ok2.validate().expect("top_k >= 1 is valid");
}

#[test]
fn min_p_out_of_range_rejected() {
    let p = SamplingParams {
        min_p: 1.2,
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::MinP(_))));
}

#[test]
fn presence_and_frequency_penalty_bounds_enforced() {
    let p = SamplingParams {
        presence_penalty: 3.0,
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::PresencePenalty(_))));
    let q = SamplingParams {
        frequency_penalty: -2.5,
        ..Default::default()
    };
    assert!(matches!(q.validate(), Err(SamplingError::FrequencyPenalty(_))));
}

#[test]
fn repetition_penalty_must_be_positive() {
    let p = SamplingParams {
        repetition_penalty: 0.0,
        ..Default::default()
    };
    assert!(matches!(
        p.validate(),
        Err(SamplingError::RepetitionPenalty(_))
    ));
}

#[test]
fn best_of_less_than_n_rejected() {
    let p = SamplingParams {
        n: 3,
        best_of: 2,
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::BestOf { .. })));
}

#[test]
fn min_tokens_greater_than_max_tokens_rejected() {
    let p = SamplingParams {
        min_tokens: 20,
        max_tokens: Some(10),
        ..Default::default()
    };
    assert!(matches!(p.validate(), Err(SamplingError::MinTokens { .. })));
}

#[test]
fn from_openai_maps_fields_and_defaults() {
    let req = OpenAiSampling {
        temperature: Some(0.7),
        top_p: Some(0.9),
        n: Some(2),
        max_tokens: Some(128),
        presence_penalty: Some(0.5),
        frequency_penalty: Some(0.25),
        stop: vec!["\n\n".to_string()],
        ..Default::default()
    };
    let p = SamplingParams::from_openai(req).expect("valid request");
    assert_eq!(p.temperature, 0.7);
    assert_eq!(p.top_p, 0.9);
    assert_eq!(p.n, 2);
    assert_eq!(p.best_of, 2, "best_of defaults to n");
    assert_eq!(p.max_tokens, Some(128));
    assert_eq!(p.presence_penalty, 0.5);
    assert_eq!(p.frequency_penalty, 0.25);
    assert_eq!(p.stop, vec!["\n\n".to_string()]);
}

#[test]
fn from_openai_propagates_validation_errors() {
    let req = OpenAiSampling {
        temperature: Some(-1.0),
        ..Default::default()
    };
    assert!(SamplingParams::from_openai(req).is_err());
}
