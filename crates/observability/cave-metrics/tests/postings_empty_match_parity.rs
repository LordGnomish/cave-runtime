// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for Prometheus `PostingsForMatchers` empty-matching semantics.
//!
//! Upstream: prometheus/prometheus `tsdb/querier.go` `PostingsForMatchers` /
//! `inversePostingsForMatcher` — see issue prometheus/prometheus#3575 and
//! PR #3578: "If a matcher selects an empty value, it also selects all series
//! that do not have the label set at all."
//!
//! The TSDB inverted index must therefore resolve a matcher that matches the
//! empty string (`env=""`, `env=~"prod|"`, `env!~".+"`, …) against series that
//! lack the label entirely — not only against series carrying an explicit
//! empty value.

use cave_metrics::model::{LabelMatcher, Labels, Sample};
use cave_metrics::tsdb::{Tsdb, TsdbConfig};

/// Build a db with three `up` series: one lacking `env`, one `env=prod`, one `env=dev`.
fn seed() -> (Tsdb, i64) {
    let db = Tsdb::new(TsdbConfig::default());
    let ts = 1_700_000_000_000i64;
    db.append(
        Labels::from_pairs([("__name__", "up"), ("job", "api")]),
        Sample::new(ts, 1.0),
    );
    db.append(
        Labels::from_pairs([("__name__", "up"), ("job", "api"), ("env", "prod")]),
        Sample::new(ts, 1.0),
    );
    db.append(
        Labels::from_pairs([("__name__", "up"), ("job", "api"), ("env", "dev")]),
        Sample::new(ts, 1.0),
    );
    (db, ts)
}

fn env_values(res: &[(Labels, Vec<Sample>)]) -> Vec<Option<String>> {
    let mut v: Vec<Option<String>> = res
        .iter()
        .map(|(l, _)| l.get("env").map(|s| s.to_string()))
        .collect();
    v.sort();
    v
}

#[test]
fn equal_empty_matches_series_without_label() {
    let (db, ts) = seed();
    let m = vec![
        LabelMatcher::equal("__name__", "up"),
        LabelMatcher::equal("env", ""),
    ];
    let res = db.select(&m, ts - 1000, ts + 1000);
    // `env=""` selects exactly the series lacking `env`.
    assert_eq!(env_values(&res), vec![None], "env=\"\" must select the series with no env label");
}

#[test]
fn regex_matching_empty_includes_absent_label() {
    let (db, ts) = seed();
    let m = vec![
        LabelMatcher::equal("__name__", "up"),
        LabelMatcher::regex("env", "prod|").unwrap(),
    ];
    let res = db.select(&m, ts - 1000, ts + 1000);
    // `env=~"prod|"` matches empty → absent series + prod, but NOT dev.
    assert_eq!(
        env_values(&res),
        vec![None, Some("prod".to_string())],
        "env=~\"prod|\" must select the absent series and prod, not dev"
    );
}

#[test]
fn not_equal_selects_absent_label() {
    let (db, ts) = seed();
    let m = vec![
        LabelMatcher::equal("__name__", "up"),
        LabelMatcher::not_equal("env", "prod"),
    ];
    let res = db.select(&m, ts - 1000, ts + 1000);
    // `env!="prod"` matches absent (env="") and dev, not prod.
    assert_eq!(
        env_values(&res),
        vec![None, Some("dev".to_string())],
        "env!=\"prod\" must select absent + dev"
    );
}

#[test]
fn not_regex_requiring_nonempty_excludes_absent() {
    let (db, ts) = seed();
    let m = vec![
        LabelMatcher::equal("__name__", "up"),
        // env!~".+" matches the empty string → selects absent + (any with empty value)
        LabelMatcher::not_regex("env", ".+").unwrap(),
    ];
    let res = db.select(&m, ts - 1000, ts + 1000);
    assert_eq!(
        env_values(&res),
        vec![None],
        "env!~\".+\" matches empty → only the absent series"
    );
}

#[test]
fn positive_equal_still_exact() {
    let (db, ts) = seed();
    let m = vec![
        LabelMatcher::equal("__name__", "up"),
        LabelMatcher::equal("env", "prod"),
    ];
    let res = db.select(&m, ts - 1000, ts + 1000);
    assert_eq!(env_values(&res), vec![Some("prod".to_string())]);
}
