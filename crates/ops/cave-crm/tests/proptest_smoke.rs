// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Generic property-test scaffold: input invariants, roundtrips, idempotency.
// Extend with crate-specific properties as the public API stabilises.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn string_byte_len_invariant(s in "\\PC{0,256}") {
        // chars().count() never exceeds bytes len for any UTF-8 string.
        prop_assert!(s.chars().count() <= s.len());
        // Cloning preserves equality (Eq is reflexive).
        prop_assert_eq!(s.clone(), s);
    }

    #[test]
    fn hex_byte_roundtrip(v in proptest::collection::vec(any::<u8>(), 0..128)) {
        let encoded: String = v.iter().map(|b| format!("{:02x}", b)).collect();
        let mut decoded = Vec::with_capacity(v.len());
        for chunk in encoded.as_bytes().chunks(2) {
            let s = std::str::from_utf8(chunk).unwrap();
            decoded.push(u8::from_str_radix(s, 16).unwrap());
        }
        prop_assert_eq!(v, decoded);
    }

    #[test]
    fn sort_idempotent(mut v in proptest::collection::vec(any::<i32>(), 0..64)) {
        v.sort();
        let snapshot = v.clone();
        v.sort();
        prop_assert_eq!(snapshot, v);
    }

    #[test]
    fn vec_reverse_involution(v in proptest::collection::vec(any::<i32>(), 0..32)) {
        // Reversing twice returns the original.
        let mut twice = v.clone();
        twice.reverse();
        twice.reverse();
        prop_assert_eq!(v, twice);
    }
}
