//! Integration tests for cave-cache.
//!
//! Tests cover all major command groups: strings, lists, sets, sorted sets,
//! hashes, streams, bitmaps, HyperLogLog, geo, expiry, transactions, scripting,
//! and pub/sub.

use cave_cache::db::{Db, ServerState};
use cave_cache::commands::*;
use cave_cache::resp::Resp;
use cave_cache::types::{Entry, Value};
use cave_cache::config::Config;
use std::sync::Arc;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn args(parts: &[&str]) -> Vec<Vec<u8>> {
    parts.iter().map(|s| s.as_bytes().to_vec()).collect()
}

fn new_db() -> Db {
    Db::new()
}

fn ok() -> Resp {
    Resp::ok()
}

fn int(n: i64) -> Resp {
    Resp::Integer(n)
}

fn bulk(s: &str) -> Resp {
    Resp::BulkString(Some(s.as_bytes().to_vec()))
}

fn nil() -> Resp {
    Resp::BulkString(None)
}

// ── String commands ───────────────────────────────────────────────────────────

#[test]
fn test_set_get() {
    let mut db = new_db();
    let r = strings::cmd_set(&args(&["SET", "foo", "bar"]), &mut db).unwrap();
    assert_eq!(r, ok());
    let r = strings::cmd_get(&args(&["GET", "foo"]), &mut db).unwrap();
    assert_eq!(r, bulk("bar"));
}

#[test]
fn test_get_missing() {
    let mut db = new_db();
    let r = strings::cmd_get(&args(&["GET", "missing"]), &mut db).unwrap();
    assert_eq!(r, nil());
}

#[test]
fn test_set_ex() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v", "EX", "100"]), &mut db).unwrap();
    let e = db.keys.get(b"k".as_ref()).unwrap();
    assert!(e.expires_at.is_some());
}

#[test]
fn test_set_nx() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v1"]), &mut db).unwrap();
    let r = strings::cmd_set(&args(&["SET", "k", "v2", "NX"]), &mut db).unwrap();
    assert_eq!(r, nil()); // key exists, NX fails
    assert_eq!(strings::cmd_get(&args(&["GET", "k"]), &mut db).unwrap(), bulk("v1"));
}

#[test]
fn test_set_xx() {
    let mut db = new_db();
    let r = strings::cmd_set(&args(&["SET", "k", "v", "XX"]), &mut db).unwrap();
    assert_eq!(r, nil()); // key doesn't exist, XX fails
}

#[test]
fn test_mset_mget() {
    let mut db = new_db();
    strings::cmd_mset(&args(&["MSET", "a", "1", "b", "2", "c", "3"]), &mut db).unwrap();
    let r = strings::cmd_mget(&args(&["MGET", "a", "b", "c", "missing"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("1"), bulk("2"), bulk("3"), nil()])));
}

#[test]
fn test_incr_decr() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "n", "10"]), &mut db).unwrap();
    assert_eq!(strings::cmd_incr(&args(&["INCR", "n"]), &mut db).unwrap(), int(11));
    assert_eq!(strings::cmd_decr(&args(&["DECR", "n"]), &mut db).unwrap(), int(10));
    assert_eq!(strings::cmd_incrby(&args(&["INCRBY", "n", "5"]), &mut db).unwrap(), int(15));
    assert_eq!(strings::cmd_decrby(&args(&["DECRBY", "n", "3"]), &mut db).unwrap(), int(12));
}

#[test]
fn test_incr_creates_key() {
    let mut db = new_db();
    assert_eq!(strings::cmd_incr(&args(&["INCR", "counter"]), &mut db).unwrap(), int(1));
    assert_eq!(strings::cmd_incr(&args(&["INCR", "counter"]), &mut db).unwrap(), int(2));
}

#[test]
fn test_append_strlen() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "Hello"]), &mut db).unwrap();
    let n = strings::cmd_append(&args(&["APPEND", "k", " World"]), &mut db).unwrap();
    assert_eq!(n, int(11));
    assert_eq!(strings::cmd_strlen(&args(&["STRLEN", "k"]), &mut db).unwrap(), int(11));
    assert_eq!(strings::cmd_get(&args(&["GET", "k"]), &mut db).unwrap(), bulk("Hello World"));
}

#[test]
fn test_getrange() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "Hello World"]), &mut db).unwrap();
    let r = strings::cmd_getrange(&args(&["GETRANGE", "k", "0", "4"]), &mut db).unwrap();
    assert_eq!(r, bulk("Hello"));
}

#[test]
fn test_incrbyfloat() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "f", "10.5"]), &mut db).unwrap();
    let r = strings::cmd_incrbyfloat(&args(&["INCRBYFLOAT", "f", "0.1"]), &mut db).unwrap();
    if let Resp::BulkString(Some(v)) = r {
        let s = std::str::from_utf8(&v).unwrap();
        let f: f64 = s.parse().unwrap();
        assert!((f - 10.6).abs() < 1e-9);
    } else {
        panic!("Expected bulk string");
    }
}

