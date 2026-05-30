// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line parity tests for the relabeling engine, ported from
//! prometheus/prometheus `model/relabel/relabel_test.go` (v3.12.0,
//! source_sha a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! Each test mirrors a behavior of upstream's `relabel()` switch: the 11
//! actions (replace / keep / drop / keepequal / dropequal / hashmod /
//! labelmap / labeldrop / labelkeep / lowercase / uppercase), regex
//! `${N}` template expansion, multi-source concatenation with separator,
//! and the empty-value fast-path.

use cave_metrics::model::Labels;
use cave_metrics::scrape::relabel::{Action, RelabelConfig, process};

fn cfg() -> RelabelConfig {
    RelabelConfig::default()
}

// ── Replace ────────────────────────────────────────────────────────────────

#[test]
fn replace_expands_regex_template() {
    // upstream: a="foo", regex f(.*), target d, replacement "ch${1}-ch${1}".
    let mut lb = Labels::from_pairs([("a", "foo"), ("b", "bar"), ("c", "baz")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["a".into()],
            regex: "f(.*)".into(),
            target_label: "d".into(),
            replacement: "ch${1}-ch${1}".into(),
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("d"), Some("choo-choo"));
    // untouched labels survive
    assert_eq!(lb.get("a"), Some("foo"));
    assert_eq!(lb.get("b"), Some("bar"));
}

#[test]
fn replace_multi_source_concatenation() {
    // upstream second case: two chained replaces → a=boobam, d=boooom.
    let mut lb = Labels::from_pairs([("a", "foo"), ("b", "bar"), ("c", "baz")]);
    let keep = process(
        &mut lb,
        &[
            RelabelConfig {
                source_labels: vec!["a".into(), "b".into()],
                regex: "f(.*);(.*)r".into(),
                target_label: "a".into(),
                replacement: "b${1}${2}m".into(),
                ..cfg()
            },
            RelabelConfig {
                source_labels: vec!["c".into(), "a".into()],
                regex: "(b).*b(.*)ba(.*)".into(),
                target_label: "d".into(),
                replacement: "$1$2$2$3".into(),
                ..cfg()
            },
        ],
    );
    assert!(keep);
    assert_eq!(lb.get("a"), Some("boobam"));
    assert_eq!(lb.get("d"), Some("boooom"));
}

#[test]
fn replace_empty_value_fast_path_sets_label() {
    // No source labels → concat is "" → default regex (.*) matches → the
    // fast-path simply sets target=replacement (used to inject static labels).
    let mut lb = Labels::from_pairs([("a", "foo")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec![],
            target_label: "injected".into(),
            replacement: "static-value".into(),
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("injected"), Some("static-value"));
}

#[test]
fn replace_no_match_leaves_labels_unchanged() {
    let mut lb = Labels::from_pairs([("a", "foo")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["a".into()],
            regex: "will-not-match".into(),
            target_label: "d".into(),
            replacement: "$1".into(),
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("d"), None);
}

// ── Keep / Drop ──────────────────────────────────────────────────────────────

#[test]
fn keep_drops_target_when_regex_does_not_match() {
    let mut lb = Labels::from_pairs([("env", "staging")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["env".into()],
            regex: "prod".into(),
            action: Action::Keep,
            ..cfg()
        }],
    );
    assert!(!keep, "keep must drop the target when the regex does not match");
}

#[test]
fn keep_retains_target_when_regex_matches() {
    let mut lb = Labels::from_pairs([("env", "prod")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["env".into()],
            regex: "prod".into(),
            action: Action::Keep,
            ..cfg()
        }],
    );
    assert!(keep);
}

#[test]
fn drop_removes_target_when_regex_matches() {
    let mut lb = Labels::from_pairs([("env", "prod")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["env".into()],
            regex: "prod".into(),
            action: Action::Drop,
            ..cfg()
        }],
    );
    assert!(!keep);
}

// ── DropEqual / KeepEqual ────────────────────────────────────────────────────

