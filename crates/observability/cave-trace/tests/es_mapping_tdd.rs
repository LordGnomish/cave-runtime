// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD parity port — Jaeger Elasticsearch index-mapping generator.
//!
//! Upstream: jaegertracing/jaeger v1.52.0
//!   plugin/storage/es/mappings/mapping.go  (MappingBuilder)
//!   cmd/esmapping-generator/app/renderer/render.go
//!   plugin/storage/es/mappings/jaeger-span-{7,8}.json (+ service / dependencies)
//!
//! The esmapping-generator renders the Elasticsearch index templates Jaeger
//! installs for its span / service / dependencies indices. The template
//! parameters (`.Shards`, `.Replicas`, `.IndexPrefix`, `.UseILM`,
//! `.ILMPolicyName`, priority) are substituted into a version-specific
//! skeleton: ES ≥ 8 emits a *composable* index template (`priority` +
//! `template:{settings,mappings}`), ES 7 emits the *legacy* `_template`
//! shape (top-level `settings`/`mappings`/`aliases` + `order`).
//!
//! This is the pure render algorithm; the live ES bulk client + the
//! PUT `_index_template` HTTP call stay scope_cut (operational-storage-backends).
//!
//! RED commit: references `cave_trace::storage_es::MappingBuilder`, not yet
//! defined → crate fails to compile.

use cave_trace::storage_es::MappingBuilder;

fn settings(v: &serde_json::Value, es_version: u32) -> serde_json::Value {
    if es_version >= 8 {
        v["template"]["settings"].clone()
    } else {
        v["settings"].clone()
    }
}

fn mappings(v: &serde_json::Value, es_version: u32) -> serde_json::Value {
    if es_version >= 8 {
        v["template"]["mappings"].clone()
    } else {
        v["mappings"].clone()
    }
}

// ── 1. Shards / replicas are substituted into settings ───────────────────────

#[test]
fn shards_and_replicas_substituted() {
    let mb = MappingBuilder {
        shards: 5,
        replicas: 2,
        es_version: 8,
        ..MappingBuilder::default()
    };
    let v = mb.span_mapping();
    let s = settings(&v, 8);
    assert_eq!(s["number_of_shards"], 5);
    assert_eq!(s["number_of_replicas"], 2);
    // it must be valid JSON (round-trips through a string)
    let text = serde_json::to_string(&v).unwrap();
    let _back: serde_json::Value = serde_json::from_str(&text).unwrap();
}

// ── 2. index_patterns reflect the configured prefix ──────────────────────────

#[test]
fn index_patterns_honor_prefix() {
    let mb = MappingBuilder {
        index_prefix: Some("prod".into()),
        es_version: 8,
        ..MappingBuilder::default()
    };
    let v = mb.span_mapping();
    let patterns = v["index_patterns"].as_array().expect("index_patterns array");
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0], "prod-jaeger-span-*");

    // no prefix → bare base
    let mb2 = MappingBuilder {
        es_version: 8,
        ..MappingBuilder::default()
    };
    assert_eq!(mb2.span_mapping()["index_patterns"][0], "jaeger-span-*");
}

// ── 3. ILM block present iff UseILM ──────────────────────────────────────────

#[test]
fn ilm_settings_gated_on_use_ilm() {
    let mb = MappingBuilder {
        use_ilm: true,
        ilm_policy_name: "jaeger-ilm".into(),
        index_prefix: Some("prod".into()),
        es_version: 8,
        ..MappingBuilder::default()
    };
    let s = settings(&mb.span_mapping(), 8);
    assert_eq!(s["index.lifecycle.name"], "jaeger-ilm");
    assert_eq!(s["index.lifecycle.rollover_alias"], "prod-jaeger-span-write");

    let mb_off = MappingBuilder {
        use_ilm: false,
        es_version: 8,
        ..MappingBuilder::default()
    };
    let s_off = settings(&mb_off.span_mapping(), 8);
    assert!(s_off.get("index.lifecycle.name").is_none());
    assert!(s_off.get("index.lifecycle.rollover_alias").is_none());
}

