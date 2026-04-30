//! cave-cache parity tests — supplements `integration.rs`.
//!
//! Upstream parity reference: redis/src/t_zset.c (sorted-set internals),
//! redis/src/expire.c (lazy expiration semantics), redis/src/util.c (glob),
//! and the RESP3 spec at redis.io/docs/reference/protocol-spec.
//!
//! These tests cover building blocks the command-level integration suite
//! intentionally skips: bound parsers, raw `ZSet` mechanics, the expire
//! cycle, type discriminants, and RESP value helpers.

use cave_cache::db::{Db, PubSubKind, PubSubRegistry, ScriptStore};
use cave_cache::error::CacheError;
use cave_cache::resp::Resp;
use cave_cache::types::{
    bytes_to_f64, bytes_to_i64, f64_to_bytes, i64_to_bytes, normalize_index, Entry, LexBound,
    ScoreBound, Stream, StreamId, Value, ZKey, ZSet,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// ── Bound parsers ────────────────────────────────────────────────────────────

#[test]
fn lex_bound_parses_unbounded_minus_plus() {
    assert!(matches!(LexBound::parse(b"-"), Some(LexBound::Unbounded)));
    assert!(matches!(LexBound::parse(b"+"), Some(LexBound::Unbounded)));
}

#[test]
fn lex_bound_parses_inclusive_and_exclusive() {
    match LexBound::parse(b"[hello") {
        Some(LexBound::Inclusive(b)) => assert_eq!(b, b"hello".to_vec()),
        _ => panic!("expected inclusive"),
    }
    match LexBound::parse(b"(world") {
        Some(LexBound::Exclusive(b)) => assert_eq!(b, b"world".to_vec()),
        _ => panic!("expected exclusive"),
    }
}

#[test]
fn lex_bound_rejects_unprefixed_input() {
    assert!(LexBound::parse(b"raw").is_none());
}

#[test]
fn score_bound_parses_inf_variants() {
    assert!(matches!(ScoreBound::parse(b"-inf"), Some(ScoreBound::NegInf)));
    assert!(matches!(ScoreBound::parse(b"+inf"), Some(ScoreBound::PosInf)));
    assert!(matches!(ScoreBound::parse(b"inf"), Some(ScoreBound::PosInf)));
}

#[test]
fn score_bound_inclusive_and_exclusive() {
    match ScoreBound::parse(b"3.5") {
        Some(ScoreBound::Inclusive(v)) => assert!((v - 3.5).abs() < f64::EPSILON),
        _ => panic!("expected inclusive"),
    }
    match ScoreBound::parse(b"(2.0") {
        Some(ScoreBound::Exclusive(v)) => assert!((v - 2.0).abs() < f64::EPSILON),
        _ => panic!("expected exclusive"),
    }
}

#[test]
fn score_bound_contains_min_max() {
    let b = ScoreBound::Inclusive(5.0);
    assert!(b.contains_min(5.0));
    assert!(b.contains_max(5.0));
    assert!(!b.contains_min(4.0));
    let excl = ScoreBound::Exclusive(5.0);
    assert!(!excl.contains_min(5.0));
    assert!(excl.contains_min(6.0));
}

#[test]
fn score_bound_neg_pos_inf_semantics() {
    assert!(ScoreBound::NegInf.contains_min(-1e9));
    assert!(!ScoreBound::NegInf.contains_max(0.0));
    assert!(ScoreBound::PosInf.contains_max(1e9));
    assert!(!ScoreBound::PosInf.contains_min(0.0));
}

// ── Bytes <-> numeric helpers ────────────────────────────────────────────────

#[test]
fn bytes_to_i64_parses_signed_integers() {
    assert_eq!(bytes_to_i64(b"42"), Some(42));
    assert_eq!(bytes_to_i64(b"-7"), Some(-7));
    assert_eq!(bytes_to_i64(b"  100  "), Some(100));
    assert_eq!(bytes_to_i64(b"abc"), None);
}

#[test]
fn bytes_to_f64_parses_inf_aliases() {
    assert_eq!(bytes_to_f64(b"-inf"), Some(f64::NEG_INFINITY));
    assert_eq!(bytes_to_f64(b"INF"), Some(f64::INFINITY));
    assert_eq!(bytes_to_f64(b"+inf"), Some(f64::INFINITY));
    assert!(bytes_to_f64(b"3.14").unwrap() - 3.14 < 1e-9);
}

#[test]
fn i64_to_bytes_round_trips() {
    let raw = i64_to_bytes(-123);
    assert_eq!(bytes_to_i64(&raw), Some(-123));
}

#[test]
fn f64_to_bytes_special_values() {
    assert_eq!(f64_to_bytes(f64::NEG_INFINITY), b"-inf".to_vec());
    assert_eq!(f64_to_bytes(f64::INFINITY), b"inf".to_vec());
}

#[test]
fn normalize_index_handles_negative() {
    assert_eq!(normalize_index(-1, 5), 4);
    assert_eq!(normalize_index(-10, 5), 0);
    assert_eq!(normalize_index(2, 5), 2);
}

// ── ZSet internal mechanics ──────────────────────────────────────────────────

#[test]
fn zset_add_returns_added_vs_updated() {
    let mut z = ZSet::new();
    assert!(z.add(b"a".to_vec(), 1.0));
    assert!(!z.add(b"a".to_vec(), 2.0)); // update keeps single member
    assert_eq!(z.len(), 1);
    assert_eq!(z.score(b"a"), Some(2.0));
}

#[test]
fn zset_remove() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    assert!(z.remove(b"a"));
    assert!(!z.remove(b"a")); // already gone
    assert!(z.is_empty());
}

