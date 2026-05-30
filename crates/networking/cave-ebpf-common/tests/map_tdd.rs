// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — BPF map abstraction (userspace model of the
// cilium/ebpf map types Beyla uses: BPF_MAP_TYPE_HASH, _LRU_HASH,
// _ARRAY). Mirrors the kernel update-flag semantics (BPF_ANY /
// BPF_NOEXIST / BPF_EXIST) and the E2BIG / EEXIST / ENOENT error codes.

use cave_ebpf_common::map::{BpfArray, BpfHashMap, MapError, UpdateFlag};

#[test]
fn test_hash_update_lookup_delete() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(8, false);
    m.update(1, 100, UpdateFlag::Any).unwrap();
    assert_eq!(m.lookup(&1), Some(&100));
    assert_eq!(m.len(), 1);
    m.delete(&1).unwrap();
    assert_eq!(m.lookup(&1), None);
    assert_eq!(m.len(), 0);
}

#[test]
fn test_noexist_flag_rejects_existing_key() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(8, false);
    m.update(1, 100, UpdateFlag::Any).unwrap();
    assert_eq!(
        m.update(1, 200, UpdateFlag::NoExist),
        Err(MapError::AlreadyExists)
    );
    assert_eq!(m.lookup(&1), Some(&100)); // unchanged
}

#[test]
fn test_exist_flag_requires_present_key() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(8, false);
    assert_eq!(
        m.update(1, 100, UpdateFlag::Exist),
        Err(MapError::NotFound)
    );
}

#[test]
fn test_hash_full_returns_e2big() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(2, false);
    m.update(1, 1, UpdateFlag::Any).unwrap();
    m.update(2, 2, UpdateFlag::Any).unwrap();
    assert_eq!(m.update(3, 3, UpdateFlag::Any), Err(MapError::TooBig));
    // Updating an existing key when full is allowed.
    m.update(1, 11, UpdateFlag::Any).unwrap();
    assert_eq!(m.lookup(&1), Some(&11));
}

#[test]
fn test_lru_evicts_least_recently_used_when_full() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(2, true);
    m.update(1, 1, UpdateFlag::Any).unwrap();
    m.update(2, 2, UpdateFlag::Any).unwrap();
    // Touch key 1 so key 2 becomes the LRU victim.
    assert_eq!(m.lookup(&1), Some(&1));
    m.update(3, 3, UpdateFlag::Any).unwrap(); // evicts 2
    assert_eq!(m.lookup(&2), None);
    assert_eq!(m.lookup(&1), Some(&1));
    assert_eq!(m.lookup(&3), Some(&3));
    assert_eq!(m.len(), 2);
}

#[test]
fn test_delete_missing_key_is_enoent() {
    let mut m: BpfHashMap<u32, u64> = BpfHashMap::new(4, false);
    assert_eq!(m.delete(&9), Err(MapError::NotFound));
}

#[test]
fn test_array_is_zero_initialised_and_bounds_checked() {
    let mut a: BpfArray<u64> = BpfArray::new(4);
    assert_eq!(a.lookup(0), Some(&0));
    assert_eq!(a.lookup(3), Some(&0));
    assert_eq!(a.lookup(4), None);
    a.update(2, 42).unwrap();
    assert_eq!(a.lookup(2), Some(&42));
    assert_eq!(a.update(4, 1), Err(MapError::TooBig));
}