#[test]
fn test_setnx_setex() {
    let mut db = new_db();
    assert_eq!(strings::cmd_setnx(&args(&["SETNX", "k", "v"]), &mut db).unwrap(), int(1));
    assert_eq!(strings::cmd_setnx(&args(&["SETNX", "k", "v2"]), &mut db).unwrap(), int(0));
    strings::cmd_setex(&args(&["SETEX", "k2", "100", "val"]), &mut db).unwrap();
    assert!(db.keys.get(b"k2".as_ref()).unwrap().expires_at.is_some());
}

#[test]
fn test_getdel() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    assert_eq!(strings::cmd_getdel(&args(&["GETDEL", "k"]), &mut db).unwrap(), bulk("v"));
    assert_eq!(strings::cmd_get(&args(&["GET", "k"]), &mut db).unwrap(), nil());
}

// ── List commands ─────────────────────────────────────────────────────────────

#[test]
fn test_lpush_rpush_lrange() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "a", "b", "c"]), &mut db).unwrap();
    let r = lists::cmd_lrange(&args(&["LRANGE", "l", "0", "-1"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("a"), bulk("b"), bulk("c")])));
}

#[test]
fn test_lpop_rpop() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "a", "b", "c"]), &mut db).unwrap();
    assert_eq!(lists::cmd_lpop(&args(&["LPOP", "l"]), &mut db).unwrap(), bulk("a"));
    assert_eq!(lists::cmd_rpop(&args(&["RPOP", "l"]), &mut db).unwrap(), bulk("c"));
}

#[test]
fn test_llen() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "x", "y"]), &mut db).unwrap();
    assert_eq!(lists::cmd_llen(&args(&["LLEN", "l"]), &mut db).unwrap(), int(2));
}

#[test]
fn test_lindex_lset() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "a", "b", "c"]), &mut db).unwrap();
    assert_eq!(lists::cmd_lindex(&args(&["LINDEX", "l", "1"]), &mut db).unwrap(), bulk("b"));
    lists::cmd_lset(&args(&["LSET", "l", "1", "B"]), &mut db).unwrap();
    assert_eq!(lists::cmd_lindex(&args(&["LINDEX", "l", "1"]), &mut db).unwrap(), bulk("B"));
}

#[test]
fn test_lrem() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "a", "b", "a", "c", "a"]), &mut db).unwrap();
    let r = lists::cmd_lrem(&args(&["LREM", "l", "2", "a"]), &mut db).unwrap();
    assert_eq!(r, int(2));
    assert_eq!(lists::cmd_llen(&args(&["LLEN", "l"]), &mut db).unwrap(), int(3));
}

#[test]
fn test_ltrim() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "a", "b", "c", "d"]), &mut db).unwrap();
    lists::cmd_ltrim(&args(&["LTRIM", "l", "1", "2"]), &mut db).unwrap();
    let r = lists::cmd_lrange(&args(&["LRANGE", "l", "0", "-1"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("b"), bulk("c")])));
}

#[test]
fn test_lmove() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "src", "a", "b", "c"]), &mut db).unwrap();
    let r = lists::cmd_lmove(&args(&["LMOVE", "src", "dst", "LEFT", "RIGHT"]), &mut db).unwrap();
    assert_eq!(r, bulk("a"));
    let dst = lists::cmd_lrange(&args(&["LRANGE", "dst", "0", "-1"]), &mut db).unwrap();
    assert_eq!(dst, Resp::Array(Some(vec![bulk("a")])));
}

// ── Set commands ──────────────────────────────────────────────────────────────

#[test]
fn test_sadd_smembers_scard() {
    let mut db = new_db();
    let r = sets::cmd_sadd(&args(&["SADD", "s", "a", "b", "c", "a"]), &mut db).unwrap();
    assert_eq!(r, int(3)); // only 3 unique
    assert_eq!(sets::cmd_scard(&args(&["SCARD", "s"]), &mut db).unwrap(), int(3));
    if let Resp::Array(Some(members)) = sets::cmd_smembers(&args(&["SMEMBERS", "s"]), &mut db).unwrap() {
        assert_eq!(members.len(), 3);
    } else {
        panic!("Expected array");
    }
}

