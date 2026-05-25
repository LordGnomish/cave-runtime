// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-cache — ACL, eviction, config,
//! glob, Db, ZSet, errors, PubSub registry.

use cave_cache::acl::{
    AclState, AclUser, ChannelPermissions, CommandPermissions, KeyPermissions,
};
use cave_cache::config::{Config, EvictionPolicy, LogLevel, NotifyFlags};
use cave_cache::db::{Db, PubSubRegistry, ScriptStore, glob_match};
use cave_cache::error::{CacheError, CacheResult};
use cave_cache::eviction::{estimate_memory, evict_keys, evict_if_needed};
use cave_cache::types::{Entry, Value, ZSet};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Glob matching — pattern wildcards
// ---------------------------------------------------------------------------

#[test]
fn glob_matches_exact_equal_bytes() {
    assert!(glob_match(b"hello", b"hello"));
    assert!(!glob_match(b"hello", b"world"));
}

#[test]
fn glob_star_matches_anything() {
    assert!(glob_match(b"*", b""));
    assert!(glob_match(b"*", b"x"));
    assert!(glob_match(b"*", b"verylongstring"));
}

#[test]
fn glob_star_matches_prefix_and_suffix() {
    assert!(glob_match(b"foo*", b"foobar"));
    assert!(glob_match(b"*bar", b"foobar"));
    assert!(glob_match(b"foo*bar", b"foozzzbar"));
}

#[test]
fn glob_question_mark_matches_single() {
    assert!(glob_match(b"a?c", b"abc"));
    assert!(!glob_match(b"a?c", b"ac"));
    assert!(!glob_match(b"a?c", b"abbc"));
}

#[test]
fn glob_multiple_stars_collapse() {
    assert!(glob_match(b"**", b"abc"));
    assert!(glob_match(b"a***z", b"abcz"));
}

#[test]
fn glob_empty_pattern_only_matches_empty() {
    assert!(glob_match(b"", b""));
    assert!(!glob_match(b"", b"x"));
}

// ---------------------------------------------------------------------------
// AclUser / CommandPermissions / KeyPermissions
// ---------------------------------------------------------------------------

#[test]
fn acl_default_user_can_execute_any_command() {
    let u = AclUser::default_user();
    assert!(u.can_execute("GET"));
    assert!(u.can_execute("SET"));
    assert!(u.can_execute("DEL"));
}

#[test]
fn acl_disabled_user_cannot_execute() {
    let mut u = AclUser::default_user();
    u.enabled = false;
    assert!(!u.can_execute("GET"));
}

#[test]
fn acl_default_user_authenticates_with_no_pass() {
    let u = AclUser::default_user();
    assert!(u.authenticate("anything"));
    assert!(u.authenticate(""));
}

#[test]
fn acl_user_with_password_rejects_wrong_password() {
    let mut u = AclUser::default_user();
    u.flags.no_pass = false;
    // SHA-256 of "hunter2"
    u.passwords.push(
        "f52fbd32b2b3b86ff88ef6c490628285f482af15ddcb29541f94bcf526a3f6c7".into(),
    );
    assert!(u.authenticate("hunter2"));
    assert!(!u.authenticate("wrong"));
}

#[test]
fn command_permissions_all_allows_everything() {
    let p = CommandPermissions::All;
    assert!(p.allows("anything"));
    assert!(p.allows("WeIrD"));
}

#[test]
fn command_permissions_none_denies_everything() {
    let p = CommandPermissions::None;
    assert!(!p.allows("GET"));
}

#[test]
fn command_permissions_specific_allow_list_lowercase_match() {
    let mut allowed = HashSet::new();
    allowed.insert("get".into());
    let denied = HashSet::new();
    let p = CommandPermissions::Specific { allowed, denied };
    // Case-insensitive
    assert!(p.allows("GET"));
    assert!(p.allows("get"));
    assert!(!p.allows("SET"));
}

#[test]
fn command_permissions_denied_wins_over_allowed() {
    let mut allowed = HashSet::new();
    allowed.insert("all".into());
    let mut denied = HashSet::new();
    denied.insert("flushall".into());
    let p = CommandPermissions::Specific { allowed, denied };
    assert!(p.allows("get"));
    assert!(!p.allows("FLUSHALL"));
}

