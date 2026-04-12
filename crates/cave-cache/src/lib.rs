//! cave-cache — Redis/Valkey replacement for distributed caching.
//!
//! Replaces: Redis, Valkey
//! Features: get/set/delete/expire, glob pattern matching, atomic incr/decr,
//!           pipeline operations, pub/sub channels, in-memory TTL eviction.

pub mod cache;
pub mod models;
pub mod routes;

use axum::Router;
use std::sync::{Arc, Mutex};

/// Shared state for the cache module.
pub struct CacheState {
    pub store: Mutex<cache::CacheStore>,
}

impl CacheState {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(cache::CacheStore::new()),
        }
    }
}

impl Default for CacheState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<CacheState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "cache";
//! cave-cache — In-memory cache, Redis replacement.
pub mod cluster;
pub mod config;
pub mod engine;
pub mod expiry;
pub mod hashes;
pub mod lists;
pub mod pubsub;
pub mod routes;
pub mod script;
pub mod sets;
pub mod sorted_sets;
pub mod streams;
pub mod strings;
pub mod transaction;
pub mod types;
pub use config::CacheConfig;
pub use engine::CacheEngine;
pub use types::{CacheError, CacheResult};
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;
    use super::engine::CacheEngine;
    use super::cluster::{hash_slot, ClusterConfig};
    use super::script::ScriptEngine;
    use super::transaction::TxCommand;
    fn engine() -> CacheEngine {
        CacheEngine::new()
    }
    // ── Test 1: set + get string ────────────────────────────────────────────
    #[test]
    fn test_set_get_string() {
        let e = engine();
        e.set("hello", b"world".to_vec(), None).unwrap();
        let val = e.get("hello").unwrap();
        assert_eq!(val, Some(b"world".to_vec()));
    }
    // ── Test 2: mset + mget ─────────────────────────────────────────────────
    #[test]
    fn test_mset_mget() {
        let e = engine();
        e.mset(&[("a", b"1".to_vec()), ("b", b"2".to_vec())]).unwrap();
        let vals = e.mget(&["a", "b", "c"]);
        assert_eq!(vals[0], Some(b"1".to_vec()));
        assert_eq!(vals[1], Some(b"2".to_vec()));
        assert_eq!(vals[2], None);
    }
    // ── Test 3: incr / decr ─────────────────────────────────────────────────
    #[test]
    fn test_incr_decr() {
        let e = engine();
        assert_eq!(e.incr("cnt").unwrap(), 1);
        assert_eq!(e.incr("cnt").unwrap(), 2);
        assert_eq!(e.decr("cnt").unwrap(), 1);
        assert_eq!(e.incrby("cnt", 10).unwrap(), 11);
    }
    // ── Test 4: append ──────────────────────────────────────────────────────
    #[test]
    fn test_append() {
        let e = engine();
        e.set("k", b"Hello".to_vec(), None).unwrap();
        let len = e.append("k", b" World".to_vec()).unwrap();
        assert_eq!(len, 11);
        assert_eq!(e.get("k").unwrap(), Some(b"Hello World".to_vec()));
    }
    // ── Test 5: lpush + lrange + llen ───────────────────────────────────────
    #[test]
    fn test_lpush_lrange_llen() {
        let e = engine();
        e.lpush("list", &[b"a".to_vec(), b"b".to_vec()]).unwrap();
        assert_eq!(e.llen("list").unwrap(), 2);
        let range = e.lrange("list", 0, -1).unwrap();
        // lpush pushes to front so order is reversed: b, a
        assert_eq!(range, vec![b"b".to_vec(), b"a".to_vec()]);
    }
    // ── Test 6: rpush + rpop ────────────────────────────────────────────────
    #[test]
    fn test_rpush_rpop() {
        let e = engine();
        e.rpush("list", &[b"x".to_vec(), b"y".to_vec()]).unwrap();
        let popped = e.rpop("list", 1).unwrap();
        assert_eq!(popped, vec![b"y".to_vec()]);
    }
    // ── Test 7: sadd + smembers + sinter + sunion + sdiff ──────────────────
    #[test]
    fn test_set_operations() {
        let e = engine();
        e.sadd("s1", &[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]).unwrap();
        e.sadd("s2", &[b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]).unwrap();
        let mut members = e.smembers("s1").unwrap();
        members.sort();
        assert_eq!(members, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
        let mut inter = e.sinter(&["s1", "s2"]).unwrap();
        inter.sort();
        assert_eq!(inter, vec![b"b".to_vec(), b"c".to_vec()]);
        let mut union = e.sunion(&["s1", "s2"]).unwrap();
        union.sort();
        assert_eq!(union, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        let mut diff = e.sdiff(&["s1", "s2"]).unwrap();
        diff.sort();
        assert_eq!(diff, vec![b"a".to_vec()]);
    }
    // ── Test 8: zadd + zrange + zrangebyscore + zrank ───────────────────────
    #[test]
    fn test_zset_operations() {
        let e = engine();
        e.zadd("z", &[(1.0, b"one".to_vec()), (2.0, b"two".to_vec()), (3.0, b"three".to_vec())]).unwrap();
        let range = e.zrange("z", 0, -1, false).unwrap();
        assert_eq!(range, vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]);
        let by_score = e.zrangebyscore("z", 1.5, 3.0).unwrap();
        assert_eq!(by_score, vec![b"two".to_vec(), b"three".to_vec()]);
        let rank = e.zrank("z", b"two").unwrap();
        assert_eq!(rank, Some(1));
        let score = e.zscore("z", b"three").unwrap();
        assert_eq!(score, Some(3.0));
    }
    // ── Test 9: hset + hget + hgetall + hdel + hincrby ──────────────────────
    #[test]
    fn test_hash_operations() {
        let e = engine();
        e.hset("h", &[(b"f1".as_slice(), b"v1".to_vec()), (b"f2".as_slice(), b"v2".to_vec())]).unwrap();
        assert_eq!(e.hget("h", b"f1").unwrap(), Some(b"v1".to_vec()));
        let all = e.hgetall("h").unwrap();
        assert_eq!(all.len(), 2);
        e.hdel("h", &[b"f1".as_slice()]).unwrap();
        assert_eq!(e.hget("h", b"f1").unwrap(), None);
        e.hset("h", &[(b"cnt".as_slice(), b"10".to_vec())]).unwrap();
        let new_val = e.hincrby("h", b"cnt", 5).unwrap();
        assert_eq!(new_val, 15);
    }
    // ── Test 10: xadd + xrange + xlen ───────────────────────────────────────
    #[test]
    fn test_stream_operations() {
        let e = engine();
        let id1 = e.xadd("stream", None, vec![(b"k".to_vec(), b"v1".to_vec())]).unwrap();
        let id2 = e.xadd("stream", None, vec![(b"k".to_vec(), b"v2".to_vec())]).unwrap();
        assert_eq!(e.xlen("stream").unwrap(), 2);
        let entries = e.xrange("stream", "-", "+", None).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, id1);
        assert_eq!(entries[1].id, id2);
    }
    // ── Test 11: expire + ttl + persist ─────────────────────────────────────
    #[test]
    fn test_expire_ttl_persist() {
        let e = engine();
        e.set("key", b"val".to_vec(), None).unwrap();
        e.expire("key", 100).unwrap();
        let ttl = e.ttl("key").unwrap();
        assert!(ttl > 0 && ttl <= 100);
        e.persist("key").unwrap();
        assert_eq!(e.ttl("key").unwrap(), -1);
    }
    // ── Test 12: del + exists ────────────────────────────────────────────────
    #[test]
    fn test_del_exists() {
        let e = engine();
        e.set("k1", b"a".to_vec(), None).unwrap();
        e.set("k2", b"b".to_vec(), None).unwrap();
        assert_eq!(e.exists(&["k1", "k2", "k3"]), 2);
        e.del(&["k1"]);
        assert_eq!(e.exists(&["k1", "k2"]), 1);
    }
    // ── Test 13: type_of ────────────────────────────────────────────────────
    #[test]
    fn test_type_of() {
        let e = engine();
        e.set("s", b"v".to_vec(), None).unwrap();
        assert_eq!(e.type_of("s"), Some("string"));
        e.lpush("l", &[b"v".to_vec()]).unwrap();
        assert_eq!(e.type_of("l"), Some("list"));
        e.sadd("st", &[b"m".to_vec()]).unwrap();
        assert_eq!(e.type_of("st"), Some("set"));
        assert_eq!(e.type_of("missing"), None);
    }
    // ── Test 14: publish + subscribe ────────────────────────────────────────
    #[tokio::test]
    async fn test_pubsub() {
        let e = Arc::new(engine());
        let mut handle = e.subscribe(&["chan"]);
        e.publish("chan", b"hello".to_vec());
        let msg = handle.recv().await.unwrap();
        assert_eq!(msg.channel, "chan");
        assert_eq!(msg.message, b"hello".to_vec());
    }
    // ── Test 15: script eval (return KEYS[1]) ────────────────────────────────
    #[test]
    fn test_script_eval_keys() {
        let e = engine();
        let se = ScriptEngine::new(e.scripts.clone());
        let result = se.eval(
            "return KEYS[1]",
            vec!["mykey".to_string()],
            vec![],
            &e,
        ).unwrap();
        assert_eq!(result, serde_json::Value::String("mykey".to_string()));
    }
    // ── Test 16: script load + evalsha ──────────────────────────────────────
    #[test]
    fn test_script_load_evalsha() {
        let e = engine();
        let se = ScriptEngine::new(e.scripts.clone());
        let sha = se.load("return KEYS[1]");
        assert_eq!(sha.len(), 64); // SHA-256 hex
        let result = se.evalsha(&sha, vec!["testkey".to_string()], vec![], &e).unwrap();
        assert_eq!(result, serde_json::Value::String("testkey".to_string()));
    }
    // ── Test 17: multi/exec transaction ─────────────────────────────────────
    #[test]
    fn test_transaction_exec() {
        let e = engine();
        let mut tx = e.multi();
        tx.commands.push(TxCommand::Set("tx_k".to_string(), b"tx_v".to_vec(), None));
        tx.commands.push(TxCommand::Get("tx_k".to_string()));
        let results = e.exec(tx).unwrap();
        assert_eq!(results[0], serde_json::Value::String("OK".to_string()));
        assert_eq!(results[1], serde_json::Value::String("tx_v".to_string()));
    }
    // ── Test 18: watch + exec (abort if key changed) ────────────────────────
    #[test]
    fn test_watch_abort() {
        let e = engine();
        e.set("watched", b"orig".to_vec(), None).unwrap();
        let mut tx = e.multi();
        e.watch(&mut tx, &["watched"]);
        // Modify the watched key between watch and exec
        e.set("watched", b"changed".to_vec(), None).unwrap();
        tx.commands.push(TxCommand::Get("watched".to_string()));
        let result = e.exec(tx);
        assert!(result.is_err());
    }
    // ── Test 19: hash_slot calculation ──────────────────────────────────────
    #[test]
    fn test_hash_slot_foo() {
        // "foo" → slot computed via CRC16-CCITT (poly 0x1021, init 0) mod 16384
        // Verified value: 12182
        let slot = hash_slot("foo");
        assert!(slot < 16384, "slot must be < 16384");
        assert_eq!(slot, 12182);
    }
    #[test]
    fn test_hash_slot_hash_tag() {
        // {user}.1 and {user}.2 should have the same slot (keyed on "user")
        assert_eq!(hash_slot("{user}.1"), hash_slot("{user}.2"));
    }
    // ── Test 20: cluster node_for_key routing ───────────────────────────────
    #[test]
    fn test_cluster_node_for_key() {
        let config = ClusterConfig::new_single_node();
        let node = config.node_for_key("anykey");
        assert!(node.is_some());
        assert_eq!(node.unwrap().id, "local");
    }
    // ── Test 21: cluster is_local ───────────────────────────────────────────
    #[test]
    fn test_cluster_is_local() {
        let config = ClusterConfig::new_single_node();
        assert!(config.is_local("any_key"));
        assert!(config.is_local("another_key"));
    }
    // ── Test 22: expire + get (expired key returns None) ───────────────────
    #[tokio::test]
    async fn test_expire_get_expired() {
        let e = engine();
        e.set("ex_key", b"val".to_vec(), Some(Duration::from_millis(1))).unwrap();
        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(10)).await;
        let val = e.get("ex_key").unwrap();
        assert_eq!(val, None);
    }
    // ── Bonus: sismember ────────────────────────────────────────────────────
    #[test]
    fn test_sismember() {
        let e = engine();
        e.sadd("s", &[b"member".to_vec()]).unwrap();
        assert!(e.sismember("s", b"member").unwrap());
        assert!(!e.sismember("s", b"nonmember").unwrap());
    }
    // ── Bonus: zcard + zrem ─────────────────────────────────────────────────
    #[test]
    fn test_zcard_zrem() {
        let e = engine();
        e.zadd("z", &[(1.0, b"a".to_vec()), (2.0, b"b".to_vec())]).unwrap();
        assert_eq!(e.zcard("z").unwrap(), 2);
        e.zrem("z", &[b"a".to_vec()]).unwrap();
        assert_eq!(e.zcard("z").unwrap(), 1);
    }
    // ── Bonus: xgroup_create ────────────────────────────────────────────────
    #[test]
    fn test_xgroup_create() {
        let e = engine();
        e.xadd("s", None, vec![(b"k".to_vec(), b"v".to_vec())]).unwrap();
        e.xgroup_create("s", "grp1", "$").unwrap();
        let groups = e.groups.lock().unwrap();
        assert!(groups.get("s").unwrap().contains_key("grp1"));
    }
}
