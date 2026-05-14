//! Line-by-line ports of upstream Redis command tests, cross-
//! referenced from `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: redis/redis @ 7.4.0
//!   * src/t_string.c (+ tests/unit/type/string.tcl)
//!   * src/t_list.c   (+ tests/unit/type/list.tcl)
//!   * src/t_hash.c   (+ tests/unit/type/hash.tcl)
//!   * src/t_set.c    (+ tests/unit/type/set.tcl)
//!   * src/t_zset.c   (+ tests/unit/type/zset.tcl)
//!
//! Tcl test cases (Redis's primary test harness) ported to Rust
//! `#[test]` fns. Each asserts the same input → output equivalence
//! class as the upstream test.

use cave_cache::commands::hashes::{cmd_hdel, cmd_hget, cmd_hgetall, cmd_hset};
use cave_cache::commands::lists::{cmd_llen, cmd_lpop, cmd_lpush, cmd_lrange, cmd_rpush};
use cave_cache::commands::sets::{cmd_sadd, cmd_scard, cmd_sismember, cmd_smembers, cmd_srem};
use cave_cache::commands::sorted_sets::{cmd_zadd, cmd_zcard, cmd_zrank, cmd_zrem, cmd_zscore};
use cave_cache::commands::strings::{cmd_append, cmd_get, cmd_incr, cmd_set, cmd_strlen};
use cave_cache::db::Db;
use cave_cache::error::CacheError;
use cave_cache::resp::Resp;

