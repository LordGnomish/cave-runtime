# TDD coverage audit — cave-workflows vs argoproj/argo-workflows @ v4.0.5

- **Cave crate:** `crates/orchestration/cave-workflows`
- **Upstream:** https://github.com/argoproj/argo-workflows @ `v4.0.5`
- **Upstream test files:** 293 · **upstream test symbols (Go `func TestXxx` / TS `test`/`it`):** 1294
- **Cave test fns** (`#[test]` / `#[tokio::test]`): 59 (incl. 11 Charter v2 self-audit assertions + 4 generic proptest smoke invariants; **~44 behavioral**)
- Audited: do NOT modify src, do NOT commit.

## Scope orientation

cave-workflows is a **pure-function reducer port** of the Argo controller's scheduling
core, not the full Argo server/controller/executor binary. The behavioral surface that
maps 1:1 to upstream lives in:

- `workflow_crd.rs` — Workflow / WorkflowSpec / Template CRD shapes + `validate()` +
  `topo_order()` (mirrors `pkg/apis/workflow/v1alpha1/workflow_types.go` +
  `util/sorting/topological_sorting`).
- `executor.rs` — `next_actions()` / `aggregate_phase()` / `retry_decision()` /
  `record_success()` / `apply_parameter_defaults()` / `parse_duration_seconds()` /
  `template_index()` (mirrors `workflow/controller/{dag,steps,operator}`).
- `store.rs` — in-memory CRUD (mirrors `server/workflow/store` + workflow_server CRUD).

`models.rs` + `engine.rs` are a **legacy generic node/edge workflow** model (n8n-era,
NodeType::Trigger/Action/…) that predates the Argo repoint. It is fully tested
(17 tests) but has **no upstream counterpart** in argo-workflows v4.0.5 — it is not
scored below (the Argo behavior it overlaps, DAG validation + toposort, is covered
honestly by `workflow_crd.rs`).

The vast majority of the 1294 upstream symbols are **scope-cut**: server/apiserver,
auth/SSO, gRPC, artifact *drivers* (S3/GCS/Azure/Git/HTTP/OSS wire I/O), CLI commands,
UI (TS/TSX), telemetry/Prometheus, persistence/sqldb, e2e suites, manifests/codegen.
cave-workflows deliberately models artifact *repositories* as data (serde), not drivers.

