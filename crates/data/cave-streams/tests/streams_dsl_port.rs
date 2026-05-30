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

use cave_streams::kafka_streams_dsl::{Record, StreamsBuilder};

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