#[test]
fn test_srem_sismember() {
    let mut db = new_db();
    sets::cmd_sadd(&args(&["SADD", "s", "a", "b", "c"]), &mut db).unwrap();
    assert_eq!(sets::cmd_sismember(&args(&["SISMEMBER", "s", "a"]), &mut db).unwrap(), int(1));
    sets::cmd_srem(&args(&["SREM", "s", "a"]), &mut db).unwrap();
    assert_eq!(sets::cmd_sismember(&args(&["SISMEMBER", "s", "a"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_set_operations() {
    let mut db = new_db();
    sets::cmd_sadd(&args(&["SADD", "s1", "a", "b", "c"]), &mut db).unwrap();
    sets::cmd_sadd(&args(&["SADD", "s2", "b", "c", "d"]), &mut db).unwrap();

    if let Resp::Array(Some(members)) = sets::cmd_sinter(&args(&["SINTER", "s1", "s2"]), &mut db).unwrap() {
        let mut m: Vec<String> = members.iter().filter_map(|r| {
            if let Resp::BulkString(Some(v)) = r { Some(String::from_utf8(v.clone()).unwrap()) } else { None }
        }).collect();
        m.sort();
        assert_eq!(m, vec!["b", "c"]);
    }

    if let Resp::Array(Some(members)) = sets::cmd_sdiff(&args(&["SDIFF", "s1", "s2"]), &mut db).unwrap() {
        assert_eq!(members.len(), 1); // only "a"
    }

    if let Resp::Array(Some(members)) = sets::cmd_sunion(&args(&["SUNION", "s1", "s2"]), &mut db).unwrap() {
        assert_eq!(members.len(), 4); // a, b, c, d
    }
}

#[test]
fn test_smove() {
    let mut db = new_db();
    sets::cmd_sadd(&args(&["SADD", "src", "a", "b"]), &mut db).unwrap();
    sets::cmd_sadd(&args(&["SADD", "dst", "c"]), &mut db).unwrap();
    let r = sets::cmd_smove(&args(&["SMOVE", "src", "dst", "a"]), &mut db).unwrap();
    assert_eq!(r, int(1));
    assert_eq!(sets::cmd_scard(&args(&["SCARD", "src"]), &mut db).unwrap(), int(1));
    assert_eq!(sets::cmd_scard(&args(&["SCARD", "dst"]), &mut db).unwrap(), int(2));
}

// ── Sorted Set commands ───────────────────────────────────────────────────────

#[test]
fn test_zadd_zscore_zrank() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a", "2.0", "b", "3.0", "c"]), &mut db).unwrap();
    assert_eq!(sorted_sets::cmd_zscore(&args(&["ZSCORE", "z", "b"]), &mut db).unwrap(), bulk("2"));
    assert_eq!(sorted_sets::cmd_zrank(&args(&["ZRANK", "z", "a"]), &mut db).unwrap(), int(0));
    assert_eq!(sorted_sets::cmd_zrank(&args(&["ZRANK", "z", "c"]), &mut db).unwrap(), int(2));
    assert_eq!(sorted_sets::cmd_zcard(&args(&["ZCARD", "z"]), &mut db).unwrap(), int(3));
}

#[test]
fn test_zrange() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a", "2.0", "b", "3.0", "c"]), &mut db).unwrap();
    let r = sorted_sets::cmd_zrange(&args(&["ZRANGE", "z", "0", "-1"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("a"), bulk("b"), bulk("c")])));
}

#[test]
fn test_zrange_withscores() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a", "2.0", "b"]), &mut db).unwrap();
    let r = sorted_sets::cmd_zrange(&args(&["ZRANGE", "z", "0", "-1", "WITHSCORES"]), &mut db).unwrap();
    if let Resp::Array(Some(items)) = r {
        assert_eq!(items.len(), 4); // a, 1, b, 2
    }
}

#[test]
fn test_zrem_zincrby() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a"]), &mut db).unwrap();
    sorted_sets::cmd_zincrby(&args(&["ZINCRBY", "z", "5.0", "a"]), &mut db).unwrap();
    if let Resp::BulkString(Some(v)) = sorted_sets::cmd_zscore(&args(&["ZSCORE", "z", "a"]), &mut db).unwrap() {
        let s = std::str::from_utf8(&v).unwrap();
        let f: f64 = s.parse().unwrap();
        assert!((f - 6.0).abs() < 1e-9);
    }
    sorted_sets::cmd_zrem(&args(&["ZREM", "z", "a"]), &mut db).unwrap();
    assert_eq!(sorted_sets::cmd_zcard(&args(&["ZCARD", "z"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_zadd_nx_xx_gt_lt() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "5.0", "a"]), &mut db).unwrap();
    // NX: don't update existing
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "NX", "10.0", "a"]), &mut db).unwrap();
    if let Resp::BulkString(Some(v)) = sorted_sets::cmd_zscore(&args(&["ZSCORE", "z", "a"]), &mut db).unwrap() {
        let s = std::str::from_utf8(&v).unwrap();
        let f: f64 = s.parse().unwrap();
        assert!((f - 5.0).abs() < 1e-9); // unchanged
    }
    // GT: only update if new > current
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "GT", "3.0", "a"]), &mut db).unwrap();
    if let Resp::BulkString(Some(v)) = sorted_sets::cmd_zscore(&args(&["ZSCORE", "z", "a"]), &mut db).unwrap() {
        let f: f64 = std::str::from_utf8(&v).unwrap().parse().unwrap();
        assert!((f - 5.0).abs() < 1e-9); // unchanged since 3 < 5
    }
}

#[test]
fn test_zpopmin_zpopmax() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a", "2.0", "b", "3.0", "c"]), &mut db).unwrap();
    let r = sorted_sets::cmd_zpopmin(&args(&["ZPOPMIN", "z"]), &mut db).unwrap();
    if let Resp::Array(Some(items)) = r {
        assert_eq!(items[0], bulk("a"));
    }
    let r = sorted_sets::cmd_zpopmax(&args(&["ZPOPMAX", "z"]), &mut db).unwrap();
    if let Resp::Array(Some(items)) = r {
        assert_eq!(items[0], bulk("c"));
    }
}