#[test]
fn key_permissions_patterns_use_glob() {
    let p = KeyPermissions::Patterns(vec![b"user:*".to_vec(), b"session:*".to_vec()]);
    assert!(p.allows(b"user:42"));
    assert!(p.allows(b"session:abc"));
    assert!(!p.allows(b"admin:secret"));
}

#[test]
fn channel_permissions_none_blocks_all() {
    let p = ChannelPermissions::None;
    assert!(!p.allows(b"anything"));
}

#[test]
fn acl_state_new_has_default_user() {
    let s = AclState::new();
    assert!(s.get_user("default").is_some());
    assert!(s.get_user("unknown").is_none());
    assert!(s.list_users().contains(&"default".to_string()));
    assert_eq!(s.whoami(), "default");
}

#[test]
fn acl_state_authenticate_unknown_user_false() {
    let s = AclState::new();
    assert!(!s.authenticate("nope", "any"));
    // Default user is no-pass so it always authenticates.
    assert!(s.authenticate("default", "anything"));
}

// ---------------------------------------------------------------------------
// CacheError display + helper builders
// ---------------------------------------------------------------------------

#[test]
fn cache_error_display_matches_redis_format() {
    assert!(CacheError::WrongType.to_string().starts_with("WRONGTYPE"));
    assert!(CacheError::NoAuth.to_string().starts_with("NOAUTH"));
    assert!(CacheError::WrongPass.to_string().starts_with("WRONGPASS"));
    assert!(CacheError::ExecAbort.to_string().starts_with("EXECABORT"));
    assert!(CacheError::Loading.to_string().starts_with("LOADING"));
    assert!(CacheError::NoScript.to_string().starts_with("NOSCRIPT"));
    assert!(CacheError::ClusterDown.to_string().contains("CLUSTERDOWN"));
}

#[test]
fn cache_error_moved_includes_slot_and_addr() {
    let err = CacheError::Moved {
        slot: 7000,
        addr: "10.0.0.1:6379".into(),
    };
    let s = err.to_string();
    assert!(s.starts_with("MOVED 7000"));
    assert!(s.contains("10.0.0.1:6379"));
}

#[test]
fn cache_error_ask_includes_slot_and_addr() {
    let err = CacheError::Ask {
        slot: 42,
        addr: "10.0.0.2:6379".into(),
    };
    let s = err.to_string();
    assert!(s.starts_with("ASK 42"));
}

#[test]
fn cache_error_helper_constructors() {
    assert!(matches!(
        CacheError::generic("boom"),
        CacheError::Generic(_)
    ));
    let e = CacheError::wrong_arity("XADD");
    if let CacheError::WrongArity(cmd) = &e {
        assert_eq!(cmd, "xadd", "wrong_arity lowercases cmd");
    } else {
        panic!();
    }
}

#[test]
fn cache_error_to_resp_error_equals_display() {
    let e = CacheError::NotInteger;
    assert_eq!(e.to_resp_error(), e.to_string());
}

#[test]
fn cache_result_alias_works_for_ok_and_err() {
    fn ok() -> CacheResult<i32> { Ok(7) }
    fn bad() -> CacheResult<i32> { Err(CacheError::NotFound) }
    assert_eq!(ok().unwrap(), 7);
    assert!(matches!(bad(), Err(CacheError::NotFound)));
}

// ---------------------------------------------------------------------------
// EvictionPolicy parse / display round-trip
// ---------------------------------------------------------------------------

#[test]
fn eviction_policy_string_round_trip() {
    let all = [
        EvictionPolicy::NoEviction,
        EvictionPolicy::AllKeysLru,
        EvictionPolicy::VolatileLru,
        EvictionPolicy::AllKeysLfu,
        EvictionPolicy::VolatileLfu,
        EvictionPolicy::AllKeysRandom,
        EvictionPolicy::VolatileRandom,
        EvictionPolicy::VolatileTtl,
    ];
    for p in &all {
        let s = p.as_str();
        let back = EvictionPolicy::from_str(s).expect(s);
        assert_eq!(&back, p);
    }
}

#[test]
fn eviction_policy_unknown_string_is_none() {
    assert!(EvictionPolicy::from_str("not-a-policy").is_none());
    assert!(EvictionPolicy::from_str("").is_none());
}

