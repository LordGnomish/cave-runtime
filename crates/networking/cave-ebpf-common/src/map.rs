// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BPF map abstraction — userspace model of the cilium/ebpf map types that
//! grafana/beyla relies on (`BPF_MAP_TYPE_HASH`, `_LRU_HASH`, `_ARRAY`).
//!
//! The kernel exposes maps through the `bpf(2)` syscall; this is a pure
//! in-process model that reproduces the *observable semantics* — update
//! flags, capacity limits, LRU eviction, and the `errno` codes — without
//! the syscall. It is what Beyla's userspace event consumers see, and it
//! lets the rest of the crate (ringbuf, discover) be exercised in tests.

use std::collections::HashMap;
use std::hash::Hash;

/// Update flags mirroring the kernel's `BPF_ANY` / `BPF_NOEXIST` /
/// `BPF_EXIST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateFlag {
    /// Create or replace.
    Any,
    /// Create only; fail if the key already exists (`EEXIST`).
    NoExist,
    /// Replace only; fail if the key is absent (`ENOENT`).
    Exist,
}

/// Map operation errors, mapped to the kernel `errno` they model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// `EEXIST` — key present under `NoExist`.
    AlreadyExists,
    /// `ENOENT` — key absent under `Exist`, or delete of a missing key.
    NotFound,
    /// `E2BIG` — map at `max_entries` (non-LRU) or index out of bounds.
    TooBig,
}

/// `BPF_MAP_TYPE_HASH` / `BPF_MAP_TYPE_LRU_HASH`.
///
/// When `lru` is set, an insert into a full map evicts the
/// least-recently-used key (touched by `lookup` or `update`) rather than
/// failing with `E2BIG`.
#[derive(Debug, Clone)]
pub struct BpfHashMap<K, V> {
    entries: HashMap<K, V>,
    max_entries: usize,
    lru: bool,
    /// Recency clock; higher = more recently used.
    tick: u64,
    recency: HashMap<K, u64>,
}

impl<K: Eq + Hash + Clone, V> BpfHashMap<K, V> {
    pub fn new(max_entries: usize, lru: bool) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            lru,
            tick: 0,
            recency: HashMap::new(),
        }
    }

    fn touch(&mut self, k: &K) {
        self.tick += 1;
        self.recency.insert(k.clone(), self.tick);
    }

    pub fn update(&mut self, key: K, value: V, flag: UpdateFlag) -> Result<(), MapError> {
        let present = self.entries.contains_key(&key);
        match flag {
            UpdateFlag::NoExist if present => return Err(MapError::AlreadyExists),
            UpdateFlag::Exist if !present => return Err(MapError::NotFound),
            _ => {}
        }
        if !present && self.entries.len() >= self.max_entries {
            if self.lru {
                self.evict_lru();
            } else {
                return Err(MapError::TooBig);
            }
        }
        self.entries.insert(key.clone(), value);
        self.touch(&key);
        Ok(())
    }

    fn evict_lru(&mut self) {
        if let Some(victim) = self
            .recency
            .iter()
            .min_by_key(|&(_, &t)| t)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&victim);
            self.recency.remove(&victim);
        }
    }

    pub fn lookup(&mut self, key: &K) -> Option<&V> {
        if self.entries.contains_key(key) {
            self.touch(key);
            self.entries.get(key)
        } else {
            None
        }
    }

    pub fn delete(&mut self, key: &K) -> Result<(), MapError> {
        if self.entries.remove(key).is_some() {
            self.recency.remove(key);
            Ok(())
        } else {
            Err(MapError::NotFound)
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_entries
    }
}

/// `BPF_MAP_TYPE_ARRAY` — fixed-size, zero-initialised, index-keyed.
#[derive(Debug, Clone)]
pub struct BpfArray<V> {
    slots: Vec<V>,
}

impl<V: Default + Clone> BpfArray<V> {
    pub fn new(max_entries: usize) -> Self {
        Self {
            slots: vec![V::default(); max_entries],
        }
    }

    pub fn lookup(&self, index: usize) -> Option<&V> {
        self.slots.get(index)
    }

    pub fn update(&mut self, index: usize, value: V) -> Result<(), MapError> {
        match self.slots.get_mut(index) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(MapError::TooBig),
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
