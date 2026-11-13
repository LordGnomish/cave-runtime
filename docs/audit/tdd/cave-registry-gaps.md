# TDD Coverage Gap Report — cave-registry

| Field | Value |
|---|---|
| Crate | cave-registry (theme: registry) |
| Upstream | https://github.com/distribution/distribution (Go, OCI Registry v2) |
| Upstream version | v3.1.1 |
| Upstream test-symbol count | 315 (80 test files) |
| cave test-fn count (cave-registry crate) | 4 (generic proptest only — no behavioral tests) |
| Real impl location | `cave_artifacts::harbor` (cave-registry is a re-export alias) |

## Scope note (read first)

`cave-registry/src/lib.rs` is a 15-line alias: `pub use cave_artifacts::harbor::{RegistryState, router}`.
The registry behavior actually lives in the **cave-artifacts** crate under `src/harbor/`.
This audit therefore measures the OCI-distribution-spec surface inside
`cave_artifacts::harbor::storage` (blob / upload / manifest / tag / referrer / gc),
plus `harbor::proxy` (pull-through cache) and `harbor::webhook` (notifications).

The upstream is **distribution/distribution** — the low-level OCI registry. cave's
implementation is **Harbor**, a product built *on top of* distribution. The two overlap
on the registry data-plane (blobs/manifests/tags/gc) but Harbor adds RBAC, quota,
replication, retention, scanning that are *not* in distribution, and distribution has
pluggable storage drivers (S3/GCS/Azure/filesystem), token-JWT auth, and an OCI-blob
proxy-with-scheduler that Harbor delegates to the embedded registry. Those are honest
scope-cuts, marked below. Per the task, all gap-table cave-test/impl references point at
symbols in `crates/registry/cave-artifacts/src/harbor/`.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Blob ref-count delete (free blob only when last repo ref dropped) | storage/blob_test.go `TestBlobMount` / linkedblobstore | yes — `RegistryStorage::delete_blob` | no | portable-coverage | `delete_blob_frees_only_after_last_repo_ref` |
| Manifest delete by tag vs digest; repo pruned when empty | handlers/api_test.go `TestManifestAPI_DeleteTag*`, storage/manifeststore_test.go | yes — `RegistryStorage::delete_manifest` | no | portable-coverage | `delete_manifest_by_tag_and_prunes_empty_repo` |
| OCI 1.1 referrers + artifactType filter | storage/manifeststore_test.go (subject/referrers) | yes — `RegistryStorage::get_referrers` | no | portable-coverage | `referrers_returns_subjects_and_filters_by_artifact_type` |
| Cross-repo mount fails when source lacks blob | storage/blob_test.go `TestBlobMount`, client/repository_test.go `TestBlobMount` | yes — `RegistryStorage::mount_blob` | partial (only happy path `cross_repo_mount`) | portable-coverage | `mount_blob_false_when_source_repo_lacks_blob` |
| Upload cancel + offset query lifecycle | client/blob_writer_test.go `TestUploadSize`, handlers `TestStartPushReadOnly` | yes — `cancel_upload`, `upload_offset` | no | portable-coverage | `cancel_upload_and_offset_query_lifecycle` |
| Manifest fetch by digest vs by tag | client/repository_test.go `TestOCIManifestFetch`, `TestManifestFetchWithAccept` | yes — `RegistryStorage::get_manifest` | partial (tag path via `manifest_store_and_tag`) | portable-coverage | `get_manifest_by_digest_and_by_tag` |
| Catalog enumeration (sorted repo list) | storage/catalog_test.go `TestCatalog*`, handlers `TestCatalogAPI` | yes — `RegistryStorage::list_repos` | partial (only `catalog_empty`) | portable-coverage | `list_repos_returns_sorted_repositories` |
| complete_upload restores session on digest mismatch (retry) | client/blob_writer_test.go `TestUploadReadFrom` | yes — `complete_upload` err-path re-inserts session | partial (`upload_digest_mismatch` checks err, not retry) | portable-coverage | `complete_upload_mismatch_preserves_session_for_retry` |
| Proxy blocklist / URL rewrite for pull-through | proxy/proxymanifeststore_test.go, proxyblobstore_test.go | yes — `ProxyClient::is_blocked`, `rewrite_urls` | yes (`blocklist_matches_ecosystem_name`, `rewrite_urls_for_pypi`) | covered | — |
| Webhook fires on matching event / disabled skipped | notifications/listener_test.go, bridge_test.go | yes — `WebhookManager::fire` | yes (`test_webhook_fires_for_matching_event`, `test_disabled_webhook_not_active`) | covered | — |
| sha256 digest compute + verify | storage/paths_test.go `TestDigestFromPath` | yes — `compute_digest`, `verify_digest` | yes (sha256_tdd.rs ×4) | covered | — |
| GC removes unreferenced blobs | storage/garbagecollect_test.go (14 fns) | yes — `RegistryStorage::gc` (partial: no manifest-list dangling-ref handling) | partial (`gc_removes_unreferenced_blobs`) | portable-coverage | `gc_retains_blobs_referenced_by_manifest_config_and_layers` |
| JWT token / Bearer access control | auth/token/token_test.go (`TestTokenVerify`, `TestAccessController`, JWKS) | no — harbor delegates auth to cave-auth/OIDC; no JWT registry-token impl | n/a | scope-cut | — (distribution token-auth server; cave uses cave-auth OIDC bindings) |
| Pluggable storage drivers (S3/GCS/Azure/filesystem) | storage/driver/*_test.go (~60 fns) | no — cave uses in-memory + cave-artifacts object store | n/a | scope-cut | — (cloud-vendor driver matrix, out of launch scope) |
| Proxy scheduler TTL eviction / blob-store concurrency | proxy/scheduler/scheduler_test.go, proxyblobstore_test.go | no — harbor proxy is pkg-ecosystem pull-through, no OCI-blob scheduler | n/a | scope-cut | — (different proxy design; distribution-internal scheduler) |
| Config parser / TLS cipher suites / health-check handlers | configuration/*, registry/registry_test.go, health/* | no — runtime config + health live in cave-runtime/cave-health | n/a | scope-cut | — (infra wiring owned by other crates) |

## Recommended TDD fills (portable-coverage first)

These exercise behavior cave **already implements** in
`crates/registry/cave-artifacts/src/harbor/storage.rs` but has no test for. Cheap, high value.

1. **`delete_blob_frees_only_after_last_repo_ref`** — `RegistryStorage::delete_blob`.
   store_blob a digest into two repos; delete from repo A → blob still present (ref count 1);
   delete from repo B → `has_blob` false. Verifies the ref-set pruning branch.

2. **`delete_manifest_by_tag_and_prunes_empty_repo`** — `RegistryStorage::delete_manifest`.
   Store one tagged manifest; delete by tag returns true and repo drops out of `list_repos`;
   deleting an unknown tag returns false (the `None => return false` branch).

3. **`referrers_returns_subjects_and_filters_by_artifact_type`** — `RegistryStorage::get_referrers`.
   store_manifest two referrers with distinct `artifact_type` and a shared `subject_digest`;
   assert unfiltered returns both and `Some(filter)` returns only the matching one.

4. **`mount_blob_false_when_source_repo_lacks_blob`** — `RegistryStorage::mount_blob`.
   Existing `cross_repo_mount` only covers success. Add: mount with a `from_repo` that never
   stored the blob → returns false (the `entry.contains(from_repo)` guard).

5. **`cancel_upload_and_offset_query_lifecycle`** — `cancel_upload` + `upload_offset`.
   start_upload → patch_upload → `upload_offset` reflects bytes; `cancel_upload` returns true
   then `upload_offset` returns None; cancelling an unknown uuid returns false.

6. **`get_manifest_by_digest_and_by_tag`** — `RegistryStorage::get_manifest`.
   Store a tagged manifest; fetch by `sha256:` digest and by tag both return the same entry;
   unknown reference returns None. Covers the digest/tag branch split.

7. **`list_repos_returns_sorted_repositories`** — `RegistryStorage::list_repos`.
   Store manifests into repos out of alpha order; assert `list_repos` is sorted and deduped.

8. **`complete_upload_mismatch_preserves_session_for_retry`** — `complete_upload`.
   After a digest-mismatch error, assert the session is still resumable (`upload_offset` is
   Some / a subsequent correct `complete_upload` succeeds). Verifies the re-insert retry path.

9. **`gc_retains_blobs_referenced_by_manifest_config_and_layers`** — `RegistryStorage::gc`.
   Store an `ImageManifest` JSON plus its config+layer blobs and an orphan blob; assert gc
   removes only the orphan and retains config/layer blobs (the manifest-parse retention branch,
   beyond the single existing happy-path test).

Scope-cuts (JWT token-auth server, cloud storage-driver matrix, proxy scheduler, config/TLS/
health handlers) are legitimately skipped: they are distribution-internal subsystems that cave
delegates to other crates (cave-auth OIDC, cave-artifacts object store, cave-runtime, cave-health)
or are cloud-vendor-specific. Implementing them as registry-token/driver stubs would be inflation.