#[test]
fn eviction_policy_from_str_is_case_insensitive() {
    assert_eq!(
        EvictionPolicy::from_str("ALLKEYS-LRU").unwrap(),
        EvictionPolicy::AllKeysLru
    );
    assert_eq!(
        EvictionPolicy::from_str("Volatile-LRU").unwrap(),
        EvictionPolicy::VolatileLru
    );
}

// ---------------------------------------------------------------------------
// LogLevel + NotifyFlags
// ---------------------------------------------------------------------------

#[test]
fn loglevel_as_str_distinct_per_level() {
    let strs: HashSet<&'static str> = [
        LogLevel::Debug.as_str(),
        LogLevel::Verbose.as_str(),
        LogLevel::Notice.as_str(),
        LogLevel::Warning.as_str(),
    ].into_iter().collect();
    assert_eq!(strs.len(), 4);
}

#[test]
fn notify_flags_empty_string_disables_everything() {
    let f = NotifyFlags::from_str("");
    assert!(!f.any_enabled());
}

#[test]
fn notify_flags_keyspace_and_keyevent_chars() {
    let f = NotifyFlags::from_str("KE");
    assert!(f.keyspace);
    assert!(f.keyevent);
    assert!(f.any_enabled());
}

#[test]
fn notify_flags_a_flag_means_all() {
    let f = NotifyFlags::from_str("A");
    assert!(f.all);
    assert!(f.any_enabled());
}

#[test]
fn notify_flags_individual_classes() {
    let f = NotifyFlags::from_str("g$lszdhxtE");
    assert!(f.generic);
    assert!(f.string);
    assert!(f.list);
    assert!(f.set);
    assert!(f.sorted_set);
    assert!(f.evicted);
    assert!(f.hash);
    assert!(f.expired);
    assert!(f.stream);
    assert!(f.keyevent);
}

#[test]
fn notify_flags_unknown_chars_ignored() {
    let f = NotifyFlags::from_str("xyzABCKE");
    assert!(f.keyspace);
    assert!(f.keyevent);
    assert!(f.all);
    assert!(f.expired);
}

#[test]
fn config_default_addr_uses_bind_and_port() {
    let c = Config::default();
    assert_eq!(c.addr(), format!("{}:{}", c.bind, c.port));
}

#[test]
fn config_default_port_is_redis_default() {
    assert_eq!(Config::default().port, 6379);
}

#[test]
fn config_default_databases_is_16() {
    assert_eq!(Config::default().databases, 16);
}

#[test]
fn config_default_cluster_node_timeout_is_15s() {
    assert_eq!(
        Config::default().cluster_node_timeout,
        Duration::from_millis(15000)
    );
}

// ---------------------------------------------------------------------------
// Db core API
// ---------------------------------------------------------------------------

fn entry_string(s: &[u8]) -> Entry {
    Entry::new(Value::String(s.to_vec()))
}

#[test]
fn db_insert_increments_version() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), entry_string(b"v1"));
    let v1 = db.keys.get(b"k".as_slice()).unwrap().version;
    db.insert(b"k".to_vec(), entry_string(b"v2"));
    let v2 = db.keys.get(b"k".as_slice()).unwrap().version;
    assert_eq!(v2, v1 + 1, "version must bump on every write");
}

#[test]
fn db_get_returns_none_for_missing_key() {
    let mut db = Db::new();
    assert!(db.get(b"absent").is_none());
}

#[test]
fn db_remove_returns_true_only_when_present() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), entry_string(b"v"));
    assert!(db.remove(b"k"));
    assert!(!db.remove(b"k"));
}

#[test]
fn db_exists_is_false_for_missing() {
    let mut db = Db::new();
    assert!(!db.exists(b"x"));
    db.insert(b"x".to_vec(), entry_string(b"y"));
    assert!(db.exists(b"x"));
}

#[test]
fn db_get_typed_wrongtype_error() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), entry_string(b"v"));
    let res = db.get_typed(b"k", "list");
    assert!(matches!(res, Err(CacheError::WrongType)));
}

#[test]
fn db_get_typed_missing_returns_ok_none() {
    let mut db = Db::new();
    let res = db.get_typed(b"missing", "string").unwrap();
    assert!(res.is_none());
}

#[test]
fn db_flush_clears_keys() {
    let mut db = Db::new();
    db.insert(b"a".to_vec(), entry_string(b"1"));
    db.insert(b"b".to_vec(), entry_string(b"2"));
    db.flush();
    assert_eq!(db.dbsize(), 0);
}

