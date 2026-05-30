// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for the `cave-registry` OCI registry data-plane.
//!
//! These port behaviors from upstream **distribution/distribution v3.1.1**
//! (the OCI Registry v2 / Distribution Spec 1.1 reference implementation):
//! blob ref-count deletion, manifest delete-by-tag with empty-repo pruning,
//! OCI 1.1 referrers with `artifactType` filtering, cross-repo blob mount
//! guarding, upload-session lifecycle (offset/cancel/retry-on-mismatch),
//! manifest fetch by digest vs. tag, sorted catalog enumeration, and garbage
//! collection that retains manifest config + layer blobs.
//!
//! `cave-registry` is a thin alias (`pub use cave_artifacts::harbor::*`); the
//! real implementation lives in `cave_artifacts::harbor::storage`, which this
//! file exercises directly (cave-artifacts is a dependency of cave-registry).

use cave_artifacts::harbor::storage::{compute_digest, RegistryStorage};

// distribution/distribution storage/blob_test.go `TestBlobMount` / linkedblobstore:
// a blob is freed only once the last repository reference is dropped.
#[tokio::test]
async fn delete_blob_frees_only_after_last_repo_ref() {
    let s = RegistryStorage::default();
    let data: &[u8] = b"shared-layer-bytes";
    let digest = compute_digest(data);

    // Same content addressed into two repositories => one blob, two refs.
    s.store_blob(digest.clone(), data.into(), "team/a").await;
    s.store_blob(digest.clone(), data.into(), "team/b").await;
    assert!(s.has_blob(&digest).await);

    // Dropping the first ref returns true but the blob survives (ref set non-empty).
    assert!(s.delete_blob(&digest, "team/a").await);
    assert!(
        s.has_blob(&digest).await,
        "blob must remain while team/b still references it"
    );

    // Dropping the last ref removes the blob from content store.
    assert!(s.delete_blob(&digest, "team/b").await);
    assert!(
        !s.has_blob(&digest).await,
        "blob must be freed once the last repo ref is gone"
    );

    // Deleting a digest with no ref entry hits the `None` branch => false.
    assert!(!s.delete_blob(&digest, "team/a").await);
}

// handlers/api_test.go `TestManifestAPI_DeleteTag*` + storage/manifeststore_test.go:
// delete by tag removes the manifest and prunes the now-empty repository;
// deleting an unknown tag returns false.
#[tokio::test]
async fn delete_manifest_by_tag_and_prunes_empty_repo() {
    let s = RegistryStorage::default();
    let body: &[u8] = b"{\"schemaVersion\":2,\"mediaType\":\"x\"}";
    s.store_manifest(
        "library/alpine",
        "latest",
        "application/vnd.oci.image.manifest.v1+json".to_string(),
        body.into(),
        None,
        None,
    )
    .await;
    assert_eq!(s.list_repos().await, vec!["library/alpine".to_string()]);

    // Deleting an unknown tag hits the `None => return false` branch.
    assert!(!s.delete_manifest("library/alpine", "nonexistent").await);

    // Deleting the only tag removes the manifest and prunes the repo.
    assert!(s.delete_manifest("library/alpine", "latest").await);
    assert!(
        s.list_repos().await.is_empty(),
        "repo must be pruned once its last manifest is gone"
    );
    assert!(s.get_manifest("library/alpine", "latest").await.is_none());
}