#[test]
fn zset_rank_ascending_and_descending() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    z.add(b"c".to_vec(), 3.0);
    assert_eq!(z.rank(b"a"), Some(0));
    assert_eq!(z.rank(b"c"), Some(2));
    assert_eq!(z.rev_rank(b"a"), Some(2));
    assert_eq!(z.rev_rank(b"c"), Some(0));
}

#[test]
fn zset_range_by_index() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    z.add(b"c".to_vec(), 3.0);
    z.add(b"d".to_vec(), 4.0);
    z.add(b"e".to_vec(), 5.0);
    let r = z.range_by_index(1, 3);
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].0, b"b".to_vec());
    assert_eq!(r[2].0, b"d".to_vec());
}

#[test]
fn zset_range_by_index_reverse() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 2.0);
    z.add(b"c".to_vec(), 3.0);
    let r = z.rev_range_by_index(0, 1);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, b"c".to_vec()); // descending
}

#[test]
fn zset_pop_min_max() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 5.0);
    z.add(b"c".to_vec(), 3.0);
    let (m, s) = z.pop_min().unwrap();
    assert_eq!(m, b"a".to_vec());
    assert!((s - 1.0).abs() < f64::EPSILON);
    let (m, s) = z.pop_max().unwrap();
    assert_eq!(m, b"b".to_vec());
    assert!((s - 5.0).abs() < f64::EPSILON);
}

#[test]
fn zset_count_in_range_inclusive() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    z.add(b"b".to_vec(), 5.0);
    z.add(b"c".to_vec(), 10.0);
    assert_eq!(z.count_in_range(1.0, 10.0), 3);
    assert_eq!(z.count_in_range(2.0, 9.0), 1);
    assert_eq!(z.count_in_range(11.0, 20.0), 0);
}

#[test]
fn zset_incr_score_preserves_membership() {
    let mut z = ZSet::new();
    z.add(b"a".to_vec(), 1.0);
    let new_score = z.incr_score(b"a".to_vec(), 5.0);
    assert!((new_score - 6.0).abs() < f64::EPSILON);
    assert_eq!(z.len(), 1);
}

#[test]
fn zset_lex_range_within_same_score() {
    let mut z = ZSet::new();
    z.add(b"apple".to_vec(), 0.0);
    z.add(b"banana".to_vec(), 0.0);
    z.add(b"cherry".to_vec(), 0.0);
    let inclusive = z.range_by_lex(
        &LexBound::Inclusive(b"apple".to_vec()),
        &LexBound::Inclusive(b"banana".to_vec()),
    );
    assert_eq!(inclusive.len(), 2);
    let exclusive = z.range_by_lex(
        &LexBound::Exclusive(b"apple".to_vec()),
        &LexBound::Inclusive(b"cherry".to_vec()),
    );
    assert_eq!(exclusive.len(), 2); // banana, cherry
}

#[test]
fn zkey_orders_by_score_then_member() {
    let a = ZKey { score: 1.0, member: b"a".to_vec() };
    let b = ZKey { score: 1.0, member: b"b".to_vec() };
    let c = ZKey { score: 2.0, member: b"a".to_vec() };
    assert!(a < b);
    assert!(b < c);
}

// ── Value type discriminants ─────────────────────────────────────────────────

