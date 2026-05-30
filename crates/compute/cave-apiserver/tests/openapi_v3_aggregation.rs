// SPDX-License-Identifier: AGPL-3.0-only
//
// Parity tests for OpenAPI v3 lazy aggregation + ETag caching.
//
// Upstream: kubernetes/kubernetes (Apache-2.0)
//   staging/src/k8s.io/kube-openapi/pkg/handler3 — OpenAPIService
//   pkg/endpoints/openapi
//
// Upstream's OpenAPIService aggregates every GroupVersion spec into a single
// served document, caches the marshaled bytes together with an ETag, and only
// recomputes when the underlying spec changes. Conditional requests whose
// If-None-Match matches the current ETag get a 304 (NotModified). These tests
// pin that behaviour: union merge of paths/components, a content hash that
// moves only on real change, a rebuild counter that stays flat across no-op
// calls, and 304 semantics.

use cave_apiserver::openapi_v3::{
    OpenApiV3Cache, OpenApiV3Document, OpenApiV3Served, OpenApiV3Service,
};
use serde_json::json;
use std::collections::BTreeMap;

fn doc_with(path: &str, schema: &str) -> OpenApiV3Document {
    let mut paths = BTreeMap::new();
    paths.insert(path.to_string(), json!({"get": {"operationId": path}}));
    let mut components = BTreeMap::new();
    components.insert(schema.to_string(), json!({"type": "object"}));
    OpenApiV3Document {
        openapi: "3.0.0".to_string(),
        info: BTreeMap::new(),
        paths,
        components,
    }
}

#[test]
fn aggregate_merges_paths_and_components() {
    let mut svc = OpenApiV3Service::new();
    svc.add_group_version("apps/v1", doc_with("/apis/apps/v1/deployments", "Deployment"));
    svc.add_group_version("batch/v1", doc_with("/apis/batch/v1/jobs", "Job"));

    let agg = svc.aggregate();
    assert_eq!(agg.openapi, "3.0.0");
    assert!(agg.paths.contains_key("/apis/apps/v1/deployments"));
    assert!(agg.paths.contains_key("/apis/batch/v1/jobs"));
    assert!(agg.components.contains_key("Deployment"));
    assert!(agg.components.contains_key("Job"));
    assert_eq!(agg.paths.len(), 2);
    assert_eq!(agg.components.len(), 2);
}

#[test]
fn aggregate_hash_moves_only_on_real_change() {
    let mut svc = OpenApiV3Service::new();
    svc.add_group_version("apps/v1", doc_with("/a", "A"));
    let h1 = svc.aggregate_hash();

    // Re-reading without mutation yields the same hash.
    assert_eq!(h1, svc.aggregate_hash());

    // Adding a GroupVersion changes the aggregate hash.
    svc.add_group_version("batch/v1", doc_with("/b", "B"));
    let h2 = svc.aggregate_hash();
    assert_ne!(h1, h2);
}

#[test]
fn cache_rebuilds_only_when_spec_changes() {
    let mut svc = OpenApiV3Service::new();
    svc.add_group_version("apps/v1", doc_with("/a", "A"));

    let mut cache = OpenApiV3Cache::new();
    cache.ensure(&svc);
    assert_eq!(cache.rebuilds(), 1, "first ensure builds once");

    // No change -> no rebuild.
    cache.ensure(&svc);
    cache.ensure(&svc);
    assert_eq!(cache.rebuilds(), 1, "no-op ensures must not rebuild");

    // Spec change -> exactly one more rebuild.
    svc.add_group_version("batch/v1", doc_with("/b", "B"));
    cache.ensure(&svc);
    assert_eq!(cache.rebuilds(), 2);
    cache.ensure(&svc);
    assert_eq!(cache.rebuilds(), 2);

    assert!(!cache.bytes().is_empty());
    assert!(cache.etag().contains(&svc.aggregate_hash()));
}

#[test]
fn serve_returns_not_modified_on_matching_etag() {
    let mut svc = OpenApiV3Service::new();
    svc.add_group_version("apps/v1", doc_with("/a", "A"));

    let mut cache = OpenApiV3Cache::new();
    let body = cache.serve(&svc, None);
    let etag = match body {
        OpenApiV3Served::Body { etag, ref bytes } => {
            assert!(!bytes.is_empty());
            etag
        }
        OpenApiV3Served::NotModified { .. } => panic!("first fetch must return a body"),
    };

    // Conditional request with the current ETag -> 304.
    match cache.serve(&svc, Some(&etag)) {
        OpenApiV3Served::NotModified { etag: e } => assert_eq!(e, etag),
        OpenApiV3Served::Body { .. } => panic!("matching ETag must yield NotModified"),
    }
}

#[test]
fn serve_returns_body_on_stale_etag() {
    let mut svc = OpenApiV3Service::new();
    svc.add_group_version("apps/v1", doc_with("/a", "A"));

    let mut cache = OpenApiV3Cache::new();
    match cache.serve(&svc, Some("\"stale-deadbeef\"")) {
        OpenApiV3Served::Body { bytes, .. } => assert!(!bytes.is_empty()),
        OpenApiV3Served::NotModified { .. } => panic!("stale ETag must yield a fresh body"),
    }
}
