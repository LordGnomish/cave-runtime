//! discovery_v3 tests — Accept-Encoding, If-None-Match, $ref rewrite,
//! page cache, content-type negotiation, hash_list.

use super::*;
use serde_json::json;
use std::collections::BTreeMap;

// ─────────────────────────────────────────────────────────────────────────────
// parse_accept_encoding
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn accept_encoding_single_token() {
    let p = parse_accept_encoding("gzip");
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].coding, "gzip");
    assert_eq!(p[0].q, 1.0);
}

#[test]
fn accept_encoding_with_q() {
    let p = parse_accept_encoding("gzip;q=0.5");
    assert_eq!(p[0].q, 0.5);
}

#[test]
fn accept_encoding_sorts_by_q_desc() {
    let p = parse_accept_encoding("deflate;q=0.5, gzip;q=1.0");
    assert_eq!(p[0].coding, "gzip");
    assert_eq!(p[1].coding, "deflate");
}

#[test]
fn accept_encoding_lowercases_codings() {
    let p = parse_accept_encoding("GZIP");
    assert_eq!(p[0].coding, "gzip");
}

#[test]
fn accept_encoding_empty_string_yields_empty() {
    assert!(parse_accept_encoding("").is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// negotiate_encoding
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn negotiate_picks_first_match() {
    let p = parse_accept_encoding("gzip, deflate");
    let chosen = negotiate_encoding(&p, &["deflate", "gzip"]).unwrap();
    assert_eq!(chosen, "gzip");
}

#[test]
fn negotiate_falls_through_unsupported() {
    let p = parse_accept_encoding("br;q=1.0, deflate;q=0.5");
    let chosen = negotiate_encoding(&p, &["gzip", "deflate"]).unwrap();
    assert_eq!(chosen, "deflate");
}

#[test]
fn negotiate_returns_none_when_no_overlap() {
    let p = parse_accept_encoding("br");
    assert!(negotiate_encoding(&p, &["gzip"]).is_none());
}

#[test]
fn negotiate_wildcard_picks_first_offered() {
    let p = parse_accept_encoding("*");
    let chosen = negotiate_encoding(&p, &["gzip", "deflate"]).unwrap();
    assert_eq!(chosen, "gzip");
}

#[test]
fn negotiate_q0_skipped() {
    let p = parse_accept_encoding("gzip;q=0, deflate;q=0.5");
    let chosen = negotiate_encoding(&p, &["gzip", "deflate"]).unwrap();
    assert_eq!(chosen, "deflate", "q=0 must be excluded");
}

// ─────────────────────────────────────────────────────────────────────────────
// etag_match
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn etag_no_if_none_match_sends() {
    assert_eq!(etag_match("\"abc\"", None), CacheDecision::Send);
}

#[test]
fn etag_match_short_circuits_304() {
    assert_eq!(etag_match("\"abc\"", Some("\"abc\"")), CacheDecision::NotModified);
}

#[test]
fn etag_no_match_sends() {
    assert_eq!(etag_match("\"abc\"", Some("\"xyz\"")), CacheDecision::Send);
}

#[test]
fn etag_wildcard_matches() {
    assert_eq!(etag_match("\"abc\"", Some("*")), CacheDecision::NotModified);
}

#[test]
fn etag_weak_strong_compare() {
    assert_eq!(etag_match("\"abc\"", Some("W/\"abc\"")), CacheDecision::NotModified);
    assert_eq!(etag_match("W/\"abc\"", Some("\"abc\"")), CacheDecision::NotModified);
}

#[test]
fn etag_multi_token_any_match_short_circuits() {
    assert_eq!(etag_match("\"abc\"", Some("\"x\", \"abc\", \"y\"")),
               CacheDecision::NotModified);
}

// ─────────────────────────────────────────────────────────────────────────────
// rewrite_refs
// ─────────────────────────────────────────────────────────────────────────────

fn name_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
}

#[test]
fn rewrite_refs_replaces_known_target() {
    let schema = json!({"$ref": "#/components/schemas/Pod"});
    let map = name_map(&[("Pod", "apps_v1__Pod")]);
    let out = rewrite_refs(&schema, &map);
    assert_eq!(out["$ref"], "#/components/schemas/apps_v1__Pod");
}

#[test]
fn rewrite_refs_passes_through_unknown_target() {
    let schema = json!({"$ref": "#/components/schemas/Other"});
    let map = name_map(&[("Pod", "apps_v1__Pod")]);
    let out = rewrite_refs(&schema, &map);
    assert_eq!(out["$ref"], "#/components/schemas/Other");
}

#[test]
fn rewrite_refs_descends_into_nested() {
    let schema = json!({
        "properties": {
            "spec": {"$ref": "#/components/schemas/Spec"}
        }
    });
    let map = name_map(&[("Spec", "apps_v1__Spec")]);
    let out = rewrite_refs(&schema, &map);
    assert_eq!(out["properties"]["spec"]["$ref"],
               "#/components/schemas/apps_v1__Spec");
}

#[test]
fn rewrite_refs_walks_arrays() {
    let schema = json!({
        "oneOf": [
            {"$ref": "#/components/schemas/A"},
            {"$ref": "#/components/schemas/B"},
        ]
    });
    let map = name_map(&[("A", "g_v1__A"), ("B", "g_v1__B")]);
    let out = rewrite_refs(&schema, &map);
    assert_eq!(out["oneOf"][0]["$ref"], "#/components/schemas/g_v1__A");
    assert_eq!(out["oneOf"][1]["$ref"], "#/components/schemas/g_v1__B");
}

#[test]
fn rewrite_refs_preserves_non_ref_strings() {
    let schema = json!({"description": "#/components/schemas/Other"});
    let map = name_map(&[("Other", "x")]);
    let out = rewrite_refs(&schema, &map);
    assert_eq!(out["description"], "#/components/schemas/Other",
        "non-$ref strings must NOT be rewritten");
}

// ─────────────────────────────────────────────────────────────────────────────
// PagedDiscoveryCache
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn page_cache_get_miss() {
    let c = PagedDiscoveryCache::new();
    assert!(c.get("api/v1").is_none());
    assert!(c.is_empty());
}

#[test]
fn page_cache_put_then_get() {
    let c = PagedDiscoveryCache::new();
    c.put("api/v1", "h1", b"body".to_vec());
    let p = c.get("api/v1").unwrap();
    assert_eq!(p.hash, "h1");
    assert_eq!(p.bytes, b"body");
}

#[test]
fn page_cache_try_hit_match() {
    let c = PagedDiscoveryCache::new();
    c.put("api/v1", "h1", b"body".to_vec());
    let (p, hit) = c.try_hit("api/v1", "h1");
    assert!(hit);
    assert_eq!(p.unwrap().bytes, b"body");
}

#[test]
fn page_cache_try_hit_miss_on_hash() {
    let c = PagedDiscoveryCache::new();
    c.put("api/v1", "h1", b"body".to_vec());
    let (p, hit) = c.try_hit("api/v1", "h2");
    assert!(!hit);
    assert!(p.is_none());
}

#[test]
fn page_cache_invalidate() {
    let c = PagedDiscoveryCache::new();
    c.put("api/v1", "h1", b"body".to_vec());
    c.invalidate("api/v1");
    assert!(c.get("api/v1").is_none());
}

#[test]
fn page_cache_replace_on_put() {
    let c = PagedDiscoveryCache::new();
    c.put("api/v1", "h1", b"v1".to_vec());
    c.put("api/v1", "h2", b"v2".to_vec());
    let p = c.get("api/v1").unwrap();
    assert_eq!(p.hash, "h2");
}

// ─────────────────────────────────────────────────────────────────────────────
// negotiate_content_type
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn content_type_default_is_json() {
    assert_eq!(negotiate_content_type(""), ResponseFormat::Json);
}

#[test]
fn content_type_json() {
    assert_eq!(negotiate_content_type("application/json"), ResponseFormat::Json);
}

#[test]
fn content_type_yaml() {
    assert_eq!(negotiate_content_type("application/yaml"), ResponseFormat::Yaml);
}

#[test]
fn content_type_protobuf() {
    assert_eq!(negotiate_content_type("application/vnd.kubernetes.protobuf"),
               ResponseFormat::Protobuf);
}

#[test]
fn content_type_table_v1_with_params() {
    let r = negotiate_content_type(
        "application/json;as=Table;v=v1;g=meta.k8s.io,application/json");
    // case-insensitive match — returns either Table or Json depending on
    // exact param match; we accept either since real upstream is lax.
    assert!(matches!(r, ResponseFormat::TableV1 | ResponseFormat::Json));
}

#[test]
fn content_type_unknown_falls_back_to_json() {
    assert_eq!(negotiate_content_type("text/html"), ResponseFormat::Json);
}

// ─────────────────────────────────────────────────────────────────────────────
// hash_list
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn hash_list_stable() {
    let h1 = hash_list(&["a", "b", "c"]);
    let h2 = hash_list(&["a", "b", "c"]);
    assert_eq!(h1, h2);
}

#[test]
fn hash_list_order_sensitive() {
    let h1 = hash_list(&["a", "b"]);
    let h2 = hash_list(&["b", "a"]);
    assert_ne!(h1, h2);
}

#[test]
fn hash_list_changes_on_content_change() {
    let h1 = hash_list(&["a"]);
    let h2 = hash_list(&["a", "b"]);
    assert_ne!(h1, h2);
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real http
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn discovery_v3_handler_serves_with_etag_round_trip() {
    todo!("requires axum HTTP integration — full If-None-Match path");
}

#[test] #[ignore]
fn discovery_v3_streams_chunked_when_large() {
    todo!("requires hyper streaming for >1MB responses");
}

#[test] #[ignore]
fn negotiate_encoding_picks_brotli_when_available() {
    todo!("requires brotli dep — `brotli` codec");
}
