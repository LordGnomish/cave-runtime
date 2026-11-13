# cave-artifacts ⇄ pulpcore parity audit

| Field | Value |
|-------|-------|
| Cave crate | `cave-artifacts` (sub-module `src/pulp/`) |
| Upstream | https://github.com/pulp/pulpcore |
| Upstream tag | `3.49.0` |
| Upstream commit SHA | `d555b04604053a77c9a2f180067859964589d297` |
| Upstream license | **GPL-2.0** |
| Port policy | **CLEAN-ROOM** — behavioral spec only. NO line-by-line porting of GPL-2.0 source. Every gap below must be filled from the upstream *behavior*, not its code. |
| Audit date | 2026-05-29 |
| Auditor | automated fresh-implementation coverage audit (read-only) |

> **Scope note.** `cave-artifacts` is a *consolidated* crate bundling four upstreams
> (`harbor`, `pulp`, `nexus`, `cosign`). This audit covers **only** the `pulp` sub-module
> (`src/pulp/`), which is the pulpcore parity surface. The other sub-modules are out of scope.

> **CRITICAL structural finding.** `src/pulp/mod.rs` declares only:
> `content, distribution, models, rbac, repair, repository, routes, signing, tasks, upload`.
> The files `src/pulp/store.rs`, `src/pulp/sync.rs`, `src/pulp/plugin.rs`,
> `src/pulp/plugins/*.rs`, `src/pulp/export.rs`, `src/pulp/error.rs`,
> `src/pulp/publication.rs` exist on disk but are **NOT declared as modules** — they are
> **dead code, not compiled** into the crate. The only sync engine, the plugin-registry
> abstraction, and the seven per-format plugin adapters (rpm/deb/python/container/ansible/
> maven/file) live entirely in this dead code. Their presence inflates the apparent surface;
> their behavior does not ship. `cargo build -p cave-artifacts` succeeds against the
> compiled set only (`ContentType`/`RemotePolicy` models — NOT the `ContentUnit`/`PluginType`/
> `DownloadPolicy` models the dead store/sync reference).