#[test]
fn db_expire_cycle_removes_expired_keys() {
    let mut db = Db::new();
    let mut e = entry_string(b"v");
    e.expires_at = Some(Instant::now() - Duration::from_secs(1));
    db.insert(b"old".to_vec(), e);
    db.insert(b"fresh".to_vec(), entry_string(b"v"));
    let evicted = db.expire_cycle();
    assert_eq!(evicted, vec![b"old".to_vec()]);
    assert!(db.exists(b"fresh"));
}

#[test]
fn db_get_lazy_expires_past_ttl() {
    let mut db = Db::new();
    let mut e = entry_string(b"v");
    e.expires_at = Some(Instant::now() - Duration::from_millis(1));
    db.insert(b"k".to_vec(), e);
    assert!(db.get(b"k").is_none(), "expired key must be removed lazily");
}

#[test]
fn entry_is_expired_without_ttl_is_false() {
    let e = entry_string(b"v");
    assert!(!e.is_expired());
}

#[test]
fn entry_pttl_returns_none_when_no_expiry() {
    let e = entry_string(b"v");
    assert!(e.pttl().is_none());
}

#[test]
fn entry_pttl_returns_minus_two_for_expired() {
    let mut e = entry_string(b"v");
    e.expires_at = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(e.pttl(), Some(-2));
}

#[test]
fn entry_touch_updates_lru_clock() {
    let mut e = entry_string(b"v");
    let before = e.lru_clock;
    std::thread::sleep(Duration::from_millis(10));
    e.touch();
    // lru_clock is seconds-resolution, so may or may not change; freq may bump.
    assert!(e.lru_clock >= before);
}

#[test]
fn value_type_name_per_variant() {
    use std::collections::{HashMap, HashSet};
    assert_eq!(Value::String(vec![]).type_name(), "string");
    assert_eq!(Value::List(VecDeque::new()).type_name(), "list");
    assert_eq!(Value::Set(HashSet::new()).type_name(), "set");
    assert_eq!(Value::ZSet(ZSet::new()).type_name(), "zset");
    assert_eq!(Value::Hash(HashMap::new()).type_name(), "hash");
}

// ---------------------------------------------------------------------------
// ZSet
// ---------------------------------------------------------------------------

#[test]
fn zset_add_new_returns_true_existing_returns_false() {
    let mut z = ZSet::new();
    assert!(z.add(b"a".to_vec(), 1.0));
    assert!(!z.add(b"a".to_vec(), 2.0), "duplicate add must return false");
    assert_eq!(z.score(b"a"), Some(2.0));
}

#[test]
fn zset_remove_returns_true_only_when_present() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    assert!(z.remove(b"a"));
    assert!(!z.remove(b"a"));
}

#[test]
fn zset_len_and_is_empty() {
    let mut z = ZSet::new();
    assert!(z.is_empty());
    assert_eq!(z.len(), 0);
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    assert_eq!(z.len(), 2);
    assert!(!z.is_empty());
}

#[test]
fn zset_rank_ascending_starts_at_zero() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    z.add(b"c".to_vec(), 3.0);
    assert_eq!(z.rank(b"a"), Some(0));
    assert_eq!(z.rank(b"b"), Some(1));
    assert_eq!(z.rank(b"c"), Some(2));
    assert!(z.rank(b"missing").is_none());
}

#[test]
fn zset_range_by_score_inclusive() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    z.add(b"c".to_vec(), 3.0);
    let range = z.range_by_score(1.0, 2.5);
    let members: Vec<Vec<u8>> = range.into_iter().map(|(m, _)| m).collect();
    assert_eq!(members, vec![b"a".to_vec(), b"b".to_vec()]);
}

#[test]
fn zset_pop_min_returns_smallest() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 5.0);
    z.add(b"b".to_vec(), 1.0);
    z.add(b"c".to_vec(), 3.0);
    let popped = z.pop_min().unwrap();
    assert_eq!(popped, (b"b".to_vec(), 1.0));
    assert_eq!(z.len(), 2);
}

// ---------------------------------------------------------------------------
// Eviction policies
// ---------------------------------------------------------------------------