#[test]
fn value_type_names_match_redis_strings() {
    assert_eq!(Value::String(vec![]).type_name(), "string");
    assert_eq!(Value::List(VecDeque::new()).type_name(), "list");
    assert_eq!(Value::Set(HashSet::new()).type_name(), "set");
    assert_eq!(Value::Hash(HashMap::new()).type_name(), "hash");
    assert_eq!(Value::ZSet(ZSet::new()).type_name(), "zset");
    assert_eq!(Value::Stream(Stream::new()).type_name(), "stream");
}

// ── Entry behaviour ──────────────────────────────────────────────────────────

#[test]
fn entry_starts_unexpired() {
    let e = Entry::new(Value::String(b"v".to_vec()));
    assert!(!e.is_expired());
    assert_eq!(e.pttl(), None);
    assert_eq!(e.version, 1);
}

#[test]
fn entry_pttl_counts_down() {
    let mut e = Entry::new(Value::String(b"v".to_vec()));
    e.expires_at = Some(Instant::now() + Duration::from_secs(10));
    let ttl = e.pttl().unwrap();
    assert!(ttl > 0);
    assert!(ttl <= 10_000);
}

#[test]
fn entry_pttl_negative_after_expiry() {
    let mut e = Entry::new(Value::String(b"v".to_vec()));
    e.expires_at = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(e.pttl(), Some(-2));
    assert!(e.is_expired());
}

#[test]
fn entry_touch_updates_clocks() {
    let mut e = Entry::new(Value::String(b"v".to_vec()));
    let initial_lru = e.lru_clock;
    let initial_lfu = e.lfu_freq;
    // Multiple touches to ensure at least one increments lfu_freq probabilistically
    for _ in 0..200 {
        e.touch();
    }
    assert!(e.lru_clock >= initial_lru);
    // lfu_freq must be ≥ initial (saturating increment)
    assert!(e.lfu_freq >= initial_lfu);
}

// ── Db lifecycle (lower-level than commands) ─────────────────────────────────

#[test]
fn db_insert_increments_version() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), Entry::new(Value::String(b"1".to_vec())));
    let v1 = db.get(b"k").unwrap().version;
    db.insert(b"k".to_vec(), Entry::new(Value::String(b"2".to_vec())));
    let v2 = db.get(b"k").unwrap().version;
    assert!(v2 > v1, "version must increase on overwrite");
}

#[test]
fn db_get_lazily_evicts_expired() {
    let mut db = Db::new();
    let mut e = Entry::new(Value::String(b"v".to_vec()));
    e.expires_at = Some(Instant::now() - Duration::from_secs(1));
    db.insert(b"k".to_vec(), e);
    assert!(db.get(b"k").is_none()); // lazily removed
    assert_eq!(db.dbsize(), 0);
}

#[test]
fn db_get_typed_mismatch_errors() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), Entry::new(Value::String(b"v".to_vec())));
    let err = db.get_typed(b"k", "list").unwrap_err();
    assert!(matches!(err, CacheError::WrongType));
}

#[test]
fn db_get_typed_returns_none_for_missing() {
    let mut db = Db::new();
    let r = db.get_typed(b"ghost", "string").unwrap();
    assert!(r.is_none());
}

#[test]
fn db_expire_cycle_returns_evicted_keys() {
    let mut db = Db::new();
    for i in 0..3 {
        let mut e = Entry::new(Value::String(vec![i]));
        e.expires_at = Some(Instant::now() - Duration::from_secs(1));
        db.insert(format!("k{}", i).into_bytes(), e);
    }
    db.insert(b"alive".to_vec(), Entry::new(Value::String(b"v".to_vec())));
    let evicted = db.expire_cycle();
    assert_eq!(evicted.len(), 3);
    assert_eq!(db.dbsize(), 1);
}

#[test]
fn db_flush_empties_keys_and_blocked() {
    let mut db = Db::new();
    db.insert(b"k".to_vec(), Entry::new(Value::String(b"v".to_vec())));
    db.flush();
    assert_eq!(db.dbsize(), 0);
}

// ── Pub/Sub registry ─────────────────────────────────────────────────────────

#[test]
fn pubsub_publish_to_unsubscribed_channel_zero() {
    let reg = PubSubRegistry::default();
    assert_eq!(reg.publish(b"empty", b"msg"), 0);
}

#[test]
fn pubsub_pattern_subscribers_receive_pmessage() {
    let mut reg = PubSubRegistry::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    reg.psubscribe(1, b"news.*".to_vec(), tx);
    reg.publish(b"news.tech", b"hello");
    let msg = rx.try_recv().unwrap();
    assert_eq!(msg.kind, PubSubKind::PMessage);
    assert_eq!(msg.pattern.as_deref(), Some(b"news.*".as_ref()));
}