## Behavior → coverage table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| DAG topological order (deps first) | `util/sorting/topological_sorting_test.go::TestTopologicalSorting_ValidInput*` | `workflow_crd::topo_order` | yes (`dag_toposort_orders_dependencies_first`) | covered | — |
| DAG cycle rejection | `…TestTopologicalSorting_GraphWithCycle*`, `validation_utils_test.go::TestVerifyNoCycles` | `topo_order` / `detect_cycle_in_dag` | yes (`dag_toposort_rejects_cycle`, `validation_catches_cycle_in_workflow_dag`) | covered | — |
| Entrypoint validation | `workflow_types_test.go::TestGetTemplateByName`, e2e validation | `Workflow::validate` | yes (`workflow_validates_…`, `…rejects_missing_entrypoint`) | covered | — |
| Steps missing-template ref rejected | `examples/validation_test.go`, `common/util_test.go` | `Workflow::validate` (Steps arm) | yes (`steps_validation_rejects_missing_template_reference`) | covered | — |
| DAG root scheduled first, gated child held | `workflow/controller/dag_test.go::Test*Dag*` (sequencing) | `executor::next_actions`→`schedule_dag` | yes (`next_actions_schedules_dag_roots_first`) | covered | — |
| Gated DAG child unblocks after dep Succeeded | `dag_test.go` dependency-satisfied path | `schedule_dag` deps_ready | yes (`next_actions_unblocks_b_after_a_succeeds`) | covered | — |
| **Steps groups run sequentially (group N+1 waits for group N)** | `workflow/controller/steps_test.go::TestStepsFailedRetries` / e2e steps sequencing | `executor::next_actions`→`schedule_steps` + `group_complete` | **NO** | **portable-coverage** | drive `next_actions` on a 2-group Steps entrypoint; assert only group-0 scheduled, then `record_success` group-0 and assert group-1 scheduled |
| **Steps: all members of a group scheduled together** | `steps_test.go` parallel-within-group | `schedule_steps` (any_pending loop) | **NO** | **portable-coverage** | Steps entrypoint with 2 steps in one group → `next_actions` returns both Schedule actions |
| **aggregate phase = Suspended when any node Suspended** | `workflow_types_test.go::TestWorkflowPhase_Completed`, controller suspend | `executor::aggregate_phase` (Suspended arm) | **NO** | **portable-coverage** | insert a Suspended NodeStatus → `aggregate_phase` == `Suspended` |
| **aggregate phase = Error propagation** | `workflow_types_test.go` phase precedence | `aggregate_phase` (Error arm) | **NO** | **portable-coverage** | insert an Error node → `aggregate_phase` == `Error`; mix Error+Failed → Error wins precedence |
| aggregate phase Pending on empty | controller initial phase | `aggregate_phase` | yes (`aggregate_phase_pending_on_empty_nodes`) | covered | — |
| aggregate phase Failed propagation | phase precedence | `aggregate_phase` | yes (`aggregate_phase_failed_propagates`) | covered | — |
| aggregate phase Succeeded all-done | terminal phase | `aggregate_phase` | yes (`aggregate_phase_succeeds_when_all_succeed`) | covered | — |
| retry: limit boundary | `workflow_types_test.go::TestTemplate_RetryStrategy`, retry e2e | `executor::retry_decision` | yes (`retry_decision_respects_limit`) | covered | — |
| retry: OnFailure/OnError policy filter | retry policy matrix | `retry_decision` | yes (`retry_decision_filters_by_policy`) | partial | — |
| **retry: `Always` policy retries on any phase** | retry policy `Always` matrix | `retry_decision` (`"Always"` arm) | **NO** | **portable-coverage** | `retry_decision(Always, …, Succeeded/Failed/Error)` all → `Some(n+1)` |
| **retry: `OnTransientError` retries on Error** | retry policy `OnTransientError` | `retry_decision` (OnTransientError arm) | **NO** | **portable-coverage** | `retry_decision(OnTransientError, …, Error)` → `Some`; on `Failed` → `None` |
| **retry: unknown/None strategy → no retry** | nil-strategy guard | `retry_decision` (`strategy?` + unknown policy fallthrough) | **NO** | **portable-coverage** | `retry_decision(None,…)` == `None`; bogus policy string → `None` |
| suspend template emits Suspend w/ parsed duration | `suspend_test.go` (e2e) | `next_actions` Suspend arm | yes (`suspend_template_emits_suspend_action`) | covered | — |
| **suspend w/o duration → indefinite (`None`)** | suspend manual-resume e2e | `next_actions` Suspend arm (`duration: None`) | **NO** | **portable-coverage** | Suspend template, `duration: None` → `NodeAction::Suspend { duration_seconds: None }` |
| script template → container dispatch | container_set / script controller | `next_actions` Script arm | yes (`script_template_compiles_into_container_dispatch`) | covered | — |
| **resource template → schedule dispatch** | `resource_template_test.go` (e2e) | `next_actions` Resource arm | **NO** | **portable-coverage** | Resource entrypoint → `next_actions` first == `Schedule` |
| duration string parse (s/m/h/d) | `util/humanize`, strftime/env duration | `executor::parse_duration_seconds` | yes (`parse_duration_handles_units`) | covered | — |
| param default inheritance into bindings | `common/util_test.go::TestOverridableTemplateInputParamsValue` | `ArgumentBindings::from` | yes (`argument_bindings_inherit_defaults`) | covered | — |
| apply declared param defaults to supplied | `TestOverridableDefaultInputArts` / `TestParamsMerge` | `executor::apply_parameter_defaults` | yes (`apply_parameter_defaults_fills_missing`) | covered | — |
| **`record_success` sets outputs + finished_at + node status** | controller node-completion + `workflow_types_test.go` finished/started | `executor::record_success` | **NO** (only asserted transitively via aggregate) | **portable-coverage** | `record_success` then read back the NodeStatus: phase Succeeded, `outputs.is_some()`, `finished_at.is_some()`; non-terminal aggregate leaves wf.finished_at None |
| **`template_index` builds name→Template map** | `TestGetTemplateByName` | `executor::template_index` | **NO** | **portable-coverage** | build index over a 2-template spec; assert lookup hits + len |
| artifact repository serde roundtrip (S3/…/HDFS) | `workflow_types_test.go::TestS3Artifact/TestGCSArtifact/TestGitArtifact/…` (per-driver shape) | `ArtifactRepository` enum serde | partial (`artifact_repository_roundtrips_through_serde` — S3 only) | portable-coverage | extend: Git+Oss+Hdfs+Raw variants each roundtrip (tag/field shape) |
| RetryStrategy+backoff serde | `workflow_types_test.go` backoff fields | serde | yes (`retry_strategy_with_backoff_roundtrips`) | covered | — |
| six template variants construct | template-type enumeration | `TemplateBody` | yes (`all_six_template_variants_construct`) | covered | — |
| store CRUD roundtrip / dup / delete-missing / update | `server/workflow/store/sqlite_store_test.go::TestStoreOperation`, `workflow_server_test.go` CRUD | `store::WorkflowStore` | yes (4 tests) | covered | — |
| store `list(namespace)` filters by ns | `TestListWorkflow` namespace scoping | `WorkflowStore::list` Some(ns) arm | partial (round-trip test lists, never asserts a 2nd-namespace wf is excluded) | portable-coverage | create wf in ns "argo" + ns "prod"; `list(Some("argo"))` excludes prod |
| **DAG task `when:` conditional gating** | `functional_test.go` / expr conditionals | field present (`DagTask.when`), **not evaluated** by `schedule_dag` | n/a | **missing-impl** | (no test — conditional execution not implemented; flag as impl gap) |
| CronWorkflow schedule / next-runtime | `cron/util_test.go::TestNextRuntime*`, `cron_workflow_types_test.go` | not modeled | n/a | scope-cut | — (cron controller out of port scope) |
| Artifact *driver* I/O (S3/GCS/Azure/Git/HTTP wire) | `workflow/artifacts/**/*_test.go` | repositories modeled as data only | n/a | scope-cut | — |
| Server / auth / SSO / gRPC / CLI / UI / telemetry / sqldb | `server/**`, `cmd/**`, `ui/**`, `util/telemetry/**`, `persist/**` | not in port | n/a | scope-cut | — |

