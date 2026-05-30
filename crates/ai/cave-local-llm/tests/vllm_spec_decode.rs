// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's speculative-decoding rejection sampler
// (vllm-project/vllm `vllm/model_executor/layers/rejection_sampler.py` +
// `vllm/spec_decode/`, Apache-2.0): the modified rejection-sampling
// acceptance test (accept iff u <= min(1, p/q)), the normalized-residual
// recovery token on first rejection, the bonus token when all k drafts are
// accepted, and the running acceptance-rate metric.
//
// Randomness is injected (`uniforms`) so the algorithm is deterministic and
// testable without an RNG.

use cave_local_llm::vllm_spec_decode::{AcceptanceStats, RejectionSampler};

#[test]
fn all_drafts_accepted_emits_k_plus_bonus_token() {
    let s = RejectionSampler::new(3);
    let draft = [0u32, 1, 2];
    // Drafted-token proposal prob 0.5 each.
    let q = vec![
        vec![0.5, 0.25, 0.25],
        vec![0.25, 0.5, 0.25],
        vec![0.25, 0.25, 0.5],
    ];
    // Target assigns higher prob to each drafted token -> p/q >= 1 -> accept.
    let p = vec![
        vec![0.9, 0.05, 0.05],
        vec![0.05, 0.9, 0.05],
        vec![0.05, 0.05, 0.9],
        vec![0.1, 0.2, 0.7], // bonus position: argmax = token 2
    ];
    let u = [0.0, 0.0, 0.0];
    let r = s.sample(&draft, &q, &p, &u);
    assert_eq!(r.accepted, 3);
    assert!(r.all_accepted);
    assert_eq!(r.emitted, vec![0, 1, 2, 2], "k accepted + bonus argmax");
}

#[test]
fn high_probability_token_accepted_even_with_large_uniform() {
    let s = RejectionSampler::new(1);
    let draft = [0u32];
    let q = vec![vec![0.3, 0.7]];
    let p = vec![vec![0.6, 0.4], vec![0.8, 0.2]]; // p/q = 2 -> clamps to 1
    let u = [0.99];
    let r = s.sample(&draft, &q, &p, &u);
    assert_eq!(r.accepted, 1);
    assert!(r.all_accepted);
    assert_eq!(r.emitted.len(), 2); // accepted + bonus
}

#[test]
fn first_rejection_emits_normalized_residual_recovery_token() {
    let s = RejectionSampler::new(1);
    let draft = [0u32];
    let q = vec![vec![0.8, 0.1, 0.1]];
    // p/q for token 0 = 0.2/0.8 = 0.25; u = 0.9 > 0.25 -> reject.
    // residual = max(0, p - q) = [0, 0.6, 0] -> argmax index 1.
    let p = vec![vec![0.2, 0.7, 0.1], vec![0.5, 0.3, 0.2]];
    let u = [0.9];
    let r = s.sample(&draft, &q, &p, &u);
    assert_eq!(r.accepted, 0);
    assert!(!r.all_accepted);
    assert_eq!(r.emitted, vec![1], "recovery token from residual argmax");
}

#[test]
fn partial_acceptance_stops_at_first_rejection() {
    let s = RejectionSampler::new(2);
    let draft = [0u32, 1];
    let q = vec![vec![0.4, 0.6], vec![0.2, 0.8]];
    // pos0: p0=0.8 >= q0=0.4 -> accept (u small).
    // pos1: p(token1)=0.1 < q=0.8 -> p/q=0.125, u=0.9 -> reject.
    //       residual = max(0, [0.9,0.1]-[0.2,0.8]) = [0.7, 0] -> argmax 0.
    let p = vec![vec![0.8, 0.2], vec![0.9, 0.1], vec![0.3, 0.7]];
    let u = [0.1, 0.9];
    let r = s.sample(&draft, &q, &p, &u);
    assert_eq!(r.accepted, 1);
    assert!(!r.all_accepted);
    assert_eq!(r.emitted, vec![0, 0], "first draft + recovery, no bonus");
}

#[test]
fn acceptance_stats_track_rate() {
    let mut stats = AcceptanceStats::default();
    let s = RejectionSampler::new(3);

    // First call: all 3 accepted.
    let q = vec![vec![0.5, 0.5]; 3];
    let p_all = vec![vec![0.9, 0.1], vec![0.9, 0.1], vec![0.9, 0.1], vec![0.6, 0.4]];
    let r1 = s.sample(&[0, 0, 0], &q, &p_all, &[0.0, 0.0, 0.0]);
    stats.record(&r1);

    // Second call: reject immediately (0 accepted).
    let p_rej = vec![vec![0.1, 0.9], vec![0.5, 0.5]];
    let s1 = RejectionSampler::new(1);
    let r2 = s1.sample(&[0], &vec![vec![0.9, 0.1]], &p_rej, &[0.99]);
    stats.record(&r2);

    // 3 accepted out of 4 proposed.
    assert_eq!(stats.proposed(), 4);
    assert_eq!(stats.accepted(), 3);
    assert!((stats.acceptance_rate() - 0.75).abs() < 1e-6);
}