#[test]
fn test_zcount_zrangebyscore() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z", "1.0", "a", "2.0", "b", "3.0", "c", "4.0", "d"]), &mut db).unwrap();
    assert_eq!(sorted_sets::cmd_zcount(&args(&["ZCOUNT", "z", "2", "3"]), &mut db).unwrap(), int(2));
    let r = sorted_sets::cmd_zrangebyscore(&args(&["ZRANGEBYSCORE", "z", "2", "3"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("b"), bulk("c")])));
}

#[test]
fn test_zunionstore_zinterstore() {
    let mut db = new_db();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z1", "1.0", "a", "2.0", "b"]), &mut db).unwrap();
    sorted_sets::cmd_zadd(&args(&["ZADD", "z2", "3.0", "b", "4.0", "c"]), &mut db).unwrap();
    sorted_sets::cmd_zunionstore(&args(&["ZUNIONSTORE", "out", "2", "z1", "z2"]), &mut db).unwrap();
    assert_eq!(sorted_sets::cmd_zcard(&args(&["ZCARD", "out"]), &mut db).unwrap(), int(3));
    sorted_sets::cmd_zinterstore(&args(&["ZINTERSTORE", "out2", "2", "z1", "z2"]), &mut db).unwrap();
    assert_eq!(sorted_sets::cmd_zcard(&args(&["ZCARD", "out2"]), &mut db).unwrap(), int(1)); // only "b"
}

// ── Hash commands ─────────────────────────────────────────────────────────────

#[test]
fn test_hset_hget_hgetall() {
    let mut db = new_db();
    hashes::cmd_hset(&args(&["HSET", "h", "f1", "v1", "f2", "v2"]), &mut db).unwrap();
    assert_eq!(hashes::cmd_hget(&args(&["HGET", "h", "f1"]), &mut db).unwrap(), bulk("v1"));
    assert_eq!(hashes::cmd_hlen(&args(&["HLEN", "h"]), &mut db).unwrap(), int(2));
    if let Resp::Array(Some(items)) = hashes::cmd_hgetall(&args(&["HGETALL", "h"]), &mut db).unwrap() {
        assert_eq!(items.len(), 4); // f1, v1, f2, v2
    }
}

