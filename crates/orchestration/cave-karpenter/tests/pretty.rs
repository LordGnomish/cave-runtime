// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 10 (continuation ray #3): port of pkg/utils/pretty/pretty.go
// from kubernetes-sigs/karpenter v1.12.1 (sha ed490e8) — the log-formatting
// helpers (Concise/Slice/Map/Taint/ToSnakeCase/Sentence). Pure; the camelCase
// regexes are reproduced by hand to keep the crate regex-free.

use std::collections::BTreeMap;

use cave_karpenter::pretty::{concise, map, sentence, slice, taint, to_snake_case};
use cave_karpenter::scheduling::taints::{Effect, Taint};

#[test]
fn concise_marshals_to_json() {
    assert_eq!(concise(&vec![1, 2, 3]), "[1,2,3]");
    assert_eq!(concise(&"hi"), "\"hi\"");
}

#[test]
fn slice_truncates_after_max_items() {
    assert_eq!(slice(&["a", "b", "c"], 2), "a, b and 1 other(s)");
    assert_eq!(slice(&["a", "b"], 2), "a, b");
    assert_eq!(slice(&["a"], 5), "a");
    assert_eq!(slice::<&str>(&[], 5), "");
}

#[test]
fn map_sorts_keys_and_truncates() {
    let mut m = BTreeMap::new();
    m.insert("b", 2);
    m.insert("a", 1);
    m.insert("c", 3);
    assert_eq!(map(&m, 2), "a: 1, b: 2 and 1 other(s)");
    assert_eq!(map(&m, 5), "a: 1, b: 2, c: 3");
}

#[test]
fn taint_formats_with_and_without_value() {
    let t1 = Taint {
        key: "dedicated".to_string(),
        value: Some("gpu".to_string()),
        effect: Effect::NoSchedule,
    };
    assert_eq!(taint(&t1), "dedicated=gpu:NoSchedule");
    let t2 = Taint {
        key: "spot".to_string(),
        value: None,
        effect: Effect::NoExecute,
    };
    assert_eq!(taint(&t2), "spot:NoExecute");
}

#[test]
fn to_snake_case_handles_camel_and_acronyms() {
    assert_eq!(to_snake_case("ToSnakeCase"), "to_snake_case");
    assert_eq!(to_snake_case("HTTPServer"), "http_server");
    assert_eq!(to_snake_case("already_snake"), "already_snake");
    assert_eq!(to_snake_case("simpleWord"), "simple_word");
}

#[test]
fn sentence_capitalizes_first_char() {
    assert_eq!(sentence("hello"), "Hello");
    assert_eq!(sentence("Hello"), "Hello");
    assert_eq!(sentence(""), "");
}