// storage/manifeststore_test.go (subject/referrers): OCI 1.1 referrers index
// returns every referrer for a subject, and filters by artifactType.
#[tokio::test]
async fn referrers_returns_subjects_and_filters_by_artifact_type() {
    let s = RegistryStorage::default();
    let subject = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    s.store_manifest(
        "app/repo",
        "sig",
        "application/vnd.oci.image.manifest.v1+json".to_string(),
        (b"referrer-signature" as &[u8]).into(),
        Some(subject.to_string()),
        Some("application/vnd.dev.cosign.simplesigning.v1+json".to_string()),
    )
    .await;
    s.store_manifest(
        "app/repo",
        "sbom",
        "application/vnd.oci.image.manifest.v1+json".to_string(),
        (b"referrer-sbom" as &[u8]).into(),
        Some(subject.to_string()),
        Some("application/spdx+json".to_string()),
    )
    .await;

    // Unfiltered: both referrers come back.
    let all = s.get_referrers(subject, None).await;
    assert_eq!(all.len(), 2);

    // Filtered: only the SPDX referrer matches.
    let spdx = s.get_referrers(subject, Some("application/spdx+json")).await;
    assert_eq!(spdx.len(), 1);
    assert_eq!(
        spdx[0].artifact_type.as_deref(),
        Some("application/spdx+json")
    );

    // A subject with no referrers returns an empty list (the `None => vec![]` branch).
    assert!(s
        .get_referrers(
            "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            None
        )
        .await
        .is_empty());
}

// distribution storage/blob_test.go `TestBlobMount`: a cross-repo mount must
// fail when the named source repo does not actually reference the blob.
#[tokio::test]
async fn mount_blob_false_when_source_repo_lacks_blob() {
    let s = RegistryStorage::default();
    let data: &[u8] = b"mountable-layer";
    let digest = compute_digest(data);
    s.store_blob(digest.clone(), data.into(), "owner/src").await;

    // Blob exists, but `wrong/src` never referenced it => `entry.contains` guard fails.
    assert!(!s.mount_blob(&digest, "wrong/src", "owner/dst").await);
    // The real source repo can mount it successfully.
    assert!(s.mount_blob(&digest, "owner/src", "owner/dst").await);

    // Mounting a digest that was never stored also returns false (`has_blob` guard).
    let missing = compute_digest(b"never-stored");
    assert!(!s.mount_blob(&missing, "owner/src", "owner/dst").await);
}

// client/blob_writer_test.go `TestUploadSize` + handlers `TestStartPushReadOnly`:
// upload offset tracks appended bytes; cancel removes the session.
#[tokio::test]
async fn cancel_upload_and_offset_query_lifecycle() {
    let s = RegistryStorage::default();
    let uuid = s.start_upload("library/busybox").await;

    // Fresh session => offset 0.
    assert_eq!(s.upload_offset(&uuid).await, Some(0));

    // Append 6 bytes => offset 6.
    assert_eq!(
        s.patch_upload(&uuid, (b"abcdef" as &[u8]).into()).await,
        Some(6)
    );
    assert_eq!(s.upload_offset(&uuid).await, Some(6));

    // Cancel removes the session and reports success.
    assert!(s.cancel_upload(&uuid).await);
    assert_eq!(s.upload_offset(&uuid).await, None);

    // Cancelling an unknown uuid returns false.
    assert!(!s.cancel_upload("00000000-0000-0000-0000-000000000000").await);
    // Patching a non-existent session returns None.
    assert_eq!(s.patch_upload(&uuid, (b"x" as &[u8]).into()).await, None);
}

// client/repository_test.go `TestOCIManifestFetch` / `TestManifestFetchWithAccept`:
// a tagged manifest is fetchable both by its tag and by its sha256 digest.
#[tokio::test]
async fn get_manifest_by_digest_and_by_tag() {
    let s = RegistryStorage::default();
    let body: &[u8] = b"{\"schemaVersion\":2,\"config\":{}}";
    let expected_digest = compute_digest(body);

    let digest = s
        .store_manifest(
            "library/redis",
            "7",
            "application/vnd.oci.image.manifest.v1+json".to_string(),
            body.into(),
            None,
            None,
        )
        .await;
    // store_manifest returns the content-addressed digest.
    assert_eq!(digest, expected_digest);

    let by_tag = s.get_manifest("library/redis", "7").await.unwrap();
    let by_digest = s.get_manifest("library/redis", &digest).await.unwrap();
    // Tag path and digest path resolve to the same content.
    assert_eq!(by_tag.digest, expected_digest);
    assert_eq!(by_digest.digest, expected_digest);
    assert_eq!(by_tag.data, by_digest.data);

    // Unknown references (both tag-form and digest-form) resolve to None.
    assert!(s.get_manifest("library/redis", "8").await.is_none());
    assert!(s
        .get_manifest(
            "library/redis",
            "sha256:3333333333333333333333333333333333333333333333333333333333333333"
        )
        .await
        .is_none());
}