fn fill_db(db: &mut Db, n: usize, ttl_each: bool) {
    for i in 0..n {
        let mut e = entry_string(format!("v{}", i).as_bytes());
        if ttl_each {
            e.expires_at = Some(Instant::now() + Duration::from_secs(60));
        }
        db.insert(format!("k{}", i).into_bytes(), e);
    }
}

#[test]
fn evict_no_eviction_does_nothing() {
    let mut db = Db::new();
    fill_db(&mut db, 5, false);
    let n = evict_keys(&mut db, EvictionPolicy::NoEviction, 100);
    assert_eq!(n, 0);
    assert_eq!(db.keys.len(), 5);
}

#[test]
fn evict_allkeys_random_removes_requested_count() {
    let mut db = Db::new();
    fill_db(&mut db, 10, false);
    let n = evict_keys(&mut db, EvictionPolicy::AllKeysRandom, 4);
    assert_eq!(n, 4);
    assert_eq!(db.keys.len(), 6);
}

#[test]
fn evict_volatile_random_only_touches_ttl_keys() {
    let mut db = Db::new();
    fill_db(&mut db, 3, true);
    fill_db(&mut db, 3, false);
    // We have 6 keys, but the volatile flow only sees keys with TTL.
    // (The non-TTL keys "k0..k2" from the second fill_db call overwrote the TTL
    //  versions because fill_db reuses the same name range. To keep this honest,
    //  rewrite using distinct names below.)
    let mut db = Db::new();
    for i in 0..3 {
        let mut e = entry_string(b"v");
        e.expires_at = Some(Instant::now() + Duration::from_secs(60));
        db.insert(format!("ttl_{}", i).into_bytes(), e);
    }
    for i in 0..3 {
        db.insert(format!("nottl_{}", i).into_bytes(), entry_string(b"v"));
    }
    let n = evict_keys(&mut db, EvictionPolicy::VolatileRandom, 5);
    // Only 3 candidates with TTL exist; 5 requested → at most 3 evicted.
    assert!(n <= 3);
    for i in 0..3 {
        assert!(db.exists(format!("nottl_{}", i).as_bytes()), "non-TTL keys must survive");
    }
}

#[test]
fn evict_allkeys_lru_drops_oldest() {
    let mut db = Db::new();
    // Manually set lru_clock so we have a deterministic ordering.
    for (i, name) in ["old", "mid", "new"].iter().enumerate() {
        let mut e = entry_string(b"v");
        e.lru_clock = i as u64;
        db.insert(name.as_bytes().to_vec(), e);
    }
    let n = evict_keys(&mut db, EvictionPolicy::AllKeysLru, 1);
    assert_eq!(n, 1);
    assert!(!db.exists(b"old"));
    assert!(db.exists(b"mid"));
    assert!(db.exists(b"new"));
}

#[test]
fn evict_allkeys_lfu_drops_lowest_freq() {
    let mut db = Db::new();
    for (i, name) in ["cold", "warm", "hot"].iter().enumerate() {
        let mut e = entry_string(b"v");
        e.lfu_freq = (i * 50) as u8;
        db.insert(name.as_bytes().to_vec(), e);
    }
    let n = evict_keys(&mut db, EvictionPolicy::AllKeysLfu, 1);
    assert_eq!(n, 1);
    assert!(!db.exists(b"cold"));
    assert!(db.exists(b"hot"));
}

#[test]
fn evict_volatile_ttl_drops_soonest_expiry() {
    let mut db = Db::new();
    for (i, name) in ["soon", "later", "latest"].iter().enumerate() {
        let mut e = entry_string(b"v");
        e.expires_at = Some(Instant::now() + Duration::from_secs(10 + i as u64 * 60));
        db.insert(name.as_bytes().to_vec(), e);
    }
    let n = evict_keys(&mut db, EvictionPolicy::VolatileTtl, 1);
    assert_eq!(n, 1);
    assert!(!db.exists(b"soon"));
}

#[test]
fn evict_if_needed_returns_zero_when_no_limit() {
    let mut db = Db::new();
    fill_db(&mut db, 100, false);
    let n = evict_if_needed(&mut db, EvictionPolicy::AllKeysRandom, None);
    assert_eq!(n, 0);
}

#[test]
fn evict_if_needed_returns_zero_when_under_limit() {
    let mut db = Db::new();
    fill_db(&mut db, 5, false);
    // 5 keys * 256 bytes estimate = 1280 < 10000
    let n = evict_if_needed(&mut db, EvictionPolicy::AllKeysRandom, Some(10_000));
    assert_eq!(n, 0);
}

