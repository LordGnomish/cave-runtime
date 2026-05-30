// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral parity port of the high-level Kafka Streams DSL
//! (`org.apache.kafka.streams.kstream`) — KStream / KTable / KGroupedStream.
//!
//! upstream: apache/kafka — streams/src/main/java/org/apache/kafka/streams/kstream
//! (StreamsBuilder, KStream, KTable, KGroupedStream, Materialized, TimeWindows)
//!
//! The execution model mirrors `TopologyTestDriver`: `pipe_input` pushes one
//! record through the operator graph; `drain_output` collects what reached a
//! sink; `store_get` queries a materialized state store.

use cave_streams::kafka_streams_dsl::{DslPredicate, Record, StreamsBuilder};

fn rec_values(recs: &[Record]) -> Vec<Vec<u8>> {
    recs.iter().map(|r| r.value.clone()).collect()
}

// ─── Cycle 1: stateless KStream operators ────────────────────────────────────

#[test]
fn filter_keeps_only_matching_records() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .filter(|r| r.value.starts_with(b"keep"))
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k1", b"keep-me", 0);
    app.pipe_input("in", b"k2", b"drop-me", 0);
    app.pipe_input("in", b"k3", b"keep-too", 0);

    let out = app.drain_output("out");
    assert_eq!(rec_values(&out), vec![b"keep-me".to_vec(), b"keep-too".to_vec()]);
}

#[test]
fn filter_not_is_the_negation() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .filter_not(|r| r.value.starts_with(b"drop"))
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k1", b"keep", 0);
    app.pipe_input("in", b"k2", b"drop-this", 0);

    assert_eq!(rec_values(&app.drain_output("out")), vec![b"keep".to_vec()]);
}

#[test]
fn map_values_transforms_value_keeps_key() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .map_values(|v| v.to_ascii_uppercase())
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"key", b"hello", 7);
    let out = app.drain_output("out");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].value, b"HELLO");
    assert_eq!(out[0].key, b"key"); // key preserved
    assert_eq!(out[0].timestamp_ms, 7); // timestamp preserved
}

#[test]
fn map_can_rewrite_both_key_and_value() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .map(|r| Record::new(&r.value, &r.key, r.timestamp_ms))
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"K", b"V", 3);
    let out = app.drain_output("out");
    assert_eq!(out[0].key, b"V");
    assert_eq!(out[0].value, b"K");
}

#[test]
fn flat_map_values_fans_one_record_into_many() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .flat_map_values(|v| {
            String::from_utf8_lossy(v)
                .split(',')
                .map(|s| s.as_bytes().to_vec())
                .collect()
        })
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"a,b,c", 0);
    assert_eq!(
        rec_values(&app.drain_output("out")),
        vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]
    );
}

#[test]
fn flat_map_can_drop_to_zero_records() {
    let b = StreamsBuilder::new();
    b.stream("in").flat_map(|_r| Vec::<Record>::new()).to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"v", 0);
    assert!(app.drain_output("out").is_empty());
}

#[test]
fn select_key_rewrites_key_only() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .select_key(|r| r.value.clone())
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"orig", b"newkey", 0);
    let out = app.drain_output("out");
    assert_eq!(out[0].key, b"newkey");
    assert_eq!(out[0].value, b"newkey");
}

#[test]
fn peek_observes_without_mutating() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let seen = Rc::new(RefCell::new(0u32));
    let seen2 = seen.clone();

    let b = StreamsBuilder::new();
    b.stream("in")
        .peek(move |_r| *seen2.borrow_mut() += 1)
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"v1", 0);
    app.pipe_input("in", b"k", b"v2", 0);
    assert_eq!(*seen.borrow(), 2);
    assert_eq!(app.drain_output("out").len(), 2); // pass-through
}

#[test]
fn foreach_is_terminal_no_downstream() {
    let b = StreamsBuilder::new();
    // foreach returns () — chain ends, no sink reachable.
    b.stream("in").foreach(|_r| {});
    let mut app = b.build();
    app.pipe_input("in", b"k", b"v", 0);
    // No sink registered → draining an unknown topic yields empty.
    assert!(app.drain_output("out").is_empty());
}

