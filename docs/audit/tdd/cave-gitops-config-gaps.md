# TDD coverage gap report — cave-gitops-config

| Field | Value |
| --- | --- |
| Cave crate | `crates/ops/cave-gitops-config` (theme: ops) |
| Upstream | argoproj/argo-cd (Go) |
| Upstream version | v3.4.2 |
| Upstream test files | 364 |
| Upstream test symbols | 2909 |
| Cave test fns | 33 (29 named unit/integration + 4 generic `proptest_smoke`) |

## Scope note (read first)

`cave-gitops-config` is **not** a port of argo-cd. Its `lib.rs` declares it
"Compatible with: Kratix" — a Promise / Pipeline platform-as-a-product API. The
only genuine argo-cd-derived behavior is `models::compare_state` /
`normalize_manifest`, ported from argo-cd `controller/state.go` `CompareAppState`
(desired-vs-live manifest comparison). That behavior is already well covered by
`tests/syncstatus_tdd.rs`.

Consequently the vast majority of the 2909 upstream test symbols
(ApplicationSet generators, SCM/pull-request providers, sync hooks & waves,
server-side / three-way diff, Lua health assessment, gitops-engine internals)
have **no cave counterpart** and are legitimate scope-cuts, not gaps.

Two source files — `src/composition.rs` and `src/promise.rs` — are **orphan
modules** (not declared in `lib.rs`; they reference an `AppState` / model shape
that does not exist in the wired crate). They are dead code and are excluded
from "cave impl?" judgements below.