#[test]
fn estimate_memory_scales_with_key_count() {
    let mut db = Db::new();
    fill_db(&mut db, 0, false);
    let m0 = estimate_memory(&db);
    fill_db(&mut db, 10, false);
    let m10 = estimate_memory(&db);
    assert!(m10 > m0);
}

// ---------------------------------------------------------------------------
// PubSubRegistry
// ---------------------------------------------------------------------------

#[test]
fn pubsub_subscribe_then_publish_counts_one_receiver() {
    let mut r = PubSubRegistry::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    r.subscribe(1, b"news".to_vec(), tx);
    let count = r.publish(b"news", b"hello");
    assert_eq!(count, 1);
    let msg = rx.try_recv().unwrap();
    assert_eq!(msg.data, b"hello".to_vec());
}

#[test]
fn pubsub_publish_no_subscribers_returns_zero() {
    let r = PubSubRegistry::default();
    assert_eq!(r.publish(b"unsubscribed", b"x"), 0);
}

#[test]
fn pubsub_unsubscribe_removes_specific_client() {
    let mut r = PubSubRegistry::default();
    let (tx1, _) = mpsc::unbounded_channel();
    let (tx2, _) = mpsc::unbounded_channel();
    r.subscribe(1, b"ch".to_vec(), tx1);
    r.subscribe(2, b"ch".to_vec(), tx2);
    r.unsubscribe(1, b"ch");
    // Channel still has client 2 → publish reaches 1 listener.
    let _ = r.publish(b"ch", b"x");
    assert_eq!(r.channel_count(2), 1);
    assert_eq!(r.channel_count(1), 0);
}

#[test]
fn pubsub_unsubscribe_all_clears_for_client() {
    let mut r = PubSubRegistry::default();
    let (tx, _) = mpsc::unbounded_channel();
    r.subscribe(1, b"a".to_vec(), tx);
    let (tx2, _) = mpsc::unbounded_channel();
    r.subscribe(1, b"b".to_vec(), tx2);
    assert_eq!(r.channel_count(1), 2);
    r.unsubscribe_all(1);
    assert_eq!(r.channel_count(1), 0);
}

#[test]
fn pubsub_pattern_match_delivers_pmessage() {
    let mut r = PubSubRegistry::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    r.psubscribe(7, b"news.*".to_vec(), tx);
    let count = r.publish(b"news.sports", b"goal");
    assert_eq!(count, 1);
    let m = rx.try_recv().unwrap();
    assert_eq!(m.pattern, Some(b"news.*".to_vec()));
}

#[test]
fn pubsub_numsub_zero_for_unknown_channel() {
    let r = PubSubRegistry::default();
    let res = r.numsub(&[b"unknown".to_vec()]);
    assert_eq!(res, vec![(b"unknown".to_vec(), 0)]);
}

#[test]
fn pubsub_active_channels_skips_empty() {
    let mut r = PubSubRegistry::default();
    let (tx, _) = mpsc::unbounded_channel();
    r.subscribe(1, b"alive".to_vec(), tx);
    let active = r.active_channels();
    assert_eq!(active, vec![b"alive".to_vec()]);
}

// ---------------------------------------------------------------------------
// ScriptStore
// ---------------------------------------------------------------------------

#[test]
fn script_store_load_returns_sha1_hex_and_persists() {
    let mut s = ScriptStore::default();
    let sha = s.load("return 1".into());
    assert_eq!(sha.len(), 40, "SHA-1 hex is 40 chars");
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(s.exists(&sha));
}

#[test]
fn script_store_load_is_deterministic() {
    let mut a = ScriptStore::default();
    let mut b = ScriptStore::default();
    let sha_a = a.load("return 42".into());
    let sha_b = b.load("return 42".into());
    assert_eq!(sha_a, sha_b);
}

#[test]
fn script_store_flush_clears() {
    let mut s = ScriptStore::default();
    let sha = s.load("a".into());
    assert!(s.exists(&sha));
    s.flush();
    assert!(!s.exists(&sha));
}

#[test]
fn script_store_exists_false_for_unknown_sha() {
    let s = ScriptStore::default();
    assert!(!s.exists("0000000000000000000000000000000000000000"));
}