#[test]
fn chained_operators_compose_in_order() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .filter(|r| r.value.len() > 2)
        .map_values(|v| {
            let mut x = v.to_vec();
            x.reverse();
            x
        })
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"ab", 0); // filtered out (len 2)
    app.pipe_input("in", b"k", b"abcd", 0); // -> "dcba"
    assert_eq!(rec_values(&app.drain_output("out")), vec![b"dcba".to_vec()]);
}

#[test]
fn unknown_source_topic_is_ignored() {
    let b = StreamsBuilder::new();
    b.stream("in").to("out");
    let mut app = b.build();
    app.pipe_input("nonexistent", b"k", b"v", 0);
    assert!(app.drain_output("out").is_empty());
}

#[test]
fn drain_is_destructive() {
    let b = StreamsBuilder::new();
    b.stream("in").to("out");
    let mut app = b.build();
    app.pipe_input("in", b"k", b"v", 0);
    assert_eq!(app.drain_output("out").len(), 1);
    assert!(app.drain_output("out").is_empty()); // drained
}

// ─── Cycle 2: branch / merge / through ───────────────────────────────────────

#[test]
fn branch_routes_to_first_matching_predicate() {
    let b = StreamsBuilder::new();
    let preds: Vec<DslPredicate> = vec![
        Box::new(|r: &Record| r.value.starts_with(b"a")),
        Box::new(|r: &Record| r.value.starts_with(b"b")),
    ];
    let branches = b.stream("in").branch(preds);
    branches[0].to("as");
    branches[1].to("bs");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"apple", 0);
    app.pipe_input("in", b"k", b"banana", 0);
    app.pipe_input("in", b"k", b"avocado", 0);

    assert_eq!(
        rec_values(&app.drain_output("as")),
        vec![b"apple".to_vec(), b"avocado".to_vec()]
    );
    assert_eq!(rec_values(&app.drain_output("bs")), vec![b"banana".to_vec()]);
}

#[test]
fn branch_is_mutually_exclusive_first_match_wins() {
    let b = StreamsBuilder::new();
    let preds: Vec<DslPredicate> = vec![
        Box::new(|_r: &Record| true), // matches everything first
        Box::new(|_r: &Record| true),
    ];
    let branches = b.stream("in").branch(preds);
    branches[0].to("first");
    branches[1].to("second");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"x", 0);
    assert_eq!(app.drain_output("first").len(), 1);
    assert!(app.drain_output("second").is_empty()); // first match consumed it
}

#[test]
fn branch_drops_records_matching_no_predicate() {
    let b = StreamsBuilder::new();
    let preds: Vec<DslPredicate> = vec![Box::new(|r: &Record| r.value == b"keep")];
    let branches = b.stream("in").branch(preds);
    branches[0].to("kept");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"keep", 0);
    app.pipe_input("in", b"k", b"nomatch", 0);
    assert_eq!(rec_values(&app.drain_output("kept")), vec![b"keep".to_vec()]);
}

#[test]
fn merge_interleaves_two_streams() {
    let b = StreamsBuilder::new();
    let s1 = b.stream("in1").map_values(|v| [b"1:".as_ref(), v].concat());
    let s2 = b.stream("in2").map_values(|v| [b"2:".as_ref(), v].concat());
    s1.merge(&s2).to("out");
    let mut app = b.build();

    app.pipe_input("in1", b"k", b"x", 0);
    app.pipe_input("in2", b"k", b"y", 0);
    app.pipe_input("in1", b"k", b"z", 0);

    assert_eq!(
        rec_values(&app.drain_output("out")),
        vec![b"1:x".to_vec(), b"2:y".to_vec(), b"1:z".to_vec()]
    );
}

#[test]
fn through_persists_then_continues_downstream() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .through("mid")
        .map_values(|v| v.to_ascii_uppercase())
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"hi", 0);
    // `through` writes the pre-transform record to the intermediate topic ...
    assert_eq!(rec_values(&app.drain_output("mid")), vec![b"hi".to_vec()]);
    // ... and the stream continues through the rest of the topology.
    assert_eq!(rec_values(&app.drain_output("out")), vec![b"HI".to_vec()]);
}