The real, fillable gaps are **portable-coverage** gaps in cave's own wired
engine/store logic — behaviors cave implements but does not yet test. Upstream
test names are listed where an analogous argo-cd behavior exists; for cave-native
Kratix logic with no argo-cd analogue the upstream column is marked `n/a`.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
| --- | --- | --- | --- | --- | --- |
| Pipeline run fails when a Validate stage fails; later stages marked Skipped | `controller/state_test.go::TestCompareAppStateMissing` (failed/incomplete state) | yes — `engine::PipelineEngine::run_pipeline` (`failed` flag + Skipped branch) | no | portable-coverage | `test_run_pipeline_validate_failure_skips_remaining` |
| Configure stage overwrites existing keys (vs Transform's keep-existing) | n/a (Kratix pipeline semantics) | yes — `engine::PipelineEngine::execute_stage` Configure branch | no | portable-coverage | `test_configure_stage_overwrites_keys` |
| Deploy stage emits `{path, deployed:true}` with canonical state path | n/a | yes — `engine::PipelineEngine::execute_stage` Deploy branch | no | portable-coverage | `test_deploy_stage_outputs_state_path` |
| `validate_spec` rejects a non-object spec ("spec must be a JSON object") | `controller/state_test.go::TestCompareAppStateRepoError` (invalid-input rejection) | yes — `engine::PipelineEngine::validate_spec` (non-object branch) | no | portable-coverage | `test_validate_spec_rejects_non_object` |
| `select_destinations` with empty selectors matches all Ready clusters | n/a (Kratix destination selection) | yes — `engine::PipelineEngine::select_destinations` / `matches_selector` | no | portable-coverage | `test_select_destinations_empty_selectors_matches_all_ready` |
| `update_resource_request_status` mutates status + pipeline_run + destinations | n/a (store CRUD) | yes — `store::GitOpsStore::update_resource_request_status` | no | portable-coverage | `test_update_resource_request_status_sets_run_and_destinations` |
| `delete_resource_request` removes a request and returns true/false | n/a (store CRUD) | yes — `store::GitOpsStore::delete_resource_request` | no | portable-coverage | `test_delete_resource_request` |
| `update_pipeline_run` replaces an existing run by id | n/a (store CRUD) | yes — `store::GitOpsStore::update_pipeline_run` | no | portable-coverage | `test_update_pipeline_run_replaces_existing` |
| Notify stage emits `{notified:true}` | n/a | yes — `engine::PipelineEngine::execute_stage` Notify branch | no | portable-coverage | `test_notify_stage_outputs_notified` |
| Desired-vs-live manifest comparison → Synced/OutOfSync, missing live, whitespace normalize | `controller/state_test.go::TestCompareAppState{Empty,Missing}` | yes — `models::compare_state` / `normalize_manifest` | YES (`tests/syncstatus_tdd.rs`) | (covered) | — |
| JSON-schema required-field / type validation of resource spec | `controller/state_test.go` (manifest validation) | yes — `engine::validate_spec` | YES (3 tests) | (covered) | — |
| Promise dependency resolution (missing / inactive / success) | n/a (Kratix) | yes — `engine::resolve_dependencies` | YES (3 tests) | (covered) | — |
| Three-way / server-side diff of live vs desired objects | `gitops-engine/pkg/diff/diff_test.go::TestThreeWayDiff*`, `TestServerSideDiff` | no — cave only does normalized string equality | no | scope-cut | — (out of scope: structural k8s diff engine not ported) |
| ApplicationSet generators (git/list/cluster/matrix/merge/scm/pull-request) | `applicationset/generators/*_test.go` | no | no | scope-cut | — (out of scope: ApplicationSet not part of cave) |
| Sync hooks, sync waves, phases, sync-windows | `gitops-engine/pkg/sync/*_test.go`, `controller/sync_test.go::TestSyncWindow*` | no | no | scope-cut | — (out of scope: live k8s sync engine not ported) |
| Resource health assessment (Lua / built-in) | `gitops-engine/pkg/health/health_test.go`, `util/lua/health_test.go` | no | no | scope-cut | — (out of scope: no health subsystem in cave) |
| SCM / pull-request provider clients (github/gitlab/bitbucket/azure/gitea) | `applicationset/services/{scm_provider,pull_request}/*_test.go` | no | no | scope-cut | — (out of scope: vendor SCM integrations) |
| UI auto-sync component behavior | `ui/src/.../application-auto-sync.test.tsx` | no | no | scope-cut | — (out of scope: UI) |

## Recommended TDD fills (portable-coverage first)

These exercise behavior that **cave already implements in wired modules** but has
no test for. They are cheap, fully unit-testable, and require no new source code.

1. **`test_run_pipeline_validate_failure_skips_remaining`** — exercises
   `PipelineEngine::run_pipeline`. Build a promise whose first stage is `Validate`
   over an empty/`null` spec (so `validate_spec_basic` fails), followed by
   `Deploy` + `Notify`. Assert `run.status == PipelineRunStatus::Failed`, the
   Validate stage is `Failed`, and the trailing stages are `StageStatus::Skipped`.
   This is the single most valuable gap: the failure/skip path is the only
   untested branch of the core engine entrypoint.

2. **`test_configure_stage_overwrites_keys`** — exercises
   `PipelineEngine::execute_stage` Configure branch. Pass a `previous_output`
   containing `{"replicas": 1}` and a stage `config` of `{"replicas": 5}`; assert
   the output has `replicas == 5` (Configure *overwrites*, unlike Transform which
   uses `or_insert`). Pins the documented Transform-vs-Configure distinction.

3. **`test_deploy_stage_outputs_state_path`** — exercises
   `PipelineEngine::execute_stage` Deploy branch. Assert `output["deployed"] == true`
   and `output["path"]` equals the `state_store_path(...)` format for the request.

4. **`test_validate_spec_rejects_non_object`** — exercises
   `PipelineEngine::validate_spec`. Pass a JSON array or string as the spec; assert
   `Err` containing `"spec must be a JSON object"`. (Existing tests only cover
   object specs.)

5. **`test_select_destinations_empty_selectors_matches_all_ready`** — exercises
   `PipelineEngine::select_destinations`. With a promise that has **no**
   `destination_selectors`, assert all `ClusterStatus::Ready` clusters are
   returned and `NotReady` ones excluded (the `.all()` vacuous-truth path is
   currently untested).

6. **`test_update_resource_request_status_sets_run_and_destinations`** — exercises
   `GitOpsStore::update_resource_request_status`. Create a request, update it with
   a new status + a `PipelineRun` + a destinations vec, re-fetch, and assert all
   three fields plus a bumped `updated_at` are persisted; assert `false` for an
   unknown id.

7. **`test_delete_resource_request`** — exercises
   `GitOpsStore::delete_resource_request`. Assert `true` on delete of an existing
   request (and it is then absent), `false` on an unknown id.

8. **`test_update_pipeline_run_replaces_existing`** — exercises
   `GitOpsStore::update_pipeline_run`. Add a run, update it by id with a changed
   status, re-fetch via `get_pipeline_run`, assert the new status; assert `false`
   for an unknown id.

9. **`test_notify_stage_outputs_notified`** — exercises
   `PipelineEngine::execute_stage` Notify branch. Assert
   `output["notified"] == true` and `status == Completed`.

### Not recommended

The orphan `composition.rs` / `promise.rs` modules should be **wired or deleted**,
not tested as-is — they will not compile against the live crate. (Flagged
separately; outside this TDD-coverage report.)
