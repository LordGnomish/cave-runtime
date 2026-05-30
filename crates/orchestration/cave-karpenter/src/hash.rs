// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Drift-detection hash — port of `NodePool.Hash()` in
//! `pkg/apis/v1/nodepool.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha `ed490e8`) and the slice of `mitchellh/hashstructure` (v2,
//! `FormatV2`) it depends on.
//!
//! Upstream:
//! ```go
//! func (in *NodePool) Hash() string {
//!     return fmt.Sprint(lo.Must(hashstructure.Hash([]interface{}{
//!         in.Spec.Template.Spec,
//!         in.Spec.Template.Labels,
//!         in.Spec.Template.Annotations,
//!     }, hashstructure.FormatV2, &hashstructure.HashOptions{
//!         SlicesAsSets:    true,
//!         IgnoreZeroValue: true,
//!         ZeroNil:         true,
//!     })))
//! }
//! ```
//!
//! The drift controller stores this string in the
//! `karpenter.sh/nodepool-hash` annotation; a NodeClaim whose recorded hash
//! no longer matches its pool's current hash has drifted and is eligible for
//! disruption.
//!
//! Byte-for-byte equivalence with the Go output is unreachable — Go hashes via
//! reflection over its own structs, whereas we hash the serde-JSON projection
//! of the Rust structs. What we port faithfully is the **combination
//! structure** of `hashstructure`'s `FormatV2`, which is what gives the hash
//! its drift-detection semantics:
//!
//!   * primitives → FNV-1 (64-bit, matching Go `hash/fnv.New64`);
//!   * structs/maps → each field hashed as `ordered(key, value)`, the field
//!     hashes XOR-combined (`unordered`, so field order is irrelevant), then
//!     run once more through FNV (`finish_unordered`);
//!   * slices → with `SlicesAsSets` the element hashes are XOR-combined
//!     (order-independent set semantics); otherwise folded in order;
//!   * `IgnoreZeroValue` drops empty/zero fields so adding an unset field is a
//!     no-op; `ZeroNil` makes a JSON `null` hash as the zero value.
//!
//! No external crates: FNV is hand-rolled, consistent with the rest of the
//! crate (regex-free, dependency-light).

use crate::models::NodePool;
use serde_json::Value;
use std::collections::HashSet;

// ── FNV-1 (64-bit) — same constants as Go `hash/fnv.New64` ───────────────────
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv1_step(hash: u64, byte: u8) -> u64 {
    // FNV-1 (not 1a): multiply, then XOR — the order Go's New64 uses.
    hash.wrapping_mul(FNV_PRIME) ^ (byte as u64)
}

fn fnv1(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET_BASIS;
    for &b in bytes {
        h = fnv1_step(h, b);
    }
    h
}

/// Ordered combine of two sub-hashes (`hashstructure.hashUpdateOrdered`):
/// feed the little-endian bytes of `a` then `b` through a fresh FNV.
fn ordered(a: u64, b: u64) -> u64 {
    let mut h = FNV_OFFSET_BASIS;
    for byte in a.to_le_bytes() {
        h = fnv1_step(h, byte);
    }
    for byte in b.to_le_bytes() {
        h = fnv1_step(h, byte);
    }
    h
}

/// Unordered combine (`hashstructure.hashUpdateUnordered`): XOR, so the
/// accumulation is commutative and order-independent.
#[inline]
fn unordered(a: u64, b: u64) -> u64 {
    a ^ b
}

/// `FormatV2` re-runs the XOR accumulator through FNV once more so that an
/// empty (zero) accumulator does not collapse to a fixed value collision-prone
/// across nesting levels.
fn finish_unordered(acc: u64) -> u64 {
    let mut h = FNV_OFFSET_BASIS;
    for byte in acc.to_le_bytes() {
        h = fnv1_step(h, byte);
    }
    h
}

