// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's TypicalAcceptanceSampler (vllm-project/vllm
// `vllm/model_executor/layers/typical_acceptance_sampler.py`, Apache-2.0):
// an alternative to modified rejection sampling that accepts a drafted token
// iff the target assigns it more than an entropy-adaptive threshold
//
//     threshold = min(posterior_threshold, posterior_alpha · exp(-entropy))
//
// where `entropy` is the Shannon entropy of the target distribution at that
// position. It needs no draft distribution and no uniform samples — purely a
// function of the target probabilities — so it is fully deterministic. On the
// first rejection it emits the target argmax (recovery); on full acceptance it
// appends a bonus token from the target's next-position row.

use cave_local_llm::vllm_spec_decode::{AcceptanceResult, TypicalAcceptanceSampler};

#[test]
fn defaults_are_0_09_threshold_and_sqrt_alpha() {
    let s = TypicalAcceptanceSampler::with_defaults(3);
    assert_eq!(s.num_speculative_tokens(), 3);
    assert!((s.posterior_threshold() - 0.09).abs() < 1e-6);
    assert!((s.posterior_alpha() - 0.3).abs() < 1e-6); // sqrt(0.09)
}

#[test]
fn accepts_high_probability_token_with_bonus() {
    let s = TypicalAcceptanceSampler::with_defaults(1);
    // pos 0 peaked on token 1; bonus row peaked on token 1.
    let target = vec![vec![0.1, 0.9], vec![0.2, 0.8]];
    let r = s.sample(&[1], &target);
    assert_eq!(
        r,
        AcceptanceResult {
            accepted: 1,
            emitted: vec![1, 1],
            all_accepted: true,
        }
    );
}

#[test]
fn rejects_atypical_token_and_recovers_argmax() {
    let s = TypicalAcceptanceSampler::with_defaults(1);
    // token 1 has prob 0.01 in a peaked distribution -> below the 0.09 floor.
    let target = vec![vec![0.97, 0.01, 0.01, 0.01]];
    let r = s.sample(&[1], &target);
    assert_eq!(
        r,
        AcceptanceResult {
            accepted: 0,
            emitted: vec![0], // argmax recovery
            all_accepted: false,
        }
    );
}

#[test]
fn accepts_prefix_then_rejects() {
    let s = TypicalAcceptanceSampler::with_defaults(2);
    // pos 0 accept (peak), pos 1 reject (atypical) -> recovery argmax = 0.
    let target = vec![
        vec![0.1, 0.9],
        vec![0.97, 0.01, 0.01, 0.01],
        vec![0.5, 0.5], // bonus row (unused on rejection)
    ];
    let r = s.sample(&[1, 1], &target);
    assert_eq!(r.accepted, 1);
    assert_eq!(r.emitted, vec![1, 0]);
    assert!(!r.all_accepted);
}

#[test]
fn all_accepted_appends_bonus_argmax() {
    let s = TypicalAcceptanceSampler::with_defaults(2);
    let target = vec![
        vec![0.1, 0.9],
        vec![0.85, 0.15],
        vec![0.3, 0.7], // bonus -> argmax = 1
    ];
    let r = s.sample(&[1, 0], &target);
    assert_eq!(r.accepted, 2);
    assert_eq!(r.emitted, vec![1, 0, 1]);
    assert!(r.all_accepted);
}