// storage/catalog_test.go `TestCatalog*` + handlers `TestCatalogAPI`:
// the catalog is returned as a sorted, de-duplicated list of repositories.
#[tokio::test]
async fn list_repos_returns_sorted_repositories() {
    let s = RegistryStorage::default();
    let ct = "application/vnd.oci.image.manifest.v1+json".to_string();
    // Insert out of alphabetical order, with one repo touched twice.
    s.store_manifest("zeta/svc", "v1", ct.clone(), (b"z1" as &[u8]).into(), None, None)
        .await;
    s.store_manifest("alpha/svc", "v1", ct.clone(), (b"a1" as &[u8]).into(), None, None)
        .await;
    s.store_manifest("mid/svc", "v1", ct.clone(), (b"m1" as &[u8]).into(), None, None)
        .await;
    s.store_manifest("alpha/svc", "v2", ct.clone(), (b"a2" as &[u8]).into(), None, None)
        .await;

    // Sorted ascending and de-duplicated (alpha/svc appears once despite two tags).
    assert_eq!(
        s.list_repos().await,
        vec![
            "alpha/svc".to_string(),
            "mid/svc".to_string(),
            "zeta/svc".to_string()
        ]
    );
}

// storage/garbagecollect_test.go: GC parses each manifest and retains its
// config + layer blobs, removing only blobs unreferenced by any manifest.
#[tokio::test]
async fn gc_retains_blobs_referenced_by_manifest_config_and_layers() {
    let s = RegistryStorage::default();
    let repo = "library/node";

    // Real content blobs whose digests we feed into the manifest JSON.
    let config_bytes: &[u8] = b"{\"architecture\":\"amd64\",\"os\":\"linux\"}";
    let layer_bytes: &[u8] = b"layer-tar-gzip-payload";
    let orphan_bytes: &[u8] = b"unreferenced-orphan-blob";
    let config_digest = compute_digest(config_bytes);
    let layer_digest = compute_digest(layer_bytes);
    let orphan_digest = compute_digest(orphan_bytes);

    s.store_blob(config_digest.clone(), config_bytes.into(), repo).await;
    s.store_blob(layer_digest.clone(), layer_bytes.into(), repo).await;
    s.store_blob(orphan_digest.clone(), orphan_bytes.into(), repo).await;

    // An ImageManifest referencing the config + layer (parsed by gc()).
    let manifest_json = format!(
        "{{\"schemaVersion\":2,\
\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\
\"config\":{{\"mediaType\":\"application/vnd.oci.image.config.v1+json\",\"size\":{},\"digest\":\"{}\"}},\
\"layers\":[{{\"mediaType\":\"application/vnd.oci.image.layer.v1.tar+gzip\",\"size\":{},\"digest\":\"{}\"}}]}}",
        config_bytes.len(),
        config_digest,
        layer_bytes.len(),
        layer_digest
    );
    s.store_manifest(
        repo,
        "latest",
        "application/vnd.oci.image.manifest.v1+json".to_string(),
        manifest_json.into_bytes().into(),
        None,
        None,
    )
    .await;

    let stats = s.gc().await;

    // Only the orphan blob is removed.
    assert_eq!(stats.blobs_removed, 1);
    // blobs_retained counts the content-addressed blob store only: config + layer
    // (2). The manifest lives in the separate manifests map — its digest is added
    // to the live-set defensively but it is not a member of the blob store here.
    assert_eq!(stats.blobs_retained, 2);
    assert!(s.has_blob(&config_digest).await);
    assert!(s.has_blob(&layer_digest).await);
    assert!(
        !s.has_blob(&orphan_digest).await,
        "orphan blob must be collected"
    );
}
