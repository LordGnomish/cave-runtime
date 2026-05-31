// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Upstream behavioural-parity port — Apache Kafka 4.2.0
//! `org.apache.kafka.common.utils.Utils.murmur2` +
//! `org.apache.kafka.clients.producer.internals.DefaultPartitioner`.
//!
//! Kafka's default producer partitioner hashes the record key with the
//! 32-bit MurmurHash2 variant, folds the sign bit away with `toPositive`,
//! and takes the result modulo the partition count.  Our producer only
//! shipped an FNV-1a key hash; this port adds the wire-compatible
//! `kafka_murmur2`/`to_positive`/`default_partition` so a Kafka client that
//! pre-computes its own partition lands on the same one cave-streams does.
//!
//! Test vectors are the canonical `UtilsTest.testMurmur2()` constants from
//! the Kafka source tree (clients/src/test/.../common/utils/UtilsTest.java).

use cave_streams::models::PartitionerStrategy;
use cave_streams::producer::{choose_partition, default_partition, kafka_murmur2, to_positive};

#[test]
fn murmur2_matches_kafka_utils_test_vectors() {
    // Exact constants asserted in Apache Kafka's UtilsTest.testMurmur2().
    assert_eq!(kafka_murmur2(b"21"), -1758910492);
    assert_eq!(kafka_murmur2(b"foobar"), -1067176724);
    assert_eq!(kafka_murmur2(b"a-little-bit-long-string"), -1240745061);
    assert_eq!(kafka_murmur2(b"a-little-bit-longer-string"), -1106571547);
    assert_eq!(
        kafka_murmur2(b"lkjh234lh9fiuh90y23oiuhsafujhadof229phr9h19h89h8"),
        -1011670077
    );
}

#[test]
fn to_positive_clears_only_the_sign_bit() {
    assert_eq!(to_positive(-1), i32::MAX);
    assert_eq!(to_positive(0), 0);
    assert_eq!(to_positive(123), 123);
    // toPositive(murmur2("21")) == -1758910492 & 0x7fffffff == 388573156.
    assert_eq!(to_positive(kafka_murmur2(b"21")), 388573156);
}

#[test]
fn default_partition_is_to_positive_murmur2_mod_n() {
    // 388573156 % 10 == 6
    assert_eq!(default_partition(b"21", 10), 6);
    // toPositive(murmur2("foobar")) == 1080306924; % 100 == 24
    assert_eq!(default_partition(b"foobar", 100), 24);
    // Always in range.
    for n in 1u32..=64 {
        assert!(default_partition(b"some-key", n) < n);
    }
}

#[test]
fn murmur2_strategy_uses_kafka_partitioner_for_keyed_records() {
    let key = b"foobar";
    let p = choose_partition(&PartitionerStrategy::Murmur2, Some(key), 100, 7);
    assert_eq!(p, 24, "keyed Murmur2 must match DefaultPartitioner");
}

#[test]
fn murmur2_strategy_falls_back_to_round_robin_without_key() {
    // Null/empty key → sticky round-robin, exactly like Kafka's DefaultPartitioner.
    let p = choose_partition(&PartitionerStrategy::Murmur2, None, 8, 11);
    assert_eq!(p, 11 % 8);
    let p_empty = choose_partition(&PartitionerStrategy::Murmur2, Some(b""), 8, 3);
    assert_eq!(p_empty, 3 % 8);
}
