// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts nexus integration tests
//! Nexus module unit + integration tests.
//!
//! Covers the new code introduced by Faz 2: format adapter behaviour,
//! store CRUD with blob dedupe + cascading delete, cleanup criteria,
//! routing rule precedence, and a small set of HTTP integration tests
//! exercised through `tower::ServiceExt::oneshot` against the actual
//! axum router.

#![cfg(test)]

use super::cleanup;
use super::error::NexusError;
use super::format::{FormatAdapter, FormatRegistry, RawFormat};
use super::models::{
    Asset, BlobRef, CleanupCriteria, CleanupPolicy, Component, Format, Repository,
    RepositoryType, RoutingDecision, RoutingMode, RoutingRule, WritePolicy,
};
use super::routing;
use super::store::{sha256_hex, NexusStore};
use chrono::{Duration, Utc};
use uuid::Uuid;

// ── helpers ─────────────────────────────────────────────────────────────

fn mk_repo(name: &str, repo_type: RepositoryType, format: Format) -> Repository {
    let now = Utc::now();
    Repository {
        id: Uuid::new_v4(),
        name: name.into(),
        format,
        repo_type,
        online: true,
        cleanup_policies: vec![],
        created_at: now,
        updated_at: now,
    }
}

fn mk_hosted_raw(name: &str) -> Repository {
    mk_repo(
        name,
        RepositoryType::Hosted {
            write_policy: WritePolicy::Allow,
        },
        Format::Raw,
    )
}

fn mk_asset(repo: &Repository, path: &str, bytes: &[u8]) -> Asset {
    let now = Utc::now();
    Asset {
        id: Uuid::new_v4(),
        component_id: Uuid::new_v4(),
        repository_id: repo.id,
        repository_name: repo.name.clone(),
        path: path.into(),
        blob: BlobRef {
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
        },
        content_type: "application/octet-stream".into(),
        created_at: now,
        last_modified: now,
        last_downloaded: None,
        download_count: 0,
    }
}

// ── Format adapter tests ────────────────────────────────────────────────

#[test]
fn raw_parse_path_with_directory() {
    let coord = RawFormat.parse_path("dir/sub/file.txt").unwrap();
    assert_eq!(coord.group.as_deref(), Some("dir/sub"));
    assert_eq!(coord.name, "file.txt");
    assert_eq!(coord.version, None);
}

#[test]
fn raw_parse_path_without_directory() {
    let coord = RawFormat.parse_path("file.txt").unwrap();
    assert_eq!(coord.group, None);
    assert_eq!(coord.name, "file.txt");
}

#[test]
fn raw_parse_rejects_empty() {
    let err = RawFormat.parse_path("").unwrap_err();
    assert!(matches!(err, NexusError::InvalidPath(_)));
}

#[test]
fn raw_parse_rejects_trailing_slash() {
    let err = RawFormat.parse_path("dir/").unwrap_err();
    assert!(matches!(err, NexusError::InvalidPath(_)));
}

#[test]
fn raw_validate_rejects_path_traversal() {
    let err = RawFormat
        .validate_upload("dir/../etc/passwd", b"")
        .unwrap_err();
    assert!(matches!(err, NexusError::InvalidPath(_)));
}

#[test]
fn raw_validate_accepts_normal_path() {
    assert!(RawFormat.validate_upload("dir/file.txt", b"hello").is_ok());
}

#[test]
fn raw_content_type_sniffs_extensions() {
    assert_eq!(RawFormat.content_type("foo.json"), "application/json");
    assert_eq!(RawFormat.content_type("foo.tar.gz"), "application/gzip");
    assert_eq!(RawFormat.content_type("foo.unknown"), "application/octet-stream");
    assert_eq!(RawFormat.content_type("noext"), "application/octet-stream");
}

#[test]
fn format_registry_returns_raw_adapter() {
    let reg = FormatRegistry::with_defaults();
    let raw = reg.get(Format::Raw).unwrap();
    assert_eq!(raw.format(), Format::Raw);
}

#[test]
fn format_registry_reports_unimplemented_format() {
    let reg = FormatRegistry::with_defaults();
    let err = match reg.get(Format::Maven2) {
        Ok(_) => panic!("expected Maven2 to be unsupported in initial port"),
        Err(e) => e,
    };
    assert!(matches!(err, NexusError::FormatUnavailable(_)));
}

#[test]
fn format_parse_round_trips() {
    for raw_str in [
        "raw", "maven2", "npm", "docker", "pypi", "nuget", "helm", "apt", "yum",
    ] {
        let fmt = Format::parse(raw_str).expect("parses");
        assert_eq!(fmt.as_str(), raw_str);
    }
    assert!(Format::parse("unknown").is_none());
}