#[test]
fn test_hdel_hexists() {
    let mut db = new_db();
    hashes::cmd_hset(&args(&["HSET", "h", "f", "v"]), &mut db).unwrap();
    assert_eq!(hashes::cmd_hexists(&args(&["HEXISTS", "h", "f"]), &mut db).unwrap(), int(1));
    hashes::cmd_hdel(&args(&["HDEL", "h", "f"]), &mut db).unwrap();
    assert_eq!(hashes::cmd_hexists(&args(&["HEXISTS", "h", "f"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_hmget() {
    let mut db = new_db();
    hashes::cmd_hset(&args(&["HSET", "h", "a", "1", "b", "2"]), &mut db).unwrap();
    let r = hashes::cmd_hmget(&args(&["HMGET", "h", "a", "b", "missing"]), &mut db).unwrap();
    assert_eq!(r, Resp::Array(Some(vec![bulk("1"), bulk("2"), nil()])));
}

#[test]
fn test_hincrby_hincrbyfloat() {
    let mut db = new_db();
    hashes::cmd_hset(&args(&["HSET", "h", "n", "10"]), &mut db).unwrap();
    assert_eq!(hashes::cmd_hincrby(&args(&["HINCRBY", "h", "n", "5"]), &mut db).unwrap(), int(15));
}

#[test]
fn test_hkeys_hvals() {
    let mut db = new_db();
    hashes::cmd_hset(&args(&["HSET", "h", "k1", "v1", "k2", "v2"]), &mut db).unwrap();
    if let Resp::Array(Some(keys)) = hashes::cmd_hkeys(&args(&["HKEYS", "h"]), &mut db).unwrap() {
        assert_eq!(keys.len(), 2);
    }
    if let Resp::Array(Some(vals)) = hashes::cmd_hvals(&args(&["HVALS", "h"]), &mut db).unwrap() {
        assert_eq!(vals.len(), 2);
    }
}

// ── Bitmap commands ───────────────────────────────────────────────────────────

#[test]
fn test_setbit_getbit() {
    let mut db = new_db();
    assert_eq!(bitmap::cmd_setbit(&args(&["SETBIT", "b", "7", "1"]), &mut db).unwrap(), int(0));
    assert_eq!(bitmap::cmd_getbit(&args(&["GETBIT", "b", "7"]), &mut db).unwrap(), int(1));
    assert_eq!(bitmap::cmd_getbit(&args(&["GETBIT", "b", "0"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_bitcount() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "b", "foobar"]), &mut db).unwrap();
    let r = bitmap::cmd_bitcount(&args(&["BITCOUNT", "b"]), &mut db).unwrap();
    assert_eq!(r, int(26)); // "foobar" has 26 set bits
}

#[test]
fn test_bitop() {
    let mut db = new_db();
    // Insert raw bytes via direct DB manipulation to avoid string escaping limits
    use cave_cache::types::{Entry, Value};
    db.keys.insert(b"k1".to_vec(), Entry::new(Value::String(vec![0xff, 0x0f])));
    db.keys.insert(b"k2".to_vec(), Entry::new(Value::String(vec![0x0f, 0xff])));
    bitmap::cmd_bitop(
        &[b"BITOP".to_vec(), b"AND".to_vec(), b"dest".to_vec(), b"k1".to_vec(), b"k2".to_vec()],
        &mut db,
    ).unwrap();
    let r = strings::cmd_get(&[b"GET".to_vec(), b"dest".to_vec()], &mut db).unwrap();
    if let Resp::BulkString(Some(v)) = r {
        assert_eq!(v[0], 0x0f);
        assert_eq!(v[1], 0x0f);
    } else {
        panic!("Expected bulk string");
    }
}

// ── HyperLogLog commands ──────────────────────────────────────────────────────

#[test]
fn test_pfadd_pfcount() {
    let mut db = new_db();
    let r = hyperloglog::cmd_pfadd(&args(&["PFADD", "hll", "a", "b", "c", "d", "e"]), &mut db).unwrap();
    assert_eq!(r, int(1)); // changed
    let count = hyperloglog::cmd_pfcount(&args(&["PFCOUNT", "hll"]), &mut db).unwrap();
    if let Resp::Integer(n) = count {
        assert!(n >= 4 && n <= 6); // approximate, within 10% of 5
    }
}

#[test]
fn test_pfmerge() {
    let mut db = new_db();
    hyperloglog::cmd_pfadd(&args(&["PFADD", "h1", "a", "b", "c"]), &mut db).unwrap();
    hyperloglog::cmd_pfadd(&args(&["PFADD", "h2", "d", "e", "f"]), &mut db).unwrap();
    hyperloglog::cmd_pfmerge(&args(&["PFMERGE", "out", "h1", "h2"]), &mut db).unwrap();
    let count = hyperloglog::cmd_pfcount(&args(&["PFCOUNT", "out"]), &mut db).unwrap();
    if let Resp::Integer(n) = count {
        assert!(n >= 4 && n <= 8); // approximate count of 6 unique elements
    }
}

// ── Key commands ──────────────────────────────────────────────────────────────

#[test]
fn test_del_exists_type() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    assert_eq!(keys::cmd_exists(&args(&["EXISTS", "k"]), &mut db).unwrap(), int(1));
    assert_eq!(keys::cmd_type(&args(&["TYPE", "k"]), &mut db).unwrap(), Resp::SimpleString(b"string".to_vec()));
    keys::cmd_del(&args(&["DEL", "k"]), &mut db).unwrap();
    assert_eq!(keys::cmd_exists(&args(&["EXISTS", "k"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_rename() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "src", "hello"]), &mut db).unwrap();
    keys::cmd_rename(&args(&["RENAME", "src", "dst"]), &mut db).unwrap();
    assert_eq!(strings::cmd_get(&args(&["GET", "dst"]), &mut db).unwrap(), bulk("hello"));
    assert_eq!(strings::cmd_get(&args(&["GET", "src"]), &mut db).unwrap(), nil());
}

#[test]
fn test_keys_pattern() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "foo", "1"]), &mut db).unwrap();
    strings::cmd_set(&args(&["SET", "foobar", "2"]), &mut db).unwrap();
    strings::cmd_set(&args(&["SET", "bar", "3"]), &mut db).unwrap();
    if let Resp::Array(Some(ks)) = keys::cmd_keys(&args(&["KEYS", "foo*"]), &mut db).unwrap() {
        assert_eq!(ks.len(), 2);
    }
}

#[test]
fn test_dbsize_flushdb() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "a", "1"]), &mut db).unwrap();
    strings::cmd_set(&args(&["SET", "b", "2"]), &mut db).unwrap();
    assert_eq!(keys::cmd_dbsize(&args(&["DBSIZE"]), &mut db).unwrap(), int(2));
    keys::cmd_flushdb(&args(&["FLUSHDB"]), &mut db).unwrap();
    assert_eq!(keys::cmd_dbsize(&args(&["DBSIZE"]), &mut db).unwrap(), int(0));
}

#[test]
fn test_copy() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "src", "hello"]), &mut db).unwrap();
    let r = keys::cmd_copy(&args(&["COPY", "src", "dst"]), &mut db).unwrap();
    assert_eq!(r, int(1));
    assert_eq!(strings::cmd_get(&args(&["GET", "dst"]), &mut db).unwrap(), bulk("hello"));
    assert_eq!(strings::cmd_get(&args(&["GET", "src"]), &mut db).unwrap(), bulk("hello")); // unchanged
}

#[test]
fn test_scan() {
    let mut db = new_db();
    for i in 0..20 {
        strings::cmd_set(&args(&["SET", &format!("key:{}", i), "v"]), &mut db).unwrap();
    }
    // SCAN 0 COUNT 100 should return all keys
    if let Resp::Array(Some(items)) = keys::cmd_scan(&args(&["SCAN", "0", "COUNT", "100"]), &mut db).unwrap() {
        if let Resp::Array(Some(ks)) = &items[1] {
            assert_eq!(ks.len(), 20);
        }
    }
}

