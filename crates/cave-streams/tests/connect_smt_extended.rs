// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/RegexRouter.java
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/TimestampRouter.java
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/Flatten.java
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/ReplaceField.java

//! Extended SMT integration tests for the 4 new built-ins added by
//! the S2 close-out: `RegexRouter`, `TimestampRouter`, `Flatten`,
//! `ReplaceField`. Each upstream class has both a `$Value` and a
//! `$Key` variant — we ship the `$Value` variant (the more common
//! use case; the `$Key` variant is a trivial wrapper that operates
//! on the key field instead and is left for a follow-up batch).

use std::collections::BTreeMap;
use std::sync::Arc;

use cave_streams::connect_worker::smt::{
    flatten::Flatten, regex_router::RegexRouter, replace_field::ReplaceField,
    timestamp_router::TimestampRouter, RecordEnvelope, Smt, SmtChain, SmtRegistry, Value,
};

fn obj(kvs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in kvs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

// -- RegexRouter ------------------------------------------------------

#[test]
fn regex_router_renames_topic_via_capture_groups() {
    let mut cfg = BTreeMap::new();
    cfg.insert("regex".into(), r"^prod\.(.+)$".into());
    cfg.insert("replacement".into(), "staging.$1".into());
    let s = RegexRouter::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new("prod.orders", Value::Null);
    let out = s.apply(r).unwrap().unwrap();
    assert_eq!(out.topic, "staging.orders");
}

#[test]
fn regex_router_leaves_non_matching_topic_unchanged() {
    let mut cfg = BTreeMap::new();
    cfg.insert("regex".into(), r"^prod\.(.+)$".into());
    cfg.insert("replacement".into(), "staging.$1".into());
    let s = RegexRouter::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new("dev.orders", Value::Null);
    let out = s.apply(r).unwrap().unwrap();
    // Non-match: upstream's RegexRouter returns the topic untouched.
    assert_eq!(out.topic, "dev.orders");
}

#[test]
fn regex_router_invalid_pattern_errors_at_config() {
    let mut cfg = BTreeMap::new();
    cfg.insert("regex".into(), "[".into());
    cfg.insert("replacement".into(), "x".into());
    assert!(RegexRouter::from_config(&cfg).is_err());
}

// -- TimestampRouter --------------------------------------------------

#[test]
fn timestamp_router_appends_yyyymmdd_to_topic() {
    let mut cfg = BTreeMap::new();
    cfg.insert("topic.format".into(), "${topic}-${timestamp}".into());
    cfg.insert("timestamp.format".into(), "yyyyMMdd".into());
    let s = TimestampRouter::from_config(&cfg).unwrap();
    // 2026-05-16T00:00:00 UTC.
    let ts_ms: i64 = 1_779_004_800_000;
    let mut r = RecordEnvelope::new("events", Value::Null);
    r.timestamp_ms = Some(ts_ms);
    let out = s.apply(r).unwrap().unwrap();
    assert_eq!(out.topic, "events-20260516");
}

#[test]
fn timestamp_router_uses_now_when_record_has_no_timestamp() {
    let mut cfg = BTreeMap::new();
    cfg.insert("topic.format".into(), "${topic}.${timestamp}".into());
    cfg.insert("timestamp.format".into(), "yyyy".into());
    let s = TimestampRouter::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new("events", Value::Null);
    let out = s.apply(r).unwrap().unwrap();
    // Should be a 4-digit year prefixed with "events.".
    assert!(out.topic.starts_with("events."));
    let year_part = out.topic.strip_prefix("events.").unwrap();
    assert_eq!(year_part.len(), 4);
    assert!(year_part.chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn timestamp_router_missing_config_errors() {
    let cfg = BTreeMap::new();
    assert!(TimestampRouter::from_config(&cfg).is_err());
}

// -- Flatten ----------------------------------------------------------

#[test]
fn flatten_collapses_nested_object_with_default_delim() {
    let cfg = BTreeMap::new();
    let s = Flatten::from_config(&cfg).unwrap();
    let nested = obj(&[
        (
            "user",
            obj(&[
                ("name", Value::String("alice".into())),
                ("addr", obj(&[("city", Value::String("Istanbul".into()))])),
            ]),
        ),
        ("amount", Value::Int(42)),
    ]);
    let r = RecordEnvelope::new("t", nested);
    let out = s.apply(r).unwrap().unwrap();
    let m = out.value.as_object().unwrap();
    assert_eq!(m.get("user.name"), Some(&Value::String("alice".into())));
    assert_eq!(
        m.get("user.addr.city"),
        Some(&Value::String("Istanbul".into()))
    );
    assert_eq!(m.get("amount"), Some(&Value::Int(42)));
    assert!(m.get("user").is_none(), "nested object replaced by leaves");
}

#[test]
fn flatten_honours_custom_delimiter() {
    let mut cfg = BTreeMap::new();
    cfg.insert("delimiter".into(), "/".into());
    let s = Flatten::from_config(&cfg).unwrap();
    let nested = obj(&[(
        "a",
        obj(&[("b", obj(&[("c", Value::Int(7))]))]),
    )]);
    let r = RecordEnvelope::new("t", nested);
    let out = s.apply(r).unwrap().unwrap();
    let m = out.value.as_object().unwrap();
    assert_eq!(m.get("a/b/c"), Some(&Value::Int(7)));
}

#[test]
fn flatten_leaves_scalar_value_untouched() {
    let cfg = BTreeMap::new();
    let s = Flatten::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new("t", Value::String("scalar".into()));
    let out = s.apply(r).unwrap().unwrap();
    assert_eq!(out.value, Value::String("scalar".into()));
}

// -- ReplaceField ----------------------------------------------------

#[test]
fn replace_field_renames_listed_keys() {
    let mut cfg = BTreeMap::new();
    cfg.insert("renames".into(), "old:new,foo:bar".into());
    let s = ReplaceField::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new(
        "t",
        obj(&[
            ("old", Value::Int(1)),
            ("foo", Value::String("v".into())),
            ("keep", Value::Bool(true)),
        ]),
    );
    let out = s.apply(r).unwrap().unwrap();
    let m = out.value.as_object().unwrap();
    assert_eq!(m.get("new"), Some(&Value::Int(1)));
    assert_eq!(m.get("bar"), Some(&Value::String("v".into())));
    assert_eq!(m.get("keep"), Some(&Value::Bool(true)));
    assert!(m.get("old").is_none());
    assert!(m.get("foo").is_none());
}

#[test]
fn replace_field_exclude_drops_listed_fields() {
    let mut cfg = BTreeMap::new();
    cfg.insert("exclude".into(), "secret,internal".into());
    let s = ReplaceField::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new(
        "t",
        obj(&[
            ("secret", Value::String("pw".into())),
            ("internal", Value::String("debug".into())),
            ("public", Value::Int(1)),
        ]),
    );
    let out = s.apply(r).unwrap().unwrap();
    let m = out.value.as_object().unwrap();
    assert!(m.get("secret").is_none());
    assert!(m.get("internal").is_none());
    assert_eq!(m.get("public"), Some(&Value::Int(1)));
}

#[test]
fn replace_field_include_keeps_only_listed() {
    let mut cfg = BTreeMap::new();
    cfg.insert("include".into(), "a,b".into());
    let s = ReplaceField::from_config(&cfg).unwrap();
    let r = RecordEnvelope::new(
        "t",
        obj(&[
            ("a", Value::Int(1)),
            ("b", Value::Int(2)),
            ("c", Value::Int(3)),
        ]),
    );
    let out = s.apply(r).unwrap().unwrap();
    let m = out.value.as_object().unwrap();
    assert_eq!(m.len(), 2);
    assert!(m.contains_key("a"));
    assert!(m.contains_key("b"));
}

// -- registry + chain wire-up ---------------------------------------

#[test]
fn registry_with_defaults_now_has_ten_builtins() {
    let reg = SmtRegistry::with_defaults();
    // 6 original + 4 new = 10.
    assert!(
        reg.len() >= 10,
        "expected ≥10 built-ins post-S2, got {}",
        reg.len()
    );
}

#[test]
fn chain_runs_4_new_smts_in_order() {
    let reg = SmtRegistry::with_defaults();
    let mut cfg = BTreeMap::new();
    cfg.insert("transforms".into(), "rgx,ts,flat,rep".into());
    cfg.insert(
        "transforms.rgx.type".into(),
        "org.apache.kafka.connect.transforms.RegexRouter".into(),
    );
    cfg.insert("transforms.rgx.regex".into(), r"^.*$".into());
    cfg.insert("transforms.rgx.replacement".into(), "renamed".into());
    cfg.insert(
        "transforms.ts.type".into(),
        "org.apache.kafka.connect.transforms.TimestampRouter".into(),
    );
    cfg.insert(
        "transforms.ts.topic.format".into(),
        "${topic}.${timestamp}".into(),
    );
    cfg.insert("transforms.ts.timestamp.format".into(), "yyyy".into());
    cfg.insert(
        "transforms.flat.type".into(),
        "org.apache.kafka.connect.transforms.Flatten$Value".into(),
    );
    cfg.insert(
        "transforms.rep.type".into(),
        "org.apache.kafka.connect.transforms.ReplaceField$Value".into(),
    );
    cfg.insert("transforms.rep.renames".into(), "x:y".into());
    let chain: SmtChain = reg.build_chain(&cfg).unwrap();
    assert_eq!(chain.len(), 4);
    let names = chain.names();
    assert_eq!(names[0], "org.apache.kafka.connect.transforms.RegexRouter");
    assert!(names[1].starts_with("org.apache.kafka.connect.transforms.TimestampRouter"));
}

#[test]
fn chain_predicate_gated_drop_short_circuits_downstream() {
    // Predicate-gated transforms in upstream are a separate concept,
    // but our `Filter` SMT in the registry serves the same role:
    // place it first, drop everything, then verify downstream SMTs
    // do not observe the dropped record. (Smoke pattern from
    // `FilterChainTest` in upstream.)
    let mut chain = SmtChain::new();
    // Use a Filter that drops EVERY record.
    let mut filter_cfg = BTreeMap::new();
    filter_cfg.insert("type".into(), "exclude".into());
    let filter = cave_streams::connect_worker::smt::filter::Filter::from_config(&filter_cfg)
        .unwrap();
    chain.push(Arc::new(filter));
    // ReplaceField after — should never run.
    let mut rep_cfg = BTreeMap::new();
    rep_cfg.insert("renames".into(), "x:y".into());
    let rep = ReplaceField::from_config(&rep_cfg).unwrap();
    chain.push(Arc::new(rep));
    let r = RecordEnvelope::new("t", obj(&[("x", Value::Int(1))]));
    let out = chain.apply(r).unwrap();
    assert!(out.is_none());
}