/// Knobs mirroring `hashstructure.HashOptions` (the subset Karpenter sets).
#[derive(Debug, Clone, Default)]
pub struct HashOptions {
    /// Treat slices as unordered sets (Karpenter sets this `true`).
    pub slices_as_sets: bool,
    /// Skip zero-valued struct/map fields (Karpenter sets this `true`).
    pub ignore_zero_value: bool,
    /// Hash a `null` as the zero value rather than a distinct sentinel
    /// (Karpenter sets this `true`).
    pub zero_nil: bool,
    /// Field names to drop entirely — models `hash:"ignore"` struct tags.
    pub ignore_keys: HashSet<String>,
}

impl HashOptions {
    /// The exact option set `NodePool.Hash()` passes to `hashstructure`.
    pub fn format_v2() -> Self {
        Self {
            slices_as_sets: true,
            ignore_zero_value: true,
            zero_nil: true,
            ignore_keys: HashSet::new(),
        }
    }
}

/// True when `v` is the zero value of its JSON type — the predicate
/// `IgnoreZeroValue` / `ZeroNil` gate on.
fn is_zero(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// Hash an arbitrary JSON value under `FormatV2` semantics.
pub fn hash_value(v: &Value, opts: &HashOptions) -> u64 {
    visit(v, opts)
}

fn visit(v: &Value, opts: &HashOptions) -> u64 {
    match v {
        Value::Null => {
            if opts.zero_nil {
                // Zero value: hash the empty byte string.
                fnv1(&[])
            } else {
                fnv1(b"\x00null")
            }
        }
        Value::Bool(b) => fnv1(&[b'b', *b as u8]),
        Value::String(s) => {
            // Tag-prefix so a string never collides with a bool/number that
            // happens to share a byte pattern.
            let mut bytes = Vec::with_capacity(s.len() + 1);
            bytes.push(b's');
            bytes.extend_from_slice(s.as_bytes());
            fnv1(&bytes)
        }
        Value::Number(n) => {
            let mut bytes = Vec::with_capacity(9);
            if let Some(i) = n.as_i64() {
                bytes.push(b'i');
                bytes.extend_from_slice(&i.to_le_bytes());
            } else if let Some(u) = n.as_u64() {
                bytes.push(b'u');
                bytes.extend_from_slice(&u.to_le_bytes());
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                bytes.push(b'f');
                bytes.extend_from_slice(&f.to_bits().to_le_bytes());
            }
            fnv1(&bytes)
        }
        Value::Array(arr) => {
            if opts.slices_as_sets {
                let mut acc: u64 = 0;
                for elem in arr {
                    acc = unordered(acc, visit(elem, opts));
                }
                finish_unordered(acc)
            } else {
                // Ordered fold seeded by a length-distinct base so `[a]` and
                // `[[a]]` cannot collide.
                let mut acc = fnv1(b"[");
                for elem in arr {
                    acc = ordered(acc, visit(elem, opts));
                }
                acc
            }
        }
        Value::Object(map) => {
            let mut acc: u64 = 0;
            for (key, val) in map {
                if opts.ignore_zero_value && is_zero(val) {
                    continue;
                }
                if opts.ignore_keys.contains(key) {
                    continue;
                }
                let key_hash = visit(&Value::String(key.clone()), opts);
                let val_hash = visit(val, opts);
                acc = unordered(acc, ordered(key_hash, val_hash));
            }
            finish_unordered(acc)
        }
    }
}

/// `NodePool.Hash()` — the drift hash over `[template.spec, template.labels,
/// template.annotations]`, rendered as a decimal string exactly like Go's
/// `fmt.Sprint(uint64)`.
pub fn nodepool_hash(pool: &NodePool) -> String {
    let spec = serde_json::to_value(&pool.template.spec).unwrap_or(Value::Null);
    let labels = serde_json::to_value(&pool.template.labels).unwrap_or(Value::Null);
    let annotations = serde_json::to_value(&pool.template.annotations).unwrap_or(Value::Null);
    let envelope = Value::Array(vec![spec, labels, annotations]);
    hash_value(&envelope, &HashOptions::format_v2()).to_string()
}