// ── Expiry commands ───────────────────────────────────────────────────────────

#[test]
fn test_expire_ttl_persist() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    expiry::cmd_expire(&args(&["EXPIRE", "k", "100"]), &mut db).unwrap();
    let ttl = expiry::cmd_ttl(&args(&["TTL", "k"]), &mut db).unwrap();
    if let Resp::Integer(t) = ttl {
        assert!(t > 90 && t <= 100);
    }
    expiry::cmd_persist(&args(&["PERSIST", "k"]), &mut db).unwrap();
    assert_eq!(expiry::cmd_ttl(&args(&["TTL", "k"]), &mut db).unwrap(), int(-1));
}

#[test]
fn test_pexpire_pttl() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    expiry::cmd_pexpire(&args(&["PEXPIRE", "k", "100000"]), &mut db).unwrap();
    let pttl = expiry::cmd_pttl(&args(&["PTTL", "k"]), &mut db).unwrap();
    if let Resp::Integer(t) = pttl {
        assert!(t > 90000 && t <= 100000);
    }
}

#[test]
fn test_ttl_no_expire() {
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    assert_eq!(expiry::cmd_ttl(&args(&["TTL", "k"]), &mut db).unwrap(), int(-1));
    assert_eq!(expiry::cmd_ttl(&args(&["TTL", "missing"]), &mut db).unwrap(), int(-2));
}

// ── Geo commands ──────────────────────────────────────────────────────────────

#[test]
fn test_geoadd_geopos() {
    let mut db = new_db();
    let r = geo::cmd_geoadd(&args(&["GEOADD", "cities", "13.361389", "38.115556", "Palermo",
                                    "15.087269", "37.502669", "Catania"]), &mut db).unwrap();
    assert_eq!(r, int(2));
    if let Resp::Array(Some(positions)) = geo::cmd_geopos(&args(&["GEOPOS", "cities", "Palermo"]), &mut db).unwrap() {
        if let Resp::Array(Some(coords)) = &positions[0] {
            // coords[0] ≈ 13.361389, coords[1] ≈ 38.115556
            if let (Resp::BulkString(Some(lon)), Resp::BulkString(Some(lat))) = (&coords[0], &coords[1]) {
                let lon: f64 = std::str::from_utf8(lon).unwrap().parse().unwrap();
                let lat: f64 = std::str::from_utf8(lat).unwrap().parse().unwrap();
                assert!((lon - 13.361389).abs() < 0.001);
                assert!((lat - 38.115556).abs() < 0.001);
            }
        }
    }
}

#[test]
fn test_geodist() {
    let mut db = new_db();
    geo::cmd_geoadd(&args(&["GEOADD", "cities", "13.361389", "38.115556", "Palermo",
                             "15.087269", "37.502669", "Catania"]), &mut db).unwrap();
    if let Resp::BulkString(Some(dist)) = geo::cmd_geodist(&args(&["GEODIST", "cities", "Palermo", "Catania", "km"]), &mut db).unwrap() {
        let d: f64 = std::str::from_utf8(&dist).unwrap().parse().unwrap();
        // Known distance: ~166.27 km
        assert!(d > 160.0 && d < 175.0);
    }
}

// ── Stream commands ───────────────────────────────────────────────────────────

#[test]
fn test_xadd_xlen_xrange() {
    let mut db = new_db();
    streams::cmd_xadd(&args(&["XADD", "s", "*", "name", "Alice", "age", "30"]), &mut db).unwrap();
    streams::cmd_xadd(&args(&["XADD", "s", "*", "name", "Bob", "age", "25"]), &mut db).unwrap();
    assert_eq!(streams::cmd_xlen(&args(&["XLEN", "s"]), &mut db).unwrap(), int(2));
    if let Resp::Array(Some(entries)) = streams::cmd_xrange(&args(&["XRANGE", "s", "-", "+"]), &mut db).unwrap() {
        assert_eq!(entries.len(), 2);
    }
}

#[test]
fn test_xdel() {
    let mut db = new_db();
    let id1 = if let Resp::BulkString(Some(id)) = streams::cmd_xadd(&args(&["XADD", "s", "*", "k", "v"]), &mut db).unwrap() {
        String::from_utf8(id).unwrap()
    } else { panic!() };
    streams::cmd_xadd(&args(&["XADD", "s", "*", "k", "v2"]), &mut db).unwrap();
    let r = streams::cmd_xdel(&args(&["XDEL", "s", &id1]), &mut db).unwrap();
    assert_eq!(r, int(1));
    assert_eq!(streams::cmd_xlen(&args(&["XLEN", "s"]), &mut db).unwrap(), int(1));
}

#[test]
fn test_xtrim() {
    let mut db = new_db();
    for i in 0..10 {
        streams::cmd_xadd(&args(&["XADD", "s", "*", "i", &i.to_string()]), &mut db).unwrap();
    }
    let trimmed = streams::cmd_xtrim(&args(&["XTRIM", "s", "MAXLEN", "5"]), &mut db).unwrap();
    assert_eq!(trimmed, int(5));
    assert_eq!(streams::cmd_xlen(&args(&["XLEN", "s"]), &mut db).unwrap(), int(5));
}

