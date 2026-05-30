// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of vLLM's multi-LoRA serving manager
// (vllm-project/vllm `vllm/lora/`, Apache-2.0): LoRA request scaling
// (alpha/rank), rank-bound registration, a fixed pool of `max_loras` GPU
// slots with LRU eviction, and the LoRA forward delta scaling * (B (A x)).

use cave_local_llm::vllm_lora::{LoRAConfig, LoRAError, LoRAManager, LoRARequest};

fn req(name: &str, id: u64, rank: usize, alpha: f32) -> LoRARequest {
    LoRARequest {
        name: name.to_string(),
        id,
        rank,
        alpha,
    }
}

#[test]
fn scaling_is_alpha_over_rank() {
    let r = req("a", 1, 8, 16.0);
    assert_eq!(r.scaling(), 2.0);
}

#[test]
fn register_rejects_rank_over_max() {
    let mut m = LoRAManager::new(LoRAConfig {
        max_loras: 2,
        max_lora_rank: 16,
    });
    assert!(matches!(
        m.register(req("big", 1, 32, 16.0)),
        Err(LoRAError::RankExceeded { .. })
    ));
    m.register(req("ok", 2, 16, 16.0)).unwrap();
}

#[test]
fn activate_loads_into_free_slot() {
    let mut m = LoRAManager::new(LoRAConfig {
        max_loras: 2,
        max_lora_rank: 16,
    });
    m.register(req("a", 1, 8, 16.0)).unwrap();
    let outcome = m.activate(1).unwrap();
    assert_eq!(outcome.evicted, None);
    assert!(m.is_active(1));
    assert_eq!(m.num_active(), 1);
}

#[test]
fn activate_unregistered_lora_errors() {
    let mut m = LoRAManager::new(LoRAConfig {
        max_loras: 2,
        max_lora_rank: 16,
    });
    assert!(matches!(m.activate(99), Err(LoRAError::NotRegistered(99))));
}

#[test]
fn full_pool_evicts_least_recently_used() {
    let mut m = LoRAManager::new(LoRAConfig {
        max_loras: 2,
        max_lora_rank: 16,
    });
    for id in 1..=3 {
        m.register(req("l", id, 8, 16.0)).unwrap();
    }
    m.activate(1).unwrap();
    m.activate(2).unwrap();
    // Pool full (1,2). Activating 3 evicts the LRU (1).
    let outcome = m.activate(3).unwrap();
    assert_eq!(outcome.evicted, Some(1));
    assert!(!m.is_active(1));
    assert!(m.is_active(2) && m.is_active(3));
    assert_eq!(m.num_active(), 2);
}

#[test]
fn reactivating_active_lora_refreshes_lru_order() {
    let mut m = LoRAManager::new(LoRAConfig {
        max_loras: 2,
        max_lora_rank: 16,
    });
    for id in 1..=3 {
        m.register(req("l", id, 8, 16.0)).unwrap();
    }
    m.activate(1).unwrap();
    m.activate(2).unwrap();
    m.activate(1).unwrap(); // touch 1 -> now 2 is LRU
    let outcome = m.activate(3).unwrap();
    assert_eq!(outcome.evicted, Some(2), "2 became LRU after touching 1");
    assert!(m.is_active(1) && m.is_active(3));
}

#[test]
fn lora_forward_delta_applies_scaled_low_rank_update() {
    // y = scaling * (B (A x)); A = [[1,0],[0,1]] (rank2,in2), B=[[1,1],[2,0]]
    // (out2,rank2), x=[1,2], scaling=0.5.
    //   A x = [1, 2];  B (A x) = [3, 2];  * 0.5 = [1.5, 1.0].
    let a = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
    let b = vec![vec![1.0, 1.0], vec![2.0, 0.0]];
    let x = vec![1.0, 2.0];
    let y = LoRAManager::lora_forward_delta(&x, &a, &b, 0.5);
    assert_eq!(y, vec![1.5, 1.0]);
}