// ── Store CRUD + dedupe tests ───────────────────────────────────────────

#[test]
fn store_create_and_get_repository() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("releases");
    store.create_repository(repo.clone()).unwrap();
    let got = store.get_repository("releases").unwrap();
    assert_eq!(got.id, repo.id);
}

#[test]
fn store_create_repository_rejects_duplicate() {
    let store = NexusStore::new();
    store.create_repository(mk_hosted_raw("releases")).unwrap();
    let err = store
        .create_repository(mk_hosted_raw("releases"))
        .unwrap_err();
    assert!(matches!(err, NexusError::RepositoryAlreadyExists(_)));
}

#[test]
fn store_group_repo_requires_existing_members() {
    let store = NexusStore::new();
    let group = mk_repo(
        "all",
        RepositoryType::Group {
            member_names: vec!["does-not-exist".into()],
        },
        Format::Raw,
    );
    let err = store.create_repository(group).unwrap_err();
    assert!(matches!(err, NexusError::GroupMemberMissing(_)));
}

#[test]
fn store_blob_dedupe_across_assets() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();

    let bytes = b"shared payload".to_vec();
    let mut a1 = mk_asset(&repo, "a.txt", &bytes);
    let mut a2 = mk_asset(&repo, "b.txt", &bytes);
    // Force fresh component IDs
    a1.component_id = Uuid::new_v4();
    a2.component_id = Uuid::new_v4();
    store.put_asset(a1.clone(), bytes.clone()).unwrap();
    store.put_asset(a2.clone(), bytes.clone()).unwrap();
    assert_eq!(store.blob_count(), 1, "single dedup'd blob expected");
}

#[test]
fn store_blob_decref_drops_when_last_asset_deleted() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"x".to_vec();
    let asset = mk_asset(&repo, "x.txt", &bytes);
    store.put_asset(asset.clone(), bytes).unwrap();
    assert_eq!(store.blob_count(), 1);
    store.delete_asset(asset.id).unwrap();
    assert_eq!(store.blob_count(), 0, "blob garbage-collected");
}

#[test]
fn store_delete_repository_cascades_components_and_assets() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();

    let component = Component {
        id: Uuid::new_v4(),
        repository_id: repo.id,
        repository_name: repo.name.clone(),
        format: Format::Raw,
        group: None,
        name: "c1".into(),
        version: None,
        created_at: Utc::now(),
    };
    let component = store.create_component(component);

    let bytes = b"payload".to_vec();
    let asset = Asset {
        component_id: component.id,
        ..mk_asset(&repo, "c1.bin", &bytes)
    };
    store.put_asset(asset, bytes).unwrap();

    store.delete_repository("r").unwrap();
    assert!(store.list_components(Some("r")).is_empty());
    assert!(store.list_assets(Some("r")).is_empty());
    assert_eq!(store.blob_count(), 0);
}

#[test]
fn store_get_asset_by_path_indexes_uploads() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"hello".to_vec();
    let asset = mk_asset(&repo, "subdir/file.txt", &bytes);
    store.put_asset(asset.clone(), bytes).unwrap();
    let got = store.get_asset_by_path("r", "subdir/file.txt").unwrap();
    assert_eq!(got.id, asset.id);
}

#[test]
fn store_record_download_increments_counter() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"payload".to_vec();
    let asset = mk_asset(&repo, "f.bin", &bytes);
    store.put_asset(asset.clone(), bytes).unwrap();
    store.record_download(asset.id).unwrap();
    store.record_download(asset.id).unwrap();
    let after = store.get_asset(asset.id).unwrap();
    assert_eq!(after.download_count, 2);
    assert!(after.last_downloaded.is_some());
}

#[test]
fn store_put_asset_rejects_sha_mismatch() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"actual".to_vec();
    let mut asset = mk_asset(&repo, "f.bin", &bytes);
    asset.blob.sha256 = "00".repeat(32); // bogus sha
    let err = store.put_asset(asset, bytes).unwrap_err();
    assert!(matches!(err, NexusError::InvalidPath(_)));
}

// ── Cleanup tests ───────────────────────────────────────────────────────

#[test]
fn cleanup_no_criteria_matches_nothing() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"x".to_vec();
    store
        .put_asset(mk_asset(&repo, "a.txt", &bytes), bytes)
        .unwrap();
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "noop".into(),
        format: None,
        criteria: CleanupCriteria::default(),
        created_at: Utc::now(),
    };
    let ids = cleanup::evaluate(&store, &policy, "r").unwrap();
    assert!(ids.is_empty(), "no criteria → no matches (safety)");
}