// ── Transaction support ───────────────────────────────────────────────────────

#[test]
fn test_transaction_state_basics() {
    use cave_cache::commands::transactions::TransactionState;
    let mut tx = TransactionState::new();
    assert!(!tx.aborted);
    tx.queue_command(args(&["SET", "k", "v"]));
    assert_eq!(tx.queued.len(), 1);
}

#[test]
fn test_watch_dirty_detection() {
    use cave_cache::commands::transactions::{TransactionState, WatchedKey};
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "k", "v"]), &mut db).unwrap();
    let version = db.keys.get(b"k".as_ref()).unwrap().version;

    let mut tx = TransactionState::new();
    tx.watched_keys.push(WatchedKey { key: b"k".to_vec(), version, db_index: 0 });

    // Not dirty yet
    assert!(!tx.is_dirty(&db, 0));

    // Modify the key
    strings::cmd_set(&args(&["SET", "k", "v2"]), &mut db).unwrap();

    // Now dirty
    assert!(tx.is_dirty(&db, 0));
}

// ── RESP encoding/decoding ────────────────────────────────────────────────────

#[test]
fn test_resp_encoding() {
    use cave_cache::resp::{encode_resp, Resp};

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::SimpleString(b"OK".to_vec()));
    assert_eq!(buf, b"+OK\r\n");

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::Integer(42));
    assert_eq!(buf, b":42\r\n");

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::BulkString(Some(b"hello".to_vec())));
    assert_eq!(buf, b"$5\r\nhello\r\n");

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::BulkString(None));
    assert_eq!(buf, b"$-1\r\n");

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::Error("ERR bad".to_string()));
    assert_eq!(buf, b"-ERR bad\r\n");

    let mut buf = Vec::new();
    encode_resp(&mut buf, &Resp::Array(Some(vec![
        Resp::BulkString(Some(b"foo".to_vec())),
        Resp::Integer(1),
    ])));
    assert_eq!(buf, b"*2\r\n$3\r\nfoo\r\n:1\r\n");
}

// ── Cluster hash slot ─────────────────────────────────────────────────────────

#[test]
fn test_hash_slot() {
    use cave_cache::cluster::hash_slot;
    // hash tag: {user}.123 uses "user" for slot calculation — same slot as "user"
    assert_eq!(hash_slot(b"{user}.123"), hash_slot(b"user"));
    assert_eq!(hash_slot(b"{user}.456"), hash_slot(b"user"));
    // Same key always gives same slot
    assert_eq!(hash_slot(b"foo"), hash_slot(b"foo"));
    // All slots in valid range [0, 16383]
    assert!(hash_slot(b"foo") < 16384);
    assert!(hash_slot(b"bar") < 16384);
}

// ── ACL ───────────────────────────────────────────────────────────────────────

#[test]
fn test_acl_authenticate() {
    use cave_cache::acl::AclState;
    let mut acl = AclState::new();
    // default user with no password: always passes when no password set
    let ok = acl.authenticate("default", "");
    assert!(ok);
}

// ── Server commands (unit) ────────────────────────────────────────────────────

#[test]
fn test_cmd_ping() {
    let r = server_cmds::cmd_ping(&args(&["PING"])).unwrap();
    assert_eq!(r, Resp::SimpleString(b"PONG".to_vec()));
    let r = server_cmds::cmd_ping(&args(&["PING", "hello"])).unwrap();
    assert_eq!(r, bulk("hello"));
}

#[test]
fn test_cmd_echo() {
    let r = server_cmds::cmd_echo(&args(&["ECHO", "world"])).unwrap();
    assert_eq!(r, bulk("world"));
}

#[test]
fn test_cmd_select() {
    let r = server_cmds::cmd_select(&args(&["SELECT", "5"]), 16).unwrap();
    assert_eq!(r, 5);
    let err = server_cmds::cmd_select(&args(&["SELECT", "16"]), 16);
    assert!(err.is_err());
}

#[test]
fn test_cmd_config_get() {
    use cave_cache::config::Config;
    let config = Config::default();
    let r = server_cmds::cmd_config_get(&args(&["CONFIG", "GET", "maxmemory"]), &config).unwrap();
    if let Resp::Array(Some(items)) = r {
        assert_eq!(items.len(), 2);
    }
}

// ── Error type coverage ───────────────────────────────────────────────────────

#[test]
fn test_wrongtype_error() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "l", "v"]), &mut db).unwrap();
    let r = strings::cmd_get(&args(&["GET", "l"]), &mut db);
    assert!(r.is_err());
    match r.unwrap_err() {
        cave_cache::error::CacheError::WrongType => {}
        e => panic!("Expected WrongType, got {:?}", e),
    }
}

#[test]
fn test_arity_error() {
    let mut db = new_db();
    let r = strings::cmd_get(&args(&["GET"]), &mut db);
    assert!(r.is_err());
}

