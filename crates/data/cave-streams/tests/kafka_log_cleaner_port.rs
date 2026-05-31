// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Upstream behavioural-parity port — Apache Kafka 4.2.0
//! `kafka.log.LogCleanerManager.grabFilthiestCompactedLog` + `LogToClean`.
//!
//! The background log cleaner does not compact every log on every pass; it
//! ranks the compactable logs by their *cleanable ratio* —
//! `cleanableBytes / totalBytes` — and cleans the dirtiest one, but only when
//! that ratio exceeds the configured `min.cleanable.dirty.ratio` (default
//! 0.5).  Our log_compaction module could compact a single log on demand but
//! had no cross-log selection; this port adds Kafka's ranking contract.

use cave_streams::log_compaction::{grab_filthiest_compacted_log, LogToClean};

#[test]
fn cleanable_ratio_is_cleanable_over_total() {
    assert_eq!(LogToClean::new("a", 50, 50).cleanable_ratio(), 0.5);
    assert_eq!(LogToClean::new("b", 25, 75).cleanable_ratio(), 0.75);
    assert_eq!(LogToClean::new("c", 75, 25).cleanable_ratio(), 0.25);
    // Empty log → 0.0 (no divide-by-zero).
    assert_eq!(LogToClean::new("d", 0, 0).cleanable_ratio(), 0.0);
}

#[test]
fn total_bytes_sums_clean_and_cleanable() {
    assert_eq!(LogToClean::new("a", 30, 70).total_bytes(), 100);
}

#[test]
fn grab_filthiest_picks_max_ratio_above_threshold() {
    let candidates = vec![
        LogToClean::new("low", 70, 30),  // 0.30
        LogToClean::new("high", 30, 70), // 0.70
        LogToClean::new("mid", 40, 60),  // 0.60
    ];
    let picked = grab_filthiest_compacted_log(&candidates, 0.5)
        .expect("a log above threshold must be selected");
    assert_eq!(picked.name, "high");
}

#[test]
fn grab_filthiest_returns_none_when_all_below_threshold() {
    let candidates = vec![
        LogToClean::new("a", 70, 30), // 0.30
        LogToClean::new("b", 60, 40), // 0.40
    ];
    assert!(grab_filthiest_compacted_log(&candidates, 0.5).is_none());
}

#[test]
fn grab_filthiest_returns_none_for_empty_candidate_set() {
    let candidates: Vec<LogToClean> = Vec::new();
    assert!(grab_filthiest_compacted_log(&candidates, 0.5).is_none());
}

#[test]
fn grab_filthiest_threshold_is_strict_and_ties_keep_first() {
    // Exactly at the threshold does not qualify (Kafka uses `>`).
    let at_threshold = vec![LogToClean::new("eq", 50, 50)]; // 0.50
    assert!(grab_filthiest_compacted_log(&at_threshold, 0.5).is_none());

    // Two equal maxima → the first one wins (Scala maxBy is first-max).
    let ties = vec![
        LogToClean::new("first", 20, 80),  // 0.80
        LogToClean::new("second", 20, 80), // 0.80
    ];
    assert_eq!(
        grab_filthiest_compacted_log(&ties, 0.5).unwrap().name,
        "first"
    );
}