fn b(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn args(parts: &[&str]) -> Vec<Vec<u8>> {
    parts.iter().map(|s| b(s)).collect()
}

fn bulk(s: &str) -> Resp {
    Resp::BulkString(Some(b(s)))
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: src/t_string.c + tests/unit/type/string.tcl
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: string.tcl / `SET and GET an item`.
/// Upstream expected: SET → "OK", GET → "Hello"
#[test]
fn upstream_string_set_then_get_round_trips() {
    let mut db = Db::new();
    let set = cmd_set(&args(&["set", "mykey", "Hello"]), &mut db).unwrap();
    assert_eq!(set, Resp::ok());
    let got = cmd_get(&args(&["get", "mykey"]), &mut db).unwrap();
    assert_eq!(got, bulk("Hello"));
}

/// Upstream: string.tcl / `GET against non-existing key`.
/// Upstream expected: nil bulk reply.
#[test]
fn upstream_string_get_missing_returns_nil() {
    let mut db = Db::new();
    let got = cmd_get(&args(&["get", "nope"]), &mut db).unwrap();
    assert!(got.is_nil(), "expected nil, got {got:?}");
}

/// Upstream: string.tcl / `SETNX target key missing → succeed; target exists → fail`.
/// Upstream expected: SET with NX returns OK iff key absent; returns nil
/// (or per-spec "did not set") when key already exists.
#[test]
fn upstream_string_set_nx_is_idempotent_under_collision() {
    let mut db = Db::new();
    let first = cmd_set(&args(&["set", "k", "v1", "NX"]), &mut db).unwrap();
    assert_eq!(first, Resp::ok());
    let second = cmd_set(&args(&["set", "k", "v2", "NX"]), &mut db).unwrap();
    assert!(second.is_nil(), "NX collision must not overwrite");
    let got = cmd_get(&args(&["get", "k"]), &mut db).unwrap();
    assert_eq!(got, bulk("v1"));
}

/// Upstream: string.tcl / `INCR against new key creates and returns 1`.
#[test]
fn upstream_string_incr_against_new_key_starts_at_one() {
    let mut db = Db::new();
    let r = cmd_incr(&args(&["incr", "counter"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(1));
    let r2 = cmd_incr(&args(&["incr", "counter"]), &mut db).unwrap();
    assert_eq!(r2, Resp::int(2));
}

/// Upstream: string.tcl / `APPEND extends the existing value`.
#[test]
fn upstream_string_append_extends_existing_value() {
    let mut db = Db::new();
    cmd_set(&args(&["set", "k", "Hello "]), &mut db).unwrap();
    let r = cmd_append(&args(&["append", "k", "World"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(11)); // "Hello World".len()
    let r = cmd_strlen(&args(&["strlen", "k"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(11));
    let got = cmd_get(&args(&["get", "k"]), &mut db).unwrap();
    assert_eq!(got, bulk("Hello World"));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: src/t_list.c + tests/unit/type/list.tcl
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: list.tcl / `RPUSH then LRANGE retrieves in insertion order`.
#[test]
fn upstream_list_rpush_lrange_preserves_insertion_order() {
    let mut db = Db::new();
    cmd_rpush(&args(&["rpush", "l", "a", "b", "c"]), &mut db).unwrap();
    let r = cmd_lrange(&args(&["lrange", "l", "0", "-1"]), &mut db).unwrap();
    assert_eq!(
        r,
        Resp::array(vec![bulk("a"), bulk("b"), bulk("c")])
    );
}

/// Upstream: list.tcl / `LPUSH prepends elements`.
/// Multiple elements LPUSHed in one call land in reverse order.
#[test]
fn upstream_list_lpush_prepends_in_reverse_order() {
    let mut db = Db::new();
    cmd_lpush(&args(&["lpush", "l", "a", "b", "c"]), &mut db).unwrap();
    let r = cmd_lrange(&args(&["lrange", "l", "0", "-1"]), &mut db).unwrap();
    assert_eq!(r, Resp::array(vec![bulk("c"), bulk("b"), bulk("a")]));
}

/// Upstream: list.tcl / `LLEN returns 0 for missing key, count for present`.
#[test]
fn upstream_list_llen_returns_zero_for_missing_key() {
    let mut db = Db::new();
    let r = cmd_llen(&args(&["llen", "nope"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(0));
    cmd_rpush(&args(&["rpush", "l", "a", "b"]), &mut db).unwrap();
    let r = cmd_llen(&args(&["llen", "l"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(2));
}

/// Upstream: list.tcl / `LPOP removes head element`.
#[test]
fn upstream_list_lpop_removes_head_element() {
    let mut db = Db::new();
    cmd_rpush(&args(&["rpush", "l", "a", "b", "c"]), &mut db).unwrap();
    let r = cmd_lpop(&args(&["lpop", "l"]), &mut db).unwrap();
    assert_eq!(r, bulk("a"));
    let r = cmd_lrange(&args(&["lrange", "l", "0", "-1"]), &mut db).unwrap();
    assert_eq!(r, Resp::array(vec![bulk("b"), bulk("c")]));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: src/t_hash.c + tests/unit/type/hash.tcl
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: hash.tcl / `HSET + HGET round trip`.
#[test]
fn upstream_hash_hset_then_hget_round_trips() {
    let mut db = Db::new();
    let r = cmd_hset(&args(&["hset", "h", "f1", "v1"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(1)); // 1 new field
    let r = cmd_hget(&args(&["hget", "h", "f1"]), &mut db).unwrap();
    assert_eq!(r, bulk("v1"));
}

/// Upstream: hash.tcl / `HSET updating existing field returns 0`.
/// Upstream contract: return value = NEW fields added, not "modified".
#[test]
fn upstream_hash_hset_overwrite_returns_zero_new_fields() {
    let mut db = Db::new();
    cmd_hset(&args(&["hset", "h", "f1", "v1"]), &mut db).unwrap();
    let r = cmd_hset(&args(&["hset", "h", "f1", "v2"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(0)); // no NEW fields
    let r = cmd_hget(&args(&["hget", "h", "f1"]), &mut db).unwrap();
    assert_eq!(r, bulk("v2"));
}

/// Upstream: hash.tcl / `HGETALL returns interleaved field-value pairs`.
#[test]
fn upstream_hash_hgetall_returns_field_value_pairs() {
    let mut db = Db::new();
    cmd_hset(&args(&["hset", "h", "f1", "v1", "f2", "v2"]), &mut db).unwrap();
    let r = cmd_hgetall(&args(&["hgetall", "h"]), &mut db).unwrap();
    let arr = match r {
        Resp::Array(Some(items)) => items,
        other => panic!("expected array, got {other:?}"),
    };
    assert_eq!(arr.len(), 4); // 2 fields × (key + value)
    // The order within HGETALL is hash-table dependent (matching Redis),
    // so assert each pair is present.
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = arr
        .chunks(2)
        .map(|c| match (&c[0], &c[1]) {
            (Resp::BulkString(Some(k)), Resp::BulkString(Some(v))) => (k.clone(), v.clone()),
            _ => panic!("expected bulk pair"),
        })
        .collect();
    assert!(pairs.contains(&(b("f1"), b("v1"))));
    assert!(pairs.contains(&(b("f2"), b("v2"))));
}

/// Upstream: hash.tcl / `HDEL removes specified fields`.
#[test]
fn upstream_hash_hdel_removes_field_and_returns_count() {
    let mut db = Db::new();
    cmd_hset(&args(&["hset", "h", "f1", "v1", "f2", "v2"]), &mut db).unwrap();
    let r = cmd_hdel(&args(&["hdel", "h", "f1"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(1));
    let r = cmd_hget(&args(&["hget", "h", "f1"]), &mut db).unwrap();
    assert!(r.is_nil());
    let r = cmd_hget(&args(&["hget", "h", "f2"]), &mut db).unwrap();
    assert_eq!(r, bulk("v2"));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: src/t_set.c + tests/unit/type/set.tcl
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: set.tcl / `SADD adds members and SCARD returns the set size`.
#[test]
fn upstream_set_sadd_then_scard_returns_size() {
    let mut db = Db::new();
    let r = cmd_sadd(&args(&["sadd", "s", "a", "b", "c"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(3));
    let r = cmd_scard(&args(&["scard", "s"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(3));
    // SADD again with one new + one duplicate → only 1 new.
    let r = cmd_sadd(&args(&["sadd", "s", "c", "d"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(1));
}

/// Upstream: set.tcl / `SISMEMBER returns 1 for present member, 0 otherwise`.
#[test]
fn upstream_set_sismember_returns_membership_bool_as_int() {
    let mut db = Db::new();
    cmd_sadd(&args(&["sadd", "s", "a", "b"]), &mut db).unwrap();
    assert_eq!(
        cmd_sismember(&args(&["sismember", "s", "a"]), &mut db).unwrap(),
        Resp::int(1)
    );
    assert_eq!(
        cmd_sismember(&args(&["sismember", "s", "x"]), &mut db).unwrap(),
        Resp::int(0)
    );
}

/// Upstream: set.tcl / `SREM removes member and returns removed count`.
#[test]
fn upstream_set_srem_removes_member_returns_count() {
    let mut db = Db::new();
    cmd_sadd(&args(&["sadd", "s", "a", "b", "c"]), &mut db).unwrap();
    let r = cmd_srem(&args(&["srem", "s", "a", "x", "b"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(2));
    let r = cmd_smembers(&args(&["smembers", "s"]), &mut db).unwrap();
    match r {
        Resp::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], bulk("c"));
        }
        other => panic!("expected array, got {other:?}"),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: src/t_zset.c + tests/unit/type/zset.tcl
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: zset.tcl / `ZADD increments cardinality and returns added count`.
#[test]
fn upstream_zset_zadd_then_zcard_returns_size() {
    let mut db = Db::new();
    let r = cmd_zadd(&args(&["zadd", "z", "1", "a", "2", "b", "3", "c"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(3));
    let r = cmd_zcard(&args(&["zcard", "z"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(3));
}

/// Upstream: zset.tcl / `ZSCORE returns the score of a present member, nil otherwise`.
#[test]
fn upstream_zset_zscore_returns_score_or_nil() {
    let mut db = Db::new();
    cmd_zadd(&args(&["zadd", "z", "3.14", "pi"]), &mut db).unwrap();
    let r = cmd_zscore(&args(&["zscore", "z", "pi"]), &mut db).unwrap();
    let bulk_value = match r {
        Resp::BulkString(Some(v)) => v,
        other => panic!("expected bulk string, got {other:?}"),
    };
    let s = std::str::from_utf8(&bulk_value).unwrap();
    assert!(s.starts_with("3.14"), "got {s}");
    // Missing member → nil bulk.
    let r = cmd_zscore(&args(&["zscore", "z", "missing"]), &mut db).unwrap();
    assert!(r.is_nil());
}

/// Upstream: zset.tcl / `ZRANK gives 0-indexed position by score ascending`.
#[test]
fn upstream_zset_zrank_returns_zero_indexed_position() {
    let mut db = Db::new();
    cmd_zadd(&args(&["zadd", "z", "1", "a", "2", "b", "3", "c"]), &mut db).unwrap();
    assert_eq!(cmd_zrank(&args(&["zrank", "z", "a"]), &mut db).unwrap(), Resp::int(0));
    assert_eq!(cmd_zrank(&args(&["zrank", "z", "b"]), &mut db).unwrap(), Resp::int(1));
    assert_eq!(cmd_zrank(&args(&["zrank", "z", "c"]), &mut db).unwrap(), Resp::int(2));
    // Missing member → nil.
    let r = cmd_zrank(&args(&["zrank", "z", "missing"]), &mut db).unwrap();
    assert!(r.is_nil());
}

/// Upstream: zset.tcl / `ZREM removes member and updates cardinality`.
#[test]
fn upstream_zset_zrem_removes_and_decrements_cardinality() {
    let mut db = Db::new();
    cmd_zadd(&args(&["zadd", "z", "1", "a", "2", "b"]), &mut db).unwrap();
    let r = cmd_zrem(&args(&["zrem", "z", "a", "nope"]), &mut db).unwrap();
    assert_eq!(r, Resp::int(1));
    assert_eq!(cmd_zcard(&args(&["zcard", "z"]), &mut db).unwrap(), Resp::int(1));
}

// ────────────────────────────────────────────────────────────────────────────
// Cross-type WRONGTYPE error
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: tests/unit/type/string.tcl / `WRONGTYPE when GET against a list`.
/// Redis errors all hash-collision commands across types with WRONGTYPE.
#[test]
fn upstream_wrongtype_when_get_targets_list_key() {
    let mut db = Db::new();
    cmd_rpush(&args(&["rpush", "k", "a"]), &mut db).unwrap();
    let err = cmd_get(&args(&["get", "k"]), &mut db).unwrap_err();
    assert!(matches!(err, CacheError::WrongType));
}
