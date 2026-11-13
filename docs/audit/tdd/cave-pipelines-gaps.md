# TDD Coverage Audit — cave-pipelines

- **Cave crate:** `crates/orchestration/cave-pipelines` (theme: orchestration)
- **Upstream:** [tektoncd/pipeline](https://github.com/tektoncd/pipeline) @ **v0.55.0**
- **Upstream test files:** 229 — **1218 test symbols** (`func TestXxx`)
- **Cave test functions:** 79 (`#[test]` / `#[tokio::test]`) across `src/{engine,entrypoint,executor,models,notifications,triggers,workspace,catalog,github,build,jenkins}.rs` + `tests/{entrypoint_tdd,proptest_smoke}.rs`

## Scope orientation

cave-pipelines is **not** a 1:1 port of Tekton's reconciler/CRD-validation stack. Per
`ADR-RUNTIME-PARITY-100-PCT-001 §5`, the k8s control-plane surface (CRD admission-webhook
validation, reconciler controllers, pod lifecycle / sidecar mounts, config-map feature-flag
plumbing, resolvers, the `cmd/entrypoint` runtime binary, metrics/events configmaps) stays in
sibling crates (cave-cri, cave-apiserver) or is intentionally out of launch scope. The cave crate
ports the **pure behavioral cores**: DAG resolution, param/result substitution, when-expressions,
matrix fan-out, step-ordering (`orderContainers`), step execution, triggers/CEL interceptors,
plus value-add CI integrations (Jenkinsfile compat, GitHub status, build configs). The huge
upstream test count is dominated by CRD `*_validation_test.go` and `reconciler/*_test.go` files
that map to scope-cut.

The portable behavioral cores are **already well covered**. This audit finds **5 genuine gaps**,
of which **3 are portable-coverage** (cave implements it, no test) and **2 are missing-impl**
(behavior the cave fn does not implement, so it is honestly a feature gap not a test gap).

## Coverage table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| DAG topological waves / linear chain / fan-out | `dag/dag_test.go` Test* | `engine::Dag::execution_waves` | yes (`dag_linear_chain`, `dag_parallel_fan_out`) | covered | — |
| DAG cycle detection | `pipeline_validation_test.go TestPipelineSpec_Validate_Failure_CycleDAG` | `engine::Dag::execution_waves` → `DagError::CycleDetected` | yes (`dag_cycle_detected`) | covered | — |
| DAG unknown dependency | `dag/dag_test.go` (invalid graph) | `engine::Dag::execution_waves` → `UnknownDependency` | yes (`dag_unknown_dep`) | covered | — |
| DAG ancestors / transitive deps | `dag/dag_test.go` | `engine::Dag::ancestors` | yes (`dag_ancestors`) | covered | — |
| `$(params.X)` substitution | `apply_test.go TestApplyReplacements` / `params_test.go TestParams_ReplaceVariables` | `engine::resolve_param_string` | yes (`resolve_param_string_basic`) | covered | — |
| `$(tasks.T.results.R)` substitution | `resultref_test.go TestNewResultReference` | `engine::resolve_param_string` | yes (`resolve_task_result_reference`) | covered | — |
| param required / enum / type validation | `param_types_test.go TestParamEnum_*`, `TestValidatePipelineParameterVariables_*` | `engine::validate_params` | yes (4 tests) | covered | — |
| when-expr in / notIn | `when_validation_test.go TestWhenExpressions_Valid` | `models::WhenExpression::evaluate` | yes (2 tests) | covered | — |
| matrix cartesian fan-out | `matrix_types_test.go TestMatrix_FanOut`, `TestMatrix_GetAllParams` | `models::Matrix::expand` | yes (2 tests) | covered | — |
| matrix `include` (explicit combination merge) | `matrix_types_test.go TestMatrix_HasInclude` | **field exists, `expand()` ignores `include`** | no | **missing-impl** | (none — feature gap; `expand()` drops `Matrix.include`) |
| step ordering `orderContainers` (wait/post chain) | `pod/entrypoint_test.go` | `entrypoint::order_containers` | yes (`tests/entrypoint_tdd.rs`, 6 tests + 2 unit) | covered | — |
| `-step_results` / `-results` flag emission | `pod/entrypoint_test.go` | `entrypoint::order_containers` | yes (`step_results_flag_emitted_comma_joined`, `task_results_flag_emitted_comma_joined`) | covered | — |
| step exec: exit-0 / nonzero / stderr / param+env | `taskrun_test.go` (reconciler) | `executor::StepExecutor::execute` | yes (5 tokio tests) | covered | — |
| step timeout duration parse (`Xh/Xm/Xs`) | `taskrun_test.go` timeout cases | `executor::parse_timeout` (private) | indirect only | covered (private, exercised via `execute` timeout suffix) | — |
| finally always-runs | `pipelinerun_test.go` finally cases | `engine::should_run_finally` | yes (`should_run_finally_always_true`) | covered | — |
| CEL interceptor eval (==, !=, &&, \|\|, .matches) | `interceptors/cel/cel_test.go` | `triggers::evaluate_cel` | yes (4 tests) | covered | — |
| interceptor gate: CEL filter | `interceptors/cel/cel_test.go` | `triggers::passes_interceptors` (Cel arm) | yes (`passes_interceptors_cel`) | covered | — |
| interceptor gate: GitHub event-type filter | `interceptors/github/github_test.go` | `triggers::passes_interceptors` (GitHub arm) | yes (2 tests) | covered | — |
| interceptor gate: **Bitbucket event-type filter** | `interceptors/bitbucket/bitbucket_test.go` | `triggers::passes_interceptors` (Bitbucket arm, reads `x-event-key`) | **no** | **portable-coverage** | `passes_interceptors` with a `Interceptor::Bitbucket{event_types:["repo:push"]}` + matching/non-matching `x-event-key` header (mirror the GitHub pair) |
| workspace bind EmptyDir / PVC / get / cleanup | `workspace/apply_test.go` | `workspace::WorkspaceManager` | yes (5 tests) | covered | — |
| notify filter (onSuccess/onFailure/always/onComplete) | (downstream value-add) | `notifications::NotifyOn::matches` | yes (4 tests) | covered | — |
| notify dispatch **short-circuit when filter rejects** | (downstream value-add) | `notifications::send_notification` (early `Ok(())` when `!matches`) | **no** (only `matches` tested, not the gate inside `send_notification`) | **portable-coverage** | `send_notification` with an `OnSuccess` rule + a `Running` event over an `Email{...}` config (no network) asserting `Ok(())` and that no transport fires |
| GitHub commit-state mapping (incl. Skipped→Success) | `github_test.go` | `github::CommitState::from(&RunPhase)` | yes (3 tests cover all 6 phases) | covered | — |
| GitHub enterprise api_base override | `github_test.go` | `github::GitHubConfig::api_base` | yes (2 tests) | covered | — |
| Jenkinsfile parse (agent/env/post/stages) | (Jenkins-compat value-add) | `jenkins::parse_jenkinsfile` | yes (5 tests) | covered | — |
| Jenkinsfile → PipelineSpec (post→finally, slug) | (Jenkins-compat value-add) | `jenkins::to_pipeline_spec` | yes (3 tests) | covered | — |
| build config docker/kaniko/buildpacks cli args + interpolation | (build value-add) | `build::BuildConfig::{dockerfile,kaniko,cli_args,interpolated}` | yes (5 tests) | covered | — |
| catalog builtin/search/list/get | (catalog value-add) | `catalog::TaskCatalog` | yes (7 tests) | covered | — |
| `Matrix.include` merge into combinations | `matrix_types_test.go TestMatrix_HasInclude` / `TestMatrix_GetAllParams` | not merged (see above) | no | **missing-impl** | feature, not a test fill |
| CEL `.matches('regex')` path | `interceptors/cel/cel_test.go` | `triggers::evaluate_cel` (regex arm) | **no** (==/!=/&&/\|\| tested, `.matches` arm not) | **portable-coverage** | `evaluate_cel("body.ref.matches('refs/heads/.*')", body)` true + a non-matching pattern false |

## Recommended TDD fills (portable-coverage first)

These exercise behavior that **cave already implements** but has no test. Listed by exact public
cave fn:

1. **`cave_pipelines::triggers::passes_interceptors`** — Bitbucket arm. The `Interceptor::Bitbucket`
   branch (reads the `x-event-key` header, filters on `event_types`) is implemented but only the
   GitHub and CEL arms are tested. Add a matching/non-matching pair mirroring
   `passes_interceptors_github` / `passes_interceptors_github_filtered`.

2. **`cave_pipelines::triggers::evaluate_cel`** — `.matches('<regex>')` arm. Equality, inequality,
   AND, OR are tested; the regex `.matches(...)` branch (the only path that pulls in the `regex`
   crate) is untested. Add a true case (`body.ref.matches('refs/heads/.*')`) and a false case.

3. **`cave_pipelines::notifications::send_notification`** — filter short-circuit. `NotifyOn::matches`
   is unit-tested, but the gate *inside* `send_notification` (returns `Ok(())` without dispatching
   when `notify_on` does not match the event status) has no test. Use an `Email{...}` config (no
   outbound network) with an `OnSuccess` rule and a `Running` event; assert `Ok(())`. This is the
   only async-safe, network-free dispatch path and locks in the skip semantics.

### Not test gaps (recorded for honesty)

- **`models::Matrix::expand` ignoring `Matrix.include`** and the unimplemented include-merge are a
  *feature* gap (missing-impl), not a missing test. Writing a test would be RED-by-design against
  absent behavior; flag for implementation, do not pad the test count.
- The bulk of upstream's 1218 test symbols live in CRD `*_validation_test.go`,
  `reconciler/**/*_test.go`, `cmd/entrypoint/*_test.go` (runtime binary), resolver, and
  config-map/feature-flag tests — all **scope-cut** to cave-cri / cave-apiserver / out-of-launch
  per ADR-RUNTIME-PARITY-100-PCT-001 §5. They are correctly absent from this crate.