// ─── Cycle 3: groupByKey/groupBy → count/reduce/aggregate → KTable ────────────

#[test]
fn count_maintains_a_per_key_tally() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .group_by_key()
        .count("counts")
        .to_stream()
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"a", b"_", 0);
    app.pipe_input("in", b"a", b"_", 0);
    app.pipe_input("in", b"b", b"_", 0);

    // Store holds the live tally per key (decimal-encoded Long).
    assert_eq!(app.store_get("counts", b"a"), Some(b"2".to_vec()));
    assert_eq!(app.store_get("counts", b"b"), Some(b"1".to_vec()));

    // Changelog stream emits the updated count on every input.
    let out = app.drain_output("out");
    assert_eq!(rec_values(&out), vec![b"1".to_vec(), b"2".to_vec(), b"1".to_vec()]);
    // The count changelog keeps the grouping key.
    assert_eq!(out[1].key, b"a");
}

#[test]
fn reduce_combines_values_per_key() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .group_by_key()
        .reduce("reduced", |agg, next| {
            let mut v = agg.to_vec();
            v.push(b'+');
            v.extend_from_slice(next);
            v
        })
        .to_stream()
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"a", 0);
    app.pipe_input("in", b"k", b"b", 0);
    app.pipe_input("in", b"k", b"c", 0);

    // First value initializes; subsequent values fold via the reducer.
    assert_eq!(app.store_get("reduced", b"k"), Some(b"a+b+c".to_vec()));
    assert_eq!(
        rec_values(&app.drain_output("out")),
        vec![b"a".to_vec(), b"a+b".to_vec(), b"a+b+c".to_vec()]
    );
}

#[test]
fn aggregate_with_initializer_and_aggregator() {
    let b = StreamsBuilder::new();
    // Aggregate the running sum of value lengths, decimal-encoded.
    b.stream("in")
        .group_by_key()
        .aggregate(
            "sum",
            || b"0".to_vec(),
            |_k, value, current| {
                let cur: i64 = String::from_utf8_lossy(current).parse().unwrap_or(0);
                (cur + value.len() as i64).to_string().into_bytes()
            },
        )
        .to_stream()
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"xx", 0); // +2 -> 2
    app.pipe_input("in", b"k", b"yyy", 0); // +3 -> 5
    assert_eq!(app.store_get("sum", b"k"), Some(b"5".to_vec()));
    assert_eq!(
        rec_values(&app.drain_output("out")),
        vec![b"2".to_vec(), b"5".to_vec()]
    );
}

#[test]
fn group_by_rekeys_before_grouping() {
    let b = StreamsBuilder::new();
    // Re-key by value's first byte, then count per new key.
    b.stream("in")
        .group_by(|r| vec![r.value[0]])
        .count("byprefix")
        .to_stream()
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"orig1", b"apple", 0); // key -> "a"
    app.pipe_input("in", b"orig2", b"avocado", 0); // key -> "a"
    app.pipe_input("in", b"orig3", b"banana", 0); // key -> "b"

    assert_eq!(app.store_get("byprefix", b"a"), Some(b"2".to_vec()));
    assert_eq!(app.store_get("byprefix", b"b"), Some(b"1".to_vec()));
}

#[test]
fn ktable_to_stream_then_filter() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .group_by_key()
        .count("c")
        .to_stream()
        .filter(|r| r.value != b"1") // suppress the first observation per key
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"_", 0); // count 1 -> suppressed
    app.pipe_input("in", b"k", b"_", 0); // count 2 -> emitted
    assert_eq!(rec_values(&app.drain_output("out")), vec![b"2".to_vec()]);
}

#[test]
fn ktable_map_values_transforms_changelog() {
    let b = StreamsBuilder::new();
    b.stream("in")
        .group_by_key()
        .count("c")
        .map_values(|v| [b"count=".as_ref(), v].concat())
        .to_stream()
        .to("out");
    let mut app = b.build();

    app.pipe_input("in", b"k", b"_", 0);
    assert_eq!(
        rec_values(&app.drain_output("out")),
        vec![b"count=1".to_vec()]
    );
}