#[test]
fn cleanup_older_than_matches_aged_assets() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"old".to_vec();
    let mut asset = mk_asset(&repo, "old.txt", &bytes);
    // Backdate to 60 days ago.
    asset.created_at = Utc::now() - Duration::days(60);
    asset.last_modified = asset.created_at;
    store.put_asset(asset.clone(), bytes).unwrap();

    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "age30".into(),
        format: None,
        criteria: CleanupCriteria {
            older_than_days: Some(30),
            ..Default::default()
        },
        created_at: Utc::now(),
    };
    let n = cleanup::apply(&store, &policy, "r").unwrap();
    assert_eq!(n, 1);
    assert!(store.list_assets(Some("r")).is_empty());
}

#[test]
fn cleanup_older_than_skips_fresh_assets() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"new".to_vec();
    store
        .put_asset(mk_asset(&repo, "new.txt", &bytes), bytes)
        .unwrap();
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "age30".into(),
        format: None,
        criteria: CleanupCriteria {
            older_than_days: Some(30),
            ..Default::default()
        },
        created_at: Utc::now(),
    };
    let n = cleanup::apply(&store, &policy, "r").unwrap();
    assert_eq!(n, 0);
}

#[test]
fn cleanup_regex_filters_paths() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    for name in ["snapshot/1.bin", "snapshot/2.bin", "release/1.bin"] {
        let bytes = name.as_bytes().to_vec();
        let mut asset = mk_asset(&repo, name, &bytes);
        asset.created_at = Utc::now() - Duration::days(60);
        store.put_asset(asset, bytes).unwrap();
    }
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "snap-purge".into(),
        format: None,
        criteria: CleanupCriteria {
            older_than_days: Some(30),
            regex: Some(r"^snapshot/".into()),
            ..Default::default()
        },
        created_at: Utc::now(),
    };
    let n = cleanup::apply(&store, &policy, "r").unwrap();
    assert_eq!(n, 2, "only snapshot/* deleted");
    let remaining = store.list_assets(Some("r"));
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].path, "release/1.bin");
}

#[test]
fn cleanup_format_scoped_skips_mismatched_repo() {
    let store = NexusStore::new();
    let repo = mk_hosted_raw("r");
    store.create_repository(repo.clone()).unwrap();
    let bytes = b"x".to_vec();
    let mut asset = mk_asset(&repo, "a.txt", &bytes);
    asset.created_at = Utc::now() - Duration::days(60);
    store.put_asset(asset, bytes).unwrap();
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "mvn-only".into(),
        format: Some(Format::Maven2),
        criteria: CleanupCriteria {
            older_than_days: Some(30),
            ..Default::default()
        },
        created_at: Utc::now(),
    };
    let n = cleanup::apply(&store, &policy, "r").unwrap();
    assert_eq!(n, 0);
}

#[test]
fn cleanup_invalid_regex_surfaces_error() {
    let store = NexusStore::new();
    let policy = CleanupPolicy {
        id: Uuid::new_v4(),
        name: "bad".into(),
        format: None,
        criteria: CleanupCriteria {
            regex: Some("[invalid".into()),
            older_than_days: Some(1),
            ..Default::default()
        },
        created_at: Utc::now(),
    };
    let err = cleanup::evaluate(&store, &policy, "any").unwrap_err();
    assert!(matches!(err, NexusError::InvalidRegex(_)));
}

// ── Routing tests ───────────────────────────────────────────────────────

fn rule(name: &str, mode: RoutingMode, matchers: &[&str]) -> RoutingRule {
    RoutingRule {
        id: Uuid::new_v4(),
        name: name.into(),
        mode,
        matchers: matchers.iter().map(|s| s.to_string()).collect(),
        created_at: Utc::now(),
    }
}

#[test]
fn routing_allow_passes_matching_path() {
    let r = rule("a", RoutingMode::Allow, &[r"^pkg/.*"]);
    assert_eq!(routing::evaluate(&r, "pkg/x").unwrap(), RoutingDecision::Allowed);
    assert_eq!(routing::evaluate(&r, "other/y").unwrap(), RoutingDecision::Blocked);
}

#[test]
fn routing_block_blocks_matching_path() {
    let r = rule("b", RoutingMode::Block, &[r"^secret/.*"]);
    assert_eq!(routing::evaluate(&r, "secret/x").unwrap(), RoutingDecision::Blocked);
    assert_eq!(routing::evaluate(&r, "public/y").unwrap(), RoutingDecision::Allowed);
}