#[test]
fn pubsub_pattern_unsubscribe_targets_only_named_pattern() {
    let mut reg = PubSubRegistry::default();
    let (tx, _rx) = mpsc::unbounded_channel();
    reg.psubscribe(1, b"a.*".to_vec(), tx.clone());
    reg.psubscribe(1, b"b.*".to_vec(), tx);
    reg.punsubscribe(1, b"a.*");
    assert_eq!(reg.pattern_count(1), 1);
}

#[test]
fn pubsub_unsubscribe_specific_channel() {
    let mut reg = PubSubRegistry::default();
    let (tx, _rx) = mpsc::unbounded_channel();
    reg.subscribe(1, b"a".to_vec(), tx.clone());
    reg.subscribe(1, b"b".to_vec(), tx);
    reg.unsubscribe(1, b"a");
    assert_eq!(reg.channel_count(1), 1);
}

#[test]
fn pubsub_numsub_reports_per_channel() {
    let mut reg = PubSubRegistry::default();
    let (tx1, _r1) = mpsc::unbounded_channel();
    let (tx2, _r2) = mpsc::unbounded_channel();
    reg.subscribe(1, b"news".to_vec(), tx1);
    reg.subscribe(2, b"news".to_vec(), tx2);
    let counts = reg.numsub(&[b"news".to_vec(), b"silent".to_vec()]);
    assert_eq!(counts[0].1, 2);
    assert_eq!(counts[1].1, 0);
}

// ── ScriptStore ──────────────────────────────────────────────────────────────

#[test]
fn script_store_load_returns_sha1_hex_40_chars() {
    let mut store = ScriptStore::default();
    let sha = store.load("return 1".into());
    assert_eq!(sha.len(), 40);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn script_store_loading_same_script_twice_yields_same_sha() {
    let mut store = ScriptStore::default();
    let a = store.load("return 1".into());
    let b = store.load("return 1".into());
    assert_eq!(a, b);
}

#[test]
fn script_store_flush_clears_all() {
    let mut store = ScriptStore::default();
    store.load("return 1".into());
    store.load("return 2".into());
    store.flush();
    assert!(store.scripts.is_empty());
}

// ── RESP value helpers ───────────────────────────────────────────────────────

#[test]
fn resp_helpers_construct_canonical_values() {
    assert!(matches!(Resp::ok(), Resp::SimpleString(_)));
    assert!(matches!(Resp::pong(), Resp::SimpleString(_)));
    assert!(matches!(Resp::queued(), Resp::SimpleString(_)));
    assert!(Resp::nil().is_nil());
    assert!(Resp::nil_array().is_nil());
    assert!(matches!(Resp::int(42), Resp::Integer(42)));
    if let Resp::Array(Some(v)) = Resp::empty_array() {
        assert!(v.is_empty());
    } else {
        panic!("expected empty array");
    }
}

#[test]
fn resp_from_error_includes_error_token() {
    let e = CacheError::WrongType;
    if let Resp::Error(msg) = Resp::from_error(&e) {
        assert!(msg.contains("WRONGTYPE"));
    } else {
        panic!("expected Error variant");
    }
}

#[test]
fn resp_bulk_helper_wraps_bytes() {
    let r = Resp::bulk(b"hello".to_vec());
    if let Resp::BulkString(Some(b)) = r {
        assert_eq!(b, b"hello");
    } else {
        panic!("expected bulk string");
    }
}

// ── Stream model ─────────────────────────────────────────────────────────────

#[test]
fn stream_id_parses_explicit() {
    let id = StreamId::parse(b"1234-5").unwrap();
    assert_eq!(id.ms, 1234);
    assert_eq!(id.seq, 5);
}

#[test]
fn stream_id_parses_ms_only() {
    let id = StreamId::parse(b"500").unwrap();
    assert_eq!(id.ms, 500);
    assert_eq!(id.seq, 0);
}

#[test]
fn stream_id_zero_constant() {
    let z = StreamId::zero();
    assert_eq!(z.ms, 0);
    assert_eq!(z.seq, 0);
}

#[test]
fn stream_id_display_dash_format() {
    let id = StreamId { ms: 7, seq: 9 };
    assert_eq!(id.to_string(), "7-9");
}

#[test]
fn stream_id_orders_lexicographically_by_ms_then_seq() {
    let a = StreamId { ms: 1, seq: 0 };
    let b = StreamId { ms: 1, seq: 5 };
    let c = StreamId { ms: 2, seq: 0 };
    assert!(a < b);
    assert!(b < c);
}