// ── Pub/Sub registry ──────────────────────────────────────────────────────────

#[test]
fn test_pubsub_registry() {
    use cave_cache::db::PubSubRegistry;
    use tokio::sync::mpsc;
    let mut registry = PubSubRegistry::default();
    let (tx, _rx) = mpsc::unbounded_channel();
    registry.subscribe(1, b"chan1".to_vec(), tx);
    assert_eq!(registry.numsub(&[b"chan1".to_vec()])[0].1, 1);
    registry.unsubscribe(1, b"chan1");
    assert_eq!(registry.numsub(&[b"chan1".to_vec()])[0].1, 0);
}

#[test]
fn test_pubsub_publish() {
    use cave_cache::db::PubSubRegistry;
    use tokio::sync::mpsc;
    let mut registry = PubSubRegistry::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    registry.subscribe(1, b"news".to_vec(), tx);
    let n = registry.publish(b"news", b"hello world");
    assert_eq!(n, 1);
    // Check message was delivered
    let msg = rx.try_recv().unwrap();
    assert_eq!(msg.data, b"hello world");
}

// ── ScriptStore ───────────────────────────────────────────────────────────────

#[test]
fn test_script_store() {
    use cave_cache::db::ScriptStore;
    let mut store = ScriptStore::default();
    let sha = store.load("return 1".to_string());
    assert_eq!(sha.len(), 40); // SHA1 hex = 40 chars
    assert!(store.exists(&sha));
    store.flush();
    assert!(!store.exists(&sha));
}

// ── Scripting eval ────────────────────────────────────────────────────────────

#[test]
fn test_eval_return_integer() {
    use cave_cache::db::ScriptStore;
    let mut db = new_db();
    let store = ScriptStore::default();
    let r = scripting::cmd_eval(&args(&["EVAL", "return 42", "0"]), &mut db, &store).unwrap();
    assert_eq!(r, int(42));
}

#[test]
fn test_eval_return_string() {
    use cave_cache::db::ScriptStore;
    let mut db = new_db();
    let store = ScriptStore::default();
    let r = scripting::cmd_eval(&args(&["EVAL", r#"return "hello""#, "0"]), &mut db, &store).unwrap();
    assert_eq!(r, bulk("hello"));
}

#[test]
fn test_eval_keys_argv() {
    use cave_cache::db::ScriptStore;
    let mut db = new_db();
    let store = ScriptStore::default();
    let r = scripting::cmd_eval(&args(&["EVAL", "return KEYS[1]", "1", "mykey"]), &mut db, &store).unwrap();
    assert_eq!(r, bulk("mykey"));
    let r = scripting::cmd_eval(&args(&["EVAL", "return ARGV[1]", "0", "myarg"]), &mut db, &store).unwrap();
    assert_eq!(r, bulk("myarg"));
}

#[test]
fn test_eval_redis_call() {
    use cave_cache::db::ScriptStore;
    let mut db = new_db();
    strings::cmd_set(&args(&["SET", "mykey", "myval"]), &mut db).unwrap();
    let store = ScriptStore::default();
    let r = scripting::cmd_eval(&args(&["EVAL", r#"return redis.call('GET', KEYS[1])"#, "1", "mykey"]), &mut db, &store).unwrap();
    assert_eq!(r, bulk("myval"));
}

#[test]
fn test_evalsha() {
    use cave_cache::db::ScriptStore;
    let mut db = new_db();
    let mut store = ScriptStore::default();
    let sha = store.load("return 99".to_string());
    let r = scripting::cmd_evalsha(&args(&["EVALSHA", &sha, "0"]), &mut db, &store).unwrap();
    assert_eq!(r, int(99));
}

// ── blpop_impl_sync ───────────────────────────────────────────────────────────

#[test]
fn test_blpop_impl_sync_immediate() {
    let mut db = new_db();
    lists::cmd_rpush(&args(&["RPUSH", "mylist", "alpha"]), &mut db).unwrap();
    let r = lists::blpop_impl_sync(&args(&["BLPOP", "mylist", "0"]), &mut db, true).unwrap();
    assert_eq!(r, Some((b"mylist".to_vec(), b"alpha".to_vec())));
}

#[test]
fn test_blpop_impl_sync_empty() {
    let mut db = new_db();
    let r = lists::blpop_impl_sync(&args(&["BLPOP", "absent", "0"]), &mut db, true).unwrap();
    assert_eq!(r, None);
}

// ── Glob matching ─────────────────────────────────────────────────────────────

#[test]
fn test_glob_match() {
    use cave_cache::db::glob_match;
    assert!(glob_match(b"h?llo", b"hello"));
    assert!(glob_match(b"h*llo", b"hllo"));
    assert!(glob_match(b"h*llo", b"heeeello"));
    assert!(glob_match(b"*", b"anything"));
    assert!(glob_match(b"foo*", b"foobar"));
    assert!(!glob_match(b"foo*", b"barfoo"));
    assert!(glob_match(b"*bar*", b"foobarqux"));
    assert!(glob_match(b"hello", b"hello"));
    assert!(!glob_match(b"hello", b"world"));
}