#[test]
fn routing_block_takes_precedence_over_allow() {
    let rules = vec![
        rule("allow-pkg", RoutingMode::Allow, &[r"^pkg/.*"]),
        rule("block-secret", RoutingMode::Block, &[r"^pkg/secret/.*"]),
    ];
    assert_eq!(
        routing::evaluate_all(&rules, "pkg/secret/x").unwrap(),
        RoutingDecision::Blocked
    );
    assert_eq!(
        routing::evaluate_all(&rules, "pkg/normal").unwrap(),
        RoutingDecision::Allowed
    );
}

#[test]
fn routing_no_allow_rule_defaults_to_allow() {
    // With only block rules, anything not blocked is allowed.
    let rules = vec![rule("b", RoutingMode::Block, &[r"^secret/.*"])];
    assert_eq!(
        routing::evaluate_all(&rules, "anything").unwrap(),
        RoutingDecision::Allowed
    );
}

#[test]
fn routing_allow_rule_only_matches_listed_paths() {
    // With only allow rules, paths not matched are blocked.
    let rules = vec![rule("a", RoutingMode::Allow, &[r"^pkg/.*"])];
    assert_eq!(
        routing::evaluate_all(&rules, "other").unwrap(),
        RoutingDecision::Blocked
    );
}

#[test]
fn routing_invalid_regex_surfaces_error() {
    let r = rule("bad", RoutingMode::Allow, &["[invalid"]);
    let err = routing::evaluate(&r, "any").unwrap_err();
    assert!(matches!(err, NexusError::InvalidRegex(_)));
}

// ── HTTP integration tests (axum router via tower::ServiceExt) ─────────

mod http {
    use super::super::routes::{router, NexusState};
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        if bytes.is_empty() {
            return json!(null);
        }
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = router(NexusState::new());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/nexus/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["module"], "nexus");
        assert!(body["supported_formats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "raw"));
    }

    #[tokio::test]
    async fn create_get_delete_repository_round_trip() {
        let state = NexusState::new();
        let app = router(state);

        // Create
        let create_body = json!({
            "name": "raw-releases",
            "format": "raw",
            "type": "hosted",
            "write_policy": "allow",
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/repositories")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Get
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/nexus/v1/repositories/raw-releases")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["name"], "raw-releases");

        // Delete
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/nexus/v1/repositories/raw-releases")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn upload_then_download_raw_e2e() {
        let state = NexusState::new();
        let app = router(state);

        // Repo
        let create_body = json!({
            "name": "raw-public",
            "format": "raw",
            "type": "hosted",
            "write_policy": "allow",
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/repositories")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // PUT
        let payload = b"hello nexus";
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/nexus/repository/raw-public/dir/file.txt")
                    .body(Body::from(payload.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // GET
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/nexus/repository/raw-public/dir/file.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], payload);
    }

    #[tokio::test]
    async fn delete_raw_removes_asset() {
        let state = NexusState::new();
        let app = router(state);

        // Setup repo + upload
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/repositories")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "r",
                            "format": "raw",
                            "type": "hosted",
                            "write_policy": "allow",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/nexus/repository/r/x.txt")
                    .body(Body::from("hi".as_bytes().to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Delete
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/nexus/repository/r/x.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // GET → 404
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/nexus/repository/r/x.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn upload_to_proxy_repo_is_unprocessable() {
        let state = NexusState::new();
        let app = router(state);

        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/repositories")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "proxy",
                            "format": "raw",
                            "type": "proxy",
                            "remote_url": "https://example.com",
                            "cache_ttl_minutes": 60,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/nexus/repository/proxy/x.txt")
                    .body(Body::from("bytes".as_bytes().to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_and_apply_cleanup_policy() {
        let state = NexusState::new();
        let app = router(state);

        // repo + 1 (synthetic ageing happens at the store layer, not via HTTP;
        // this test exercises HTTP shape only — empty result expected).
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/repositories")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "r",
                            "format": "raw",
                            "type": "hosted",
                            "write_policy": "allow",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/cleanup-policies")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "purge-snap",
                            "criteria": {
                                "older_than_days": 30,
                                "regex": "^snap/",
                            },
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/cleanup-policies/purge-snap/apply?repository=r&dry_run=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["repository"], "r");
        assert_eq!(body["would_delete"], 0);
    }

    #[tokio::test]
    async fn routing_rule_test_endpoint_returns_decision() {
        let state = NexusState::new();
        let app = router(state);

        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/routing-rules")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "block-secret",
                            "mode": "block",
                            "matchers": ["^secret/.*"],
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/routing-rules/block-secret/test?path=secret/leak")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["decision"], "blocked");

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/nexus/v1/routing-rules/block-secret/test?path=public/ok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_json(resp).await;
        assert_eq!(body["decision"], "allowed");
    }
}