// ── 4. ES8 emits composable template (priority + template wrapper) ───────────

#[test]
fn es8_emits_composable_template() {
    let mb = MappingBuilder {
        es_version: 8,
        priority_span_template: 503,
        ..MappingBuilder::default()
    };
    let v = mb.span_mapping();
    assert_eq!(v["priority"], 503);
    assert!(v["template"].is_object(), "ES8 wraps settings+mappings in `template`");
    assert!(v.get("order").is_none(), "ES8 must not use the legacy `order`");
}

// ── 5. ES7 emits legacy template (top-level settings/mappings + order) ───────

#[test]
fn es7_emits_legacy_template() {
    let mb = MappingBuilder {
        es_version: 7,
        priority_span_template: 503,
        ..MappingBuilder::default()
    };
    let v = mb.span_mapping();
    assert!(v.get("template").is_none(), "ES7 has no `template` wrapper");
    assert!(v["settings"].is_object());
    assert!(v["mappings"].is_object());
    assert_eq!(v["order"], 503, "legacy template carries `order`");
}

// ── 6. Span document mapping carries the canonical jaeger-span properties ────

#[test]
fn span_mapping_has_core_properties() {
    let mb = MappingBuilder {
        es_version: 8,
        ..MappingBuilder::default()
    };
    let m = mappings(&mb.span_mapping(), 8);
    let props = &m["properties"];
    assert_eq!(props["traceID"]["type"], "keyword");
    assert_eq!(props["spanID"]["type"], "keyword");
    // startTimeMillis is the routing date field
    assert_eq!(props["startTimeMillis"]["type"], "date");
    assert_eq!(props["startTimeMillis"]["format"], "epoch_millis");
    // duration is a long
    assert_eq!(props["duration"]["type"], "long");
    // tags are nested key/value objects
    assert_eq!(props["tags"]["type"], "nested");
    assert_eq!(props["tags"]["properties"]["key"]["type"], "keyword");
    // process is an object with a serviceName keyword
    assert_eq!(props["process"]["properties"]["serviceName"]["type"], "keyword");
    // date_detection disabled (jaeger pins explicit types)
    assert_eq!(m["date_detection"], false);
}

// ── 7. Service + dependencies templates use their own index patterns ─────────

#[test]
fn service_and_dependencies_have_distinct_patterns() {
    let mb = MappingBuilder {
        index_prefix: Some("prod".into()),
        es_version: 8,
        priority_service_template: 510,
        priority_dependencies_template: 520,
        ..MappingBuilder::default()
    };
    let svc = mb.service_mapping();
    assert_eq!(svc["index_patterns"][0], "prod-jaeger-service-*");
    assert_eq!(svc["priority"], 510);
    let svc_props = &mappings(&svc, 8)["properties"];
    assert_eq!(svc_props["serviceName"]["type"], "keyword");
    assert_eq!(svc_props["operationName"]["type"], "keyword");

    let dep = mb.dependencies_mapping();
    assert_eq!(dep["index_patterns"][0], "prod-jaeger-dependencies-*");
    assert_eq!(dep["priority"], 520);
    let dep_props = &mappings(&dep, 8)["properties"];
    assert_eq!(dep_props["timestamp"]["type"], "date");
    assert_eq!(dep_props["dependencies"]["type"], "nested");
}

// ── 8. Mapping limits + cache settings are emitted ───────────────────────────

#[test]
fn settings_carry_mapping_limits() {
    let mb = MappingBuilder {
        es_version: 8,
        ..MappingBuilder::default()
    };
    let s = settings(&mb.span_mapping(), 8);
    assert_eq!(s["index.mapping.nested_fields.limit"], 50);
    assert_eq!(s["index.requests.cache.enable"], true);
}