## Recommended TDD fills (portable-coverage first)

Each names the exact **public cave fn** the test would exercise. These are behaviors
the crate *already implements* but does not yet assert — highest ROI, zero src change.

1. `cave_workflows::executor::next_actions` — **Steps sequencing**: 2-group Steps
   entrypoint; assert only group-0 steps scheduled, then `record_success` the group-0
   node and re-call `next_actions` to assert group-1 scheduled. (`schedule_steps` +
   `group_complete` path is wholly untested today.)
2. `cave_workflows::executor::next_actions` — **Steps fan-out**: single group with two
   steps returns two `Schedule` actions.
3. `cave_workflows::executor::aggregate_phase` — **Suspended** branch: any Suspended
   node ⇒ `WorkflowPhase::Suspended`.
4. `cave_workflows::executor::aggregate_phase` — **Error** branch + precedence
   (Error beats Failed).
5. `cave_workflows::executor::retry_decision` — **`Always`** policy retries regardless
   of last phase (Succeeded/Failed/Error) up to `limit`.
6. `cave_workflows::executor::retry_decision` — **`OnTransientError`** retries on
   `Error`, declines on `Failed`.
7. `cave_workflows::executor::retry_decision` — **None strategy / unknown policy** ⇒
   `None`.
8. `cave_workflows::executor::next_actions` — **Suspend indefinite**: SuspendTemplate
   with `duration: None` ⇒ `NodeAction::Suspend { duration_seconds: None }`.
9. `cave_workflows::executor::next_actions` — **Resource** template entrypoint emits a
   `Schedule` action.
10. `cave_workflows::executor::record_success` — read-back assertion: completed
    NodeStatus carries `phase == Succeeded`, `outputs.is_some()`, `finished_at.is_some()`;
    a non-terminal aggregate leaves `wf.status.finished_at` unset.
11. `cave_workflows::executor::template_index` — name→Template map: correct lookups +
    map length over a multi-template spec.
12. `cave_workflows::workflow_crd::Artifact` serde — extend repository roundtrip beyond
    S3 to **Git / Oss / Hdfs / Raw** variants (tag + field-shape parity with upstream
    per-driver `TestGitArtifact`/`TestGCSArtifact`/… shape tests).
13. `cave_workflows::store::WorkflowStore::list` — namespace filtering excludes
    other-namespace workflows (currently only the happy single-ns count is asserted).

### Missing-impl (one real gap, not a test gap)

- `schedule_dag` / `schedule_steps` ignore the `when:` conditional on `DagTask` /
  `WorkflowStep` (the field is parsed but never evaluated). Upstream gates task
  execution on `when`. This is an implementation gap, not a missing test — a TDD fill
  here would be RED until `when` evaluation lands.