---

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `app/models/repository.py::Repository` | Repository CRUD, retain_repo_versions, autopublish | `pulp/models.rs::Repository`, `pulp/repository.rs` | **PARTIAL** | Struct + `create`/`update`/`versions_to_prune` exist with real prune logic. No persistence (in-memory only in dead `store.rs`); routes layer holds state. No `RepositoryContent` membership table, no `remote` resolution. |
| `app/models/repository.py::RepositoryVersion` | Immutable versions, content set add/remove, base_version, content_summary | `pulp/models.rs::RepositoryVersion` | **PARTIAL** | Type with `number`/`content_summary`/`complete`. No real add/remove-content set algebra, no version-completion transaction, no `RepositoryVersionContentDetails`. `enqueue`-only stubs in `repository.rs`. |
| `app/models/repository.py::Remote` | Remote config, download policy, TLS, proxy, rate-limit | `pulp/models.rs::Remote` | **PARTIAL** | Rich struct (tls/proxy/concurrency/retries/policy). Pure data; no connection logic, no credential resolution. |
| `app/tasks/base.py` + `sync.py` (plugin) | Synchronize remote→repo, mirror/optimize, on_demand staging | `pulp/sync.rs` (**DEAD CODE**) + `repository.rs::enqueue_sync` | **MISSING** | `sync.rs` has the only real sync loop but is **not compiled**; its `fetch_remote_index`/`fetch_content_data` are explicit stubs returning `vec![]`. Compiled path only enqueues a named task that never runs. No HTTP fetch, no diff, no mirror prune. |
| `download/base.py`,`http.py`,`file.py`,`factory.py` | Downloaders, digest/size validation during fetch, retries, throttling | — | **MISSING** | No downloader abstraction anywhere in compiled code. `BaseDownloader.validate_digests`/`validate_size` have no equivalent. |
| `app/models/content.py::Artifact` | Artifact w/ md5..sha512, init_and_validate, storage_path, dedup by digest | `pulp/models.rs::Artifact` | **PARTIAL** | All six digest fields present as `Option<String>`. No `init_and_validate` (compute+verify), no storage path derivation, no digest-based dedup/uniqueness. |
| `app/models/content.py::Content`/`ContentArtifact` | Master content, natural keys, artifact relations | `pulp/models.rs` per-format structs (RpmPackage, DebPackage, …) | **PARTIAL** | Concrete per-format structs exist with fields + `nevra()`/`coordinates()` helpers. No `Content` master abstraction, no natural-key dedup, no `ContentArtifact` link table. |
| `app/models/content.py` (digest verify) | Real checksum compute + compare | `pulp/content.rs::verify_sha256`, `pulp/repair.rs::check_artifact` | **MISSING** | `verify_sha256` only checks the *string is 64 hex chars* — it never hashes `data`. `check_artifact` compares declared size only. No real SHA-256 computation in the compiled verification path. |
| `app/models/publication.py::Publication` | Publish repo version → servable metadata, complete flag | `pulp/models.rs::Publication`, `pulp/distribution.rs::enqueue_publish` | **PARTIAL** | Type exists; publish is an enqueue-only stub. No metadata generation tied to a publication, no `PublishedArtifact`/`PublishedMetadata`. |
| content metadata generators (plugin) | repomd.xml / Packages index / PyPI simple+JSON | `pulp/content.rs::generate_repomd_xml`, `generate_deb_package_entry`, `generate_pypi_simple_page`, `generate_pypi_project_json` | **PARTIAL** | Real string/JSON generation exists and is tested. But repomd is a skeleton (no real primary.xml.gz, sizes estimated `len*512`), Deb has no `Packages.gz`/`Release`/`InRelease`, no GPG-signed Release. Genuinely useful but not production metadata. |
| `app/models/publication.py::Distribution`/`ArtifactDistribution` | base_path routing, source resolution, hidden, labels | `pulp/models.rs::Distribution`, `pulp/distribution.rs` | **COVERED** | Real `resolve_content_path`, `find_distribution_by_path`, `validate_distribution` (base_path uniqueness, `..` traversal reject, exactly-one-source). Closest-to-complete area. |
| `content/handler.py::Handler` | Serve content: match distro, stream artifact, on-demand remote stream, save-on-stream, directory listing, range/headers | `pulp/distribution.rs::resolve_content_path` | **MISSING** | Only path→relative-string resolution. No streaming, no `_match_and_stream`, no `_stream_remote_artifact` (pull-through), no `_save_artifact`, no directory HTML listing, no content-type/range headers. The core content-serving engine is absent. |
| `cache/cache.py::SyncContentCache` | Content-app response caching, Redis-backed, key derivation, invalidation | — | **MISSING** | No cache layer at all. |
| `app/models/publication.py::ContentGuard` (RBAC/Header/X509-redirect/Composite) | Authorize content downloads | `pulp/models.rs::ContentGuardType` (enum) | **PARTIAL** | Enum variants Rbac/Header/X509/ContentRedirect/Composite declared. No `permit()` evaluation logic — guards are never enforced on any served request. |
| `app/access_policy.py` + `global_access_conditions.py` + `role_util.py` | Statement-based access policies, has_model/domain/obj_perms, role assignment resolution | `pulp/rbac.rs` | **PARTIAL** | Real role→permission resolution with object scoping, group membership, wildcard `*.*`, `user_has_permission`/`get_user_permissions`, builtin roles. BUT no AccessPolicy statement engine (drf-access-policy), no domain-scoped checks, no `has_remote_param_obj_perms`-style conditions, and RBAC is never wired into route handlers (not enforced). |
| `app/models/task.py` + `tasking/worker.py` | Task model, states, worker, reserved-resource locking, dispatch, heartbeat, cancellation | `pulp/tasks.rs` | **PARTIAL** | Solid in-memory `Task`/`TaskState`/`TaskQueue`/`TaskGroup` with lifecycle, progress reports, purge. No worker process, no reserved-resource concurrency locking, no dispatch to a real executor, no DB-backed task table, no heartbeat/cleanup. Tasks enqueued by the compiled path never execute. |
| `app/models/progress.py::ProgressReport`/`GroupProgressReport` | Progress tracking on tasks | `pulp/tasks.rs::ProgressReport` | **PARTIAL** | Struct + `add_progress`. No `GroupProgressReport`, no streaming/persistence. |
| `app/tasks/upload.py` + `app/models/upload.py::Upload` | Chunked upload: create, append chunk, Content-Range, finalize→artifact | `pulp/upload.rs` | **COVERED** | Genuinely real: sequential offset enforcement, out-of-order/exceeds-size/already-completed errors, `parse_content_range`, finalize-requires-complete, registry. Strongest module. (Gap: finalize doesn't verify the declared sha256 against actual bytes — no bytes are stored.) |
| `app/tasks/orphan.py::orphan_cleanup` | Delete orphaned content/artifacts after protection time | — | **MISSING** | No orphan cleanup in compiled code. `ArtifactQuerySet.orphaned(protection_time)` has no equivalent. |
| `app/tasks/reclaim_space.py::reclaim_space` | Drop on-demand artifacts to reclaim disk, keeplist | `pulp/repair.rs::RepairReport.space_reclaimed_bytes` (field only) | **MISSING** | Field exists; no reclaim algorithm, no keeplist handling. |
| `app/tasks/purge.py` | Purge completed/old tasks by state+date | `pulp/tasks.rs::TaskQueue::purge_completed` | **PARTIAL** | Purges terminal tasks from memory. No date filter, no state-set filter, no DB. |
| `app/tasks/repair.py` (`repair_all_artifacts`) | Verify every artifact, re-download corrupt/missing | `pulp/repair.rs` | **PARTIAL** | `RepairOptions`/`RepairReport`/`ArtifactCheck` + `check_artifact`. Check is size-only (no hash); no re-download; `enqueue_repair` never runs. |
| `app/importexport.py` + `tasks/export.py`/`importer.py` | PulpExport/Import, chunked tar, TOC, create_repositories | `pulp/models.rs::PulpExport`/`PulpImport`/`ExportParams`/`ImportParams` | **PARTIAL** | Types model the API params. No tar streaming, no TOC generation/validation, no chunking, no import resource mapping. (`pulp/export.rs` is also dead code.) |
| signing (`SigningService`, `sign.py`) | Register signing service, run sign script, verify (GPG/x509/sigstore), RPM header sig | `pulp/signing.rs` | **PARTIAL** | Types + `verify_gpg_signature` (mock: length≥64 ⇒ valid), `rpm_has_signature` (real header-tag check), `CosignBundle`. No actual GPG/x509 crypto verify, no sign-script execution, no `SigningService.validate()`. |
| `app/models/acs.py::AlternateContentSource` | ACS: prefer cached/mirror sources during sync | — | **MISSING** | No ACS concept. |
| `app/models/domain.py::Domain` + multi-tenant storage | Domain isolation, per-domain storage backend | `Repository.labels`/`Distribution.labels` (loosely) | **MISSING** | No Domain model, no per-domain scoping or storage. |
| `app/models/storage.py` (FileSystem/Domain/S3/Azure) | Pluggable artifact storage backends | — | **MISSING** | No storage abstraction; artifacts have a `file: String` path field but nothing reads/writes blobs. |
| `app/replica.py` + `tasks/replica.py` | Replicate from an upstream Pulp instance | `pulp/models.rs` (none) | **MISSING** | No replica/upstream-pulp mirroring. |
| `app/models/status.py` + `viewsets` `/status/` | Server status, versions, online workers, storage | `pulp/routes.rs::status` | **PARTIAL** | A `/status/` route handler exists; returns static/fabricated status, no real worker/db/storage probing. |
| `app/viewsets/*` (filtering, pagination, ordering) | DRF viewsets: filter, paginate, order across all resources | `pulp/routes.rs` (`name__in` etc.) | **PARTIAL** | Axum routes for all major resources + `PaginatedResponse<T>` type + some filter query params. Filtering is shallow; no consistent ordering/cursor pagination; in-memory only. |
| plugin framework (`pulpcore/plugin/*`) | Plugin API: register content types, stages pipeline, declarative version | `pulp/plugin.rs` + `pulp/plugins/*` (**DEAD CODE**) | **MISSING** | The `ArtifactsPlugin` trait + 7 format adapters are not compiled. No declarative-content stages pipeline (`plugin/stages/`) anywhere. |

### Tally (pulp sub-module, 30 rows)
- COVERED: 2 (Distribution routing/validation, Chunked upload)
- PARTIAL: 16
- MISSING: 12

---

## Actionable gaps for strict-TDD

> All implementations must be **CLEAN-ROOM**: derive behavior from the documented/observed
> pulpcore semantics, never transcribe GPL-2.0 source.

### 1. Real SHA-256 verification (currently format-only) — lowest effort, highest value
- Upstream ref: `pulpcore/app/models/content.py::Artifact.init_and_validate`, `download/base.py::BaseDownloader.validate_digests`
- `pulp/content.rs::verify_sha256` accepts any 64-hex string without hashing `data`.
- Failing test idea: `fn verify_sha256_rejects_wrong_content()` — hash `b"hello"`, assert `verify_sha256(b"goodbye", "<sha256-of-hello>") == false` and `verify_sha256(b"hello", "<sha256-of-hello>") == true`. The `sha2` crate is already a dependency (used in dead `sync.rs`).

### 2. Repair `check_artifact` must hash, not just size-compare
- Upstream ref: `pulpcore/app/tasks/repair.py` (artifact checksum verification)
- `pulp/repair.rs::check_artifact` returns `Ok` whenever `sha256.len()==64 && data.len()==size` — corrupt-but-same-size data passes.
- Failing test idea: `fn check_artifact_detects_corruption_same_size()` — artifact with sha256 of `[0u8;1024]`, feed `[1u8;1024]` (same length, different bytes), assert `ArtifactCheck::Corrupted`.

### 3. ContentGuard enforcement (`permit`)
- Upstream ref: `pulpcore/content/handler.py::Handler._permit`, `app/models/publication.py::RBACContentGuard.permit`
- `ContentGuardType` enum has variants but no evaluation; guards never block a request.
- Failing test idea: `fn header_content_guard_denies_missing_header()` — build `ContentGuardType::Header{name,value}`, call `guard.permit(&request_without_header)` ⇒ `Err(Forbidden)`, and `permit(&request_with_matching_header)` ⇒ `Ok`.

### 4. Repository version content-set algebra (add/remove → new version)
- Upstream ref: `pulpcore/app/models/repository.py::RepositoryVersion`, `plugin/repo_version_utils.py`
- Compiled `add_content`/`remove_content` only enqueue a named task; no version is actually produced with the correct present/added/removed summary.
- Failing test idea: `fn new_version_computes_added_removed_summary()` — start from v1 with content {A,B}; add {C}, remove {A}; assert resulting version `number==2`, `present=={B,C}`, `added=={C}`, `removed=={A}`.

### 5. Orphan cleanup with protection time
- Upstream ref: `pulpcore/app/tasks/orphan.py::orphan_cleanup`, `ContentQuerySet.orphaned(orphan_protection_time)`
- No orphan deletion exists in compiled code.
- Failing test idea: `fn orphan_cleanup_respects_protection_time()` — content not in any repo version with `timestamp_of_interest` 1h ago and protection_time=24h is **kept**; same content aged 48h is **deleted**. Assert returned deleted-set membership.

### 6. Content serving / pull-through (on-demand remote stream + save)
- Upstream ref: `pulpcore/content/handler.py::Handler.stream_content` / `_stream_remote_artifact` / `_save_artifact`
- Only path→relative-string resolution exists; no actual artifact streaming or on-demand fetch-and-cache.
- Failing test idea: `fn on_demand_artifact_is_fetched_and_saved_on_first_request()` — distribution backed by a Remote(policy=OnDemand) with a content unit whose blob is absent; first `serve(path)` triggers a (mockable) fetch, returns the bytes, and a second `serve(path)` is served locally without re-fetch (assert fetch-count==1).
