// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! discovery v2 + OpenAPI v3 transport tests.

use super::*;
use crate::discovery::{APIResource, APIResourceList};

fn rl(group_version: &str, kinds: &[&str]) -> APIResourceList {
    APIResourceList {
        group_version: group_version.into(),
        resources: kinds
            .iter()
            .map(|k| APIResource {
                name: k.to_lowercase(),
                kind: (*k).into(),
                namespaced: true,
                verbs: vec!["get".into(), "list".into()],
                short_names: vec![],
                categories: vec![],
            })
            .collect(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ETag — `aggregated/handler_test.go::TestETag`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn etag_is_stable_for_same_input() {
    assert_eq!(etag_for_bytes(b"hello"), etag_for_bytes(b"hello"));
}

#[test]
fn etag_differs_for_different_input() {
    assert_ne!(etag_for_bytes(b"hello"), etag_for_bytes(b"world"));
}

#[test]
fn etag_is_quoted_per_rfc7232() {
    let e = etag_for_bytes(b"x");
    assert!(
        e.starts_with('"') && e.ends_with('"'),
        "ETag must be a quoted-string per RFC 7232 §2.3"
    );
}

#[test]
fn etag_for_json_is_deterministic() {
    let v = serde_json::json!({"a":1,"b":2});
    let a = etag_for_json(&v).unwrap();
    let b = etag_for_json(&v).unwrap();
    assert_eq!(a, b);
}

#[test]
fn etag_differs_for_tenant_scoped_payload() {
    // tenant_id invariant: identical resource shape from two tenants must
    // not collide on ETag, because the doc embeds the tenant in the
    // top-level metadata.
    let acme = serde_json::json!({"tenant":"acme","items":[{"name":"pods"}]});
    let globex = serde_json::json!({"tenant":"globex","items":[{"name":"pods"}]});
    assert_ne!(
        etag_for_json(&acme).unwrap(),
        etag_for_json(&globex).unwrap()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// gzip envelope shape — we don't ship deflate, but the envelope must look
// like a gzip stream so an HTTP layer that just inspects magic bytes will
// accept it.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn gzip_envelope_starts_with_magic_bytes() {
    let env = gzip_envelope(b"hello");
    assert!(is_gzip_envelope(&env));
}

#[test]
fn gzip_envelope_round_trip() {
    let env = gzip_envelope(b"hello-world");
    let payload = unwrap_gzip_envelope(&env).unwrap();
    assert_eq!(payload, b"hello-world");
}

#[test]
fn gzip_envelope_rejects_non_gzip_magic() {
    let bytes = b"\x1f\x8a\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
    assert!(!is_gzip_envelope(bytes));
}

#[test]
fn gzip_envelope_too_short_is_not_gzip() {
    assert!(!is_gzip_envelope(b"short"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Pagination — `handler_test.go::TestPagedResponse`
// ─────────────────────────────────────────────────────────────────────────────

fn make_groups(n: usize) -> Vec<APIGroupDiscovery> {
    (0..n)
        .map(|i| {
            let group_name = format!("g{i}");
            let list = rl(&format!("{group_name}/v1"), &["A"]);
            APIGroupDiscovery {
                name: group_name,
                versions: vec![from_resource_list("v1", &list)],
            }
        })
        .collect()
}

#[test]
fn page_groups_returns_first_page_when_under_limit() {
    let g = make_groups(3);
    let req = PageRequest {
        limit: 10,
        continue_token: None,
    };
    let p = page_groups(&g, &req);
    assert_eq!(p.doc.items.len(), 3);
    assert!(p.doc.continue_token.is_empty());
}

#[test]
fn page_groups_returns_partial_page_with_token() {
    let g = make_groups(10);
    let req = PageRequest {
        limit: 3,
        continue_token: None,
    };
    let p = page_groups(&g, &req);
    assert_eq!(p.doc.items.len(), 3);
    assert!(!p.doc.continue_token.is_empty());
}

#[test]
fn page_groups_consume_token_for_next_page() {
    let g = make_groups(10);
    let p1 = page_groups(
        &g,
        &PageRequest {
            limit: 3,
            continue_token: None,
        },
    );
    let p2 = page_groups(
        &g,
        &PageRequest {
            limit: 3,
            continue_token: Some(p1.doc.continue_token),
        },
    );
    assert_eq!(p2.doc.items.len(), 3);
    assert_eq!(p2.doc.items[0].name, "g3");
}

#[test]
fn page_groups_final_page_clears_token() {
    let g = make_groups(5);
    let p1 = page_groups(
        &g,
        &PageRequest {
            limit: 3,
            continue_token: None,
        },
    );
    let p2 = page_groups(
        &g,
        &PageRequest {
            limit: 3,
            continue_token: Some(p1.doc.continue_token),
        },
    );
    assert!(
        p2.doc.continue_token.is_empty(),
        "last page must NOT carry a continuation token"
    );
}

#[test]
fn page_groups_empty_input() {
    let g = vec![];
    let p = page_groups(
        &g,
        &PageRequest {
            limit: 5,
            continue_token: None,
        },
    );
    assert!(p.doc.items.is_empty());
    assert!(p.doc.continue_token.is_empty());
}

#[test]
fn page_groups_overflow_token_yields_empty_page() {
    let g = make_groups(2);
    let p = page_groups(
        &g,
        &PageRequest {
            limit: 5,
            continue_token: Some(encode_continue_pub(99)),
        },
    );
    assert!(p.doc.items.is_empty());
    assert!(p.doc.continue_token.is_empty());
}

fn encode_continue_pub(idx: usize) -> String {
    // re-implement via base64_encode-equivalent through a roundtrip helper
    // we don't expose super::encode_continue, so use the public path:
    // crank through page_groups to derive the token format.
    let g = make_groups(idx + 1);
    let req = PageRequest {
        limit: idx,
        continue_token: None,
    };
    let p = page_groups(&g, &req);
    p.doc.continue_token
}

// ─────────────────────────────────────────────────────────────────────────────
// AggregatedDiscoveryV2 round-trip
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn aggregated_v2_roundtrip() {
    let doc = AggregatedDiscoveryV2 {
        api_version: "apidiscovery.k8s.io/v2".into(),
        kind: "APIGroupDiscoveryList".into(),
        items: make_groups(2),
        continue_token: "".into(),
    };
    let s = serde_json::to_string(&doc).unwrap();
    let d2: AggregatedDiscoveryV2 = serde_json::from_str(&s).unwrap();
    assert_eq!(d2.items.len(), 2);
}

#[test]
fn from_resource_list_preserves_resources() {
    let rl = rl("apps/v1", &["Deployment", "StatefulSet"]);
    let v = from_resource_list("v1", &rl);
    assert_eq!(v.resources.len(), 2);
    assert_eq!(v.resources[0].kind, "Deployment");
}

#[test]
fn group_from_versions_constructs_group() {
    let rl = rl("apps/v1", &["Deployment"]);
    let v = from_resource_list("v1", &rl);
    let g = group_from_versions("apps", vec![v]);
    assert_eq!(g.name, "apps");
    assert_eq!(g.versions.len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAPI v3 index — `handler3/handler_test.go::TestOpenAPIV3Index`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn build_index_collects_paths_with_hash() {
    let mut specs = BTreeMap::new();
    specs.insert("apis/apps/v1".to_string(), b"{\"x\":1}".to_vec());
    specs.insert("api/v1".to_string(), b"{\"y\":2}".to_vec());
    let idx = build_index(&specs);
    assert_eq!(idx.paths.len(), 2);
    let entry = &idx.paths["api/v1"];
    assert!(
        entry
            .server_relative_url
            .starts_with("/openapi/v3/api/v1?hash="),
        "entry must reference its content-hashed URL"
    );
}

#[test]
fn build_index_hash_is_stable() {
    let mut specs = BTreeMap::new();
    specs.insert("api/v1".to_string(), b"{\"y\":2}".to_vec());
    let a = build_index(&specs);
    let b = build_index(&specs);
    assert_eq!(
        a.paths["api/v1"].server_relative_url,
        b.paths["api/v1"].server_relative_url
    );
}

#[test]
fn build_index_hash_changes_on_content_change() {
    let mut specs1 = BTreeMap::new();
    specs1.insert("api/v1".to_string(), b"{\"y\":2}".to_vec());
    let mut specs2 = BTreeMap::new();
    specs2.insert("api/v1".to_string(), b"{\"y\":3}".to_vec());
    assert_ne!(
        build_index(&specs1).paths["api/v1"].server_relative_url,
        build_index(&specs2).paths["api/v1"].server_relative_url
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real flate2 + sha2 deps.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[cfg(feature = "live-integration")]
fn gzip_real_deflate_round_trip() {
    // pending: requires `flate2` crate — full RFC 1951 deflate round-trip
}

#[test]
#[cfg(feature = "live-integration")]
fn etag_uses_sha256_when_dep_landed() {
    // pending: requires `sha2` crate — etag should be sha256-hex per upstream
}

#[test]
#[cfg(feature = "live-integration")]
fn aggregated_discovery_serves_via_http_handler() {
    // pending: requires axum wiring + Accept-Encoding negotiation
}

#[test]
#[cfg(feature = "live-integration")]
fn protobuf_response_for_application_vnd_kubernetes_protobuf() {
    // pending: requires Kubernetes protobuf wire format — `runtime.protobuf`
}
