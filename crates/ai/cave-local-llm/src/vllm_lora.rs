// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-LoRA serving manager — a pure-Rust port of vLLM's LoRA subsystem
//! (vllm-project/vllm `vllm/lora/`, Apache-2.0).
//!
//! vLLM serves many LoRA adapters over one base model by keeping a fixed pool
//! of `max_loras` GPU adapter slots; requests reference an adapter by id, and
//! an LRU policy evicts the coldest adapter when the pool is full. A LoRA
//! adapter is a low-rank update `ΔW = scaling · B·A` (with `scaling =
//! lora_alpha / rank`), so the per-token forward delta is `scaling · B (A x)`.
//!
//! This ports the request scaling, rank-bound registration, the slot pool +
//! LRU eviction policy, and the low-rank forward delta — the bookkeeping and
//! math, independent of any GPU kernel.

use std::collections::{HashMap, VecDeque};

use thiserror::Error;

/// A LoRA adapter reference (vLLM `LoRARequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct LoRARequest {
    /// Human-readable adapter name.
    pub name: String,
    /// Unique integer id (slot key).
    pub id: u64,
    /// Low-rank dimension `r`.
    pub rank: usize,
    /// LoRA alpha (scaling numerator).
    pub alpha: f32,
}

impl LoRARequest {
    /// LoRA scaling factor `alpha / rank`.
    pub fn scaling(&self) -> f32 {
        self.alpha / self.rank as f32
    }
}

/// Static LoRA serving configuration.
#[derive(Debug, Clone)]
pub struct LoRAConfig {
    /// Number of GPU adapter slots.
    pub max_loras: usize,
    /// Maximum permitted adapter rank.
    pub max_lora_rank: usize,
}

/// LoRA manager errors.
#[derive(Debug, Error, PartialEq)]
pub enum LoRAError {
    /// Adapter rank exceeds `max_lora_rank`.
    #[error("LoRA rank {rank} exceeds max_lora_rank {max}")]
    RankExceeded {
        /// Requested rank.
        rank: usize,
        /// Configured maximum.
        max: usize,
    },
    /// Activation referenced an unregistered adapter id.
    #[error("LoRA {0} is not registered")]
    NotRegistered(u64),
}

/// Result of activating an adapter into the slot pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateOutcome {
    /// Slot index the adapter now occupies.
    pub slot: usize,
    /// Adapter id evicted to make room, if any.
    pub evicted: Option<u64>,
}

/// Fixed-pool multi-LoRA manager with LRU eviction.
#[derive(Debug)]
pub struct LoRAManager {
    config: LoRAConfig,
    registry: HashMap<u64, LoRARequest>,
    slots: Vec<Option<u64>>,
    /// LRU order: front = least recently used, back = most recently used.
    lru: VecDeque<u64>,
}

impl LoRAManager {
    /// New manager with `max_loras` empty slots.
    pub fn new(config: LoRAConfig) -> Self {
        let slots = vec![None; config.max_loras];
        Self {
            config,
            registry: HashMap::new(),
            slots,
            lru: VecDeque::new(),
        }
    }

    /// Register an adapter (rank-bound checked); does not load it into a slot.
    pub fn register(&mut self, req: LoRARequest) -> Result<(), LoRAError> {
        if req.rank > self.config.max_lora_rank {
            return Err(LoRAError::RankExceeded {
                rank: req.rank,
                max: self.config.max_lora_rank,
            });
        }
        self.registry.insert(req.id, req);
        Ok(())
    }

    /// Activate a registered adapter into a GPU slot, evicting the LRU
    /// adapter if the pool is full. Reactivating an already-loaded adapter
    /// just refreshes its LRU recency.
    pub fn activate(&mut self, id: u64) -> Result<ActivateOutcome, LoRAError> {
        if !self.registry.contains_key(&id) {
            return Err(LoRAError::NotRegistered(id));
        }
        // Already active: move to MRU, no eviction.
        if let Some(slot) = self.slots.iter().position(|s| *s == Some(id)) {
            self.touch_lru(id);
            return Ok(ActivateOutcome {
                slot,
                evicted: None,
            });
        }
        // Free slot available.
        if let Some(slot) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[slot] = Some(id);
            self.lru.push_back(id);
            return Ok(ActivateOutcome {
                slot,
                evicted: None,
            });
        }
        // Pool full: evict the least-recently-used adapter.
        let victim = self.lru.pop_front().expect("pool full implies non-empty lru");
        let slot = self
            .slots
            .iter()
            .position(|s| *s == Some(victim))
            .expect("lru victim must occupy a slot");
        self.slots[slot] = Some(id);
        self.lru.push_back(id);
        Ok(ActivateOutcome {
            slot,
            evicted: Some(victim),
        })
    }

    fn touch_lru(&mut self, id: u64) {
        if let Some(pos) = self.lru.iter().position(|&x| x == id) {
            self.lru.remove(pos);
        }
        self.lru.push_back(id);
    }

    /// True if `id` currently occupies a slot.
    pub fn is_active(&self, id: u64) -> bool {
        self.slots.iter().any(|s| *s == Some(id))
    }

    /// Number of occupied slots.
    pub fn num_active(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Ids of currently active adapters (slot order).
    pub fn active_ids(&self) -> Vec<u64> {
        self.slots.iter().filter_map(|s| *s).collect()
    }

    /// LoRA forward delta `scaling · B (A x)`.
    ///
    /// * `x` — input vector of length `in`.
    /// * `a` — down-projection `[rank][in]`.
    /// * `b` — up-projection `[out][rank]`.
    /// Returns the length-`out` delta added to the base layer output.
    pub fn lora_forward_delta(x: &[f32], a: &[Vec<f32>], b: &[Vec<f32>], scaling: f32) -> Vec<f32> {
        // t = A x  (length rank)
        let t: Vec<f32> = a
            .iter()
            .map(|row| row.iter().zip(x).map(|(w, xi)| w * xi).sum())
            .collect();
        // y = scaling * (B t)  (length out)
        b.iter()
            .map(|row| scaling * row.iter().zip(&t).map(|(w, ti)| w * ti).sum::<f32>())
            .collect()
    }
}