#[test]
fn dropequal_drops_when_source_equals_target() {
    let mut lb = Labels::from_pairs([("a", "v"), ("b", "v")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["a".into()],
            target_label: "b".into(),
            action: Action::DropEqual,
            ..cfg()
        }],
    );
    assert!(!keep, "dropequal removes the target when src concat == target label");
}

#[test]
fn keepequal_keeps_when_source_equals_target() {
    let mut lb = Labels::from_pairs([("a", "v"), ("b", "v")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["a".into()],
            target_label: "b".into(),
            action: Action::KeepEqual,
            ..cfg()
        }],
    );
    assert!(keep);
}

// ── HashMod ──────────────────────────────────────────────────────────────────

#[test]
fn hashmod_uses_last_8_bytes_of_md5_modulo_modulus() {
    // md5("foo") last-8-bytes-BE % 1000 == 696 (computed from upstream's algo).
    let mut lb = Labels::from_pairs([("a", "foo")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            source_labels: vec!["a".into()],
            target_label: "shard".into(),
            modulus: 1000,
            action: Action::HashMod,
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("shard"), Some("696"));
}

// ── LabelMap / LabelDrop / LabelKeep ─────────────────────────────────────────

#[test]
fn labelmap_copies_matching_labels_to_new_names() {
    let mut lb = Labels::from_pairs([("__meta_kubernetes_pod", "p"), ("job", "api")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            regex: "__meta_kubernetes_(.*)".into(),
            replacement: "$1".into(),
            action: Action::LabelMap,
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("pod"), Some("p"));
    // original is preserved (labelmap copies, does not move)
    assert_eq!(lb.get("__meta_kubernetes_pod"), Some("p"));
}

#[test]
fn labeldrop_removes_matching_labels() {
    let mut lb = Labels::from_pairs([("__tmp_x", "1"), ("__tmp_y", "2"), ("keep", "3")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            regex: "__tmp_.*".into(),
            action: Action::LabelDrop,
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("__tmp_x"), None);
    assert_eq!(lb.get("__tmp_y"), None);
    assert_eq!(lb.get("keep"), Some("3"));
}

#[test]
fn labelkeep_removes_non_matching_labels() {
    let mut lb = Labels::from_pairs([("keep_a", "1"), ("keep_b", "2"), ("drop_c", "3")]);
    let keep = process(
        &mut lb,
        &[RelabelConfig {
            regex: "keep_.*".into(),
            action: Action::LabelKeep,
            ..cfg()
        }],
    );
    assert!(keep);
    assert_eq!(lb.get("keep_a"), Some("1"));
    assert_eq!(lb.get("keep_b"), Some("2"));
    assert_eq!(lb.get("drop_c"), None);
}

// ── Lowercase / Uppercase ────────────────────────────────────────────────────

#[test]
fn lowercase_and_uppercase_transform_concatenation() {
    let mut lb = Labels::from_pairs([("a", "MixedCase")]);
    process(
        &mut lb,
        &[
            RelabelConfig {
                source_labels: vec!["a".into()],
                target_label: "lower".into(),
                action: Action::Lowercase,
                ..cfg()
            },
            RelabelConfig {
                source_labels: vec!["a".into()],
                target_label: "upper".into(),
                action: Action::Uppercase,
                ..cfg()
            },
        ],
    );
    assert_eq!(lb.get("lower"), Some("mixedcase"));
    assert_eq!(lb.get("upper"), Some("MIXEDCASE"));
}

#[test]
fn chain_returns_false_as_soon_as_one_rule_drops() {
    // First rule keeps, second drops → overall keep == false and processing stops.
    let mut lb = Labels::from_pairs([("env", "prod"), ("team", "infra")]);
    let keep = process(
        &mut lb,
        &[
            RelabelConfig {
                source_labels: vec!["env".into()],
                regex: "prod".into(),
                action: Action::Keep,
                ..cfg()
            },
            RelabelConfig {
                source_labels: vec!["team".into()],
                regex: "infra".into(),
                action: Action::Drop,
                ..cfg()
            },
        ],
    );
    assert!(!keep);
}
