# Coverage Audit — cave-gitops-config vs Argo CD

- **Crate:** cave-gitops-config (`crates/ops/cave-gitops-config`)
- **Upstream:** Argo CD — https://github.com/argoproj/argo-cd
- **Tag / SHA:** `v3.4.2` / `0dc6b1b57dd5bb925d5b03c3d09419ab9fb4225e`
- **Upstream license:** Apache-2.0 (line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29
- **Stated honest_ratio (manifest):** 0.4667

## Framing / honest caveat

The cave crate is **NOT an Argo CD port**. `lib.rs` declares it "Compatible with: Kratix"
and implements a Kratix-style *Promise / Platform-as-a-product* config layer: a promise
registry, resource-request lifecycle, a 5-stage pipeline engine (Transform/Configure/
Deploy/Validate/Notify), an in-memory state store, cluster-destination selection, and a
9-route HTTP API. The parity manifest maps it to Argo CD `v3.4.2` and declares the bulk of
Argo CD's real CD surface as **skipped** ("owned by cave-deploy / cave-identity / portal-api /
cave-gateway / obs-stack"). This audit measures the crate against Argo CD's *actual*
functional architecture, so most rows are MISSING by design — the implemented logic only
loosely overlaps Argo CD at the conceptual level (declarative desired-state + multi-cluster
destinations + a YAML "git-like" state store).

Two source files exist on disk but are **NOT declared as modules** in `lib.rs`
(`composition.rs`, `promise.rs`) — they are orphaned/dead code and contribute nothing to the
compiled crate. They are excluded from the COVERED accounting below.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `pkg/apis/application/v1alpha1/types.go` | Application / Source / Destination / SyncPolicy CRD types | `models.rs` | PARTIAL | Cave has `Promise`/`ResourceRequest`/`ClusterDestination`/`StateStoreEntry` with serde models. Conceptually overlaps "desired state + destination" but is a Kratix data model, not Argo's `Application` spec (no `source`, `syncPolicy`, `revision`, `targetRevision`, `path`). |
| `pkg/apis/application/v1alpha1/app_project_types.go` | AppProject: source/dest allow-lists, roles, windows | — | MISSING | No project/tenancy boundary concept. Manifest defers to cave-identity, but no logic exists here. |
| `controller/appcontroller.go` | Application reconcile loop / operation processor / work queue | `engine.rs::run_pipeline` | PARTIAL | Cave runs a one-shot synchronous pipeline per request; no reconcile loop, no informer/work-queue, no resync, no continuous drift correction. |
| `controller/sync.go` + `gitops-engine` sync | Sync engine: waves, hooks, prune, sync-options, server-side apply | `engine.rs` (Deploy stage) | MISSING | Deploy stage only emits `{"path":..,"deployed":true}` — no apply, no sync waves, no hooks, no prune, no SSA, no resource ordering. |
| `controller/state.go` | Desired-vs-live comparison → OutOfSync/Synced computation | `models.rs::SyncStatus` enum | MISSING | `SyncStatus{Synced,OutOfSync,Unknown,Error}` enum exists but is only ever written as a literal `Synced` in `routes.rs`; no comparison logic computes it. |
| `util/argo/diff/diff.go` | Structured 3-way diff / normalize / ignore-diffs | — | MISSING | No diffing at all. |
| `util/argo/normalizers/*` | Known-type & JSON-pointer diff normalizers | — | MISSING | No normalization. |
| `controller/health.go` + `util/lua` | Resource health assessment (Lua hooks, built-in checks) | `models.rs::ClusterStatus` | MISSING | Only a coarse `ClusterStatus{Ready,NotReady,Unknown}` enum on the *cluster*; no per-resource health, no Lua. |
| `reposerver/repository/repository.go` | Manifest generation from a git repo (clone → render → return) | `engine.rs` Transform/Configure | PARTIAL | Cave "renders" by shallow JSON merge of stage `config` into the request spec. No git, no repo, no templating beyond key-merge. |
| `util/helm/client.go` | Helm chart fetch + `helm template` rendering | — | MISSING | Out of scope per NO-helm policy; zero logic. |
| `util/kustomize/kustomize.go` | Kustomize build rendering | — | MISSING | No kustomize. |
| `util/git/client.go` + `creds.go` | Git clone/fetch/ls-remote, credential & SSH handling | `models.rs::StateStoreEntry.path` | MISSING | A `clusters/.../x.yaml` path string is the only "git" artifact; no repo I/O, no creds, no revision resolution. |
| `applicationset/generators/*` (list/cluster/git/matrix/merge/scm/pullrequest/plugin/duck) | ApplicationSet generators producing N apps from templates | — | MISSING | No ApplicationSet / generator concept. Deferred to cave-deploy in manifest. |
| `applicationset/controllers` | ApplicationSet reconcile + template interpolation | — | MISSING | None. |
| `util/webhook/webhook.go` | Git provider webhooks (GitHub/GitLab/Bitbucket) → refresh trigger | — | MISSING | No webhook ingress; manifest defers to cave-gateway. |
| `util/rbac/rbac.go` | Casbin RBAC enforcement (policy.csv, built-in roles) | — | MISSING | No authz on any route; `requester: Uuid` is stored but never checked. Deferred to cave-identity. |
| `server/session` + `util/session` | Login / JWT session / token issue & verify | — | MISSING | No auth/session layer. |
| `util/oidc` + `util/dex` | OIDC / Dex SSO integration | — | MISSING | None. |
| `server/cluster` + `util/db` + `util/clusterauth` | Cluster credential store, kubeconfig/bearer auth, connection state | `store.rs` cluster CRUD + `engine.rs::select_destinations` | PARTIAL | Cave registers clusters (name/api_server/labels) and selects by label equality. No credentials, no kubeconfig, no live connection check; `ClusterStatus` is set manually. |
| `server/repository` + `server/repocreds` | Repo & repo-credential registration API | — | MISSING | No repo concept. |
| `controller/cache/cache.go` + `info.go` | Live cluster resource cache / managed-resource tree | — | MISSING | No live cluster cache. |
| `server/server.go` (gRPC+REST API) | Full API server: applications/projects/clusters/repos/sessions/settings | `routes.rs` (9 axum routes) | PARTIAL | A small REST surface exists for promises/requests/state/clusters/pipelines, but it is the Kratix API, not Argo's `application.ApplicationService` gRPC/REST. No project/repo/session/settings/account endpoints. |
| `server/notification` + `util/notification` | Notification triggers/templates/services (Notify) | `engine.rs` Notify stage | PARTIAL | Notify stage only emits a `tracing::info!` log line — no triggers, templates, or delivery services. Deferred to cave-knative. |
| `util/gpg` + `server/gpgkey` + `server/certificate` | GPG signature verification + TLS cert management | `models.rs::StateStoreEntry.checksum` | MISSING | `checksum` is `format!("{:x}", id)` (an id hash, not a content digest); no GPG, no signature verify, no cert store. |
| `util/argo/argo.go` validation (`ValidateRepo`, schema, refresh) | App/spec validation, refresh orchestration | `engine.rs::validate_spec` | PARTIAL | A genuine (small) JSON-Schema-subset validator: checks `required[]` presence + scalar `type` of `properties`. No `$ref`, no nested objects, no enum/format/min/max, no array item schemas. This is the strongest real overlap. |
| `controller/metrics` + `server/metrics` | Prometheus metrics exporters | — | MISSING | No metrics. Deferred to obs-stack. |

### Status summary (against Argo CD's real architecture)

- Modules enumerated: **26**
- COVERED: **0**
- PARTIAL: **7** (`models` types, reconcile/pipeline, repo render via merge, cluster mgmt+destination select, API surface, notify, spec validation)
- MISSING: **19**

## Actionable gaps for strict-TDD

Ordered lowest-effort-highest-value first. Each gap names the upstream reference and a
concrete failing test (RED-first).

1. **Desired-vs-live sync status computation** — `controller/state.go` (`CompareAppState`).
   The `SyncStatus` enum is written only as a literal `Synced`; nothing computes it.
   - Test: `sync_status_is_outofsync_when_live_differs_from_desired`
   - Assert: given a desired `StateStoreEntry.content` and a supplied "live" manifest that
     differs, a `compare_state(desired, live)` fn returns `SyncStatus::OutOfSync`; identical
     inputs return `SyncStatus::Synced`. (Currently no such function exists.)

2. **Content checksum is a real digest** — `controller/state.go` / `util/hash`.
   `StateStoreEntry.checksum` is `format!("{:x}", id.as_u128())`, unrelated to content.
   - Test: `state_entry_checksum_changes_with_content`
   - Assert: two upserts to the same path with different `content` produce different
     `checksum` values, and equal content produces equal checksums (i.e. checksum is a
     hash of `content`, not of the id).

3. **JSON-Schema validation depth (nested objects + enum)** — `util/argo/argo.go` /
   schema validation in `reposerver`. `validate_spec` only checks top-level `required`
   and scalar `type`.
   - Test: `validate_spec_rejects_value_outside_enum` and
     `validate_spec_recurses_into_nested_object_required`
   - Assert: a property with `{"enum":["a","b"]}` rejects `"c"`; a nested
     `{"type":"object","required":["x"]}` reports a missing nested field. Both currently
     pass-through as valid.

4. **AppProject source/destination allow-list enforcement** —
   `pkg/apis/application/v1alpha1/app_project_types.go` (`IsDestinationPermitted`).
   No tenancy boundary exists.
   - Test: `resource_request_denied_when_destination_not_permitted_by_project`
   - Assert: introduce a `Project` with an allowed-destination list; a `ResourceRequest`
     targeting a cluster outside the list is rejected (HTTP 403 / `Err`), one inside is
     accepted.

5. **Sync waves / hook ordering in the pipeline** — `controller/sync.go` +
   `controller/hook.go` (sync-wave annotation ordering, PreSync/PostSync hooks).
   The Deploy stage is a no-op stub.
   - Test: `pipeline_executes_stages_in_sync_wave_order`
   - Assert: stages carrying a `wave`/`order` annotation execute in ascending wave order
     regardless of declaration order, and a PreSync-equivalent stage completes before the
     Deploy stage. (Today only the `order` field exists on `PipelineStage` and is never
     used to sort.)

6. **Structured 3-way diff between desired and live manifests** — `util/argo/diff/diff.go`
   (`Normalize`, `Diff`, ignore-diffs).
   - Test: `diff_reports_changed_added_removed_keys`
   - Assert: a `diff(desired, live)` fn returns the set of changed/added/removed JSON
     pointers for two manifests; an ignored path (per an ignore-diffs config) is excluded
     from the result. No diff logic exists today.
