# TDD coverage audit — cave-rollouts vs argoproj/argo-rollouts @ v1.9.0

- Crate: `crates/orchestration/cave-rollouts`
- Upstream: https://github.com/argoproj/argo-rollouts @ v1.9.0 (149 test files, 1401 test symbols)
- Cave test fns: **42** (`#[test]`/`#[tokio::test]` in `src/*.rs`: analysis 10, engine 8, experiment 10, notifications 5, traffic_router 9; models/routes/store/types 0)

## Scope framing

Upstream is a full Kubernetes controller: the overwhelming majority of its 1401 test
symbols exercise ReplicaSet reconciliation, Service/Ingress plumbing, informer/cache
wiring, CRD validation webhooks, `kubectl-argo-rollouts` CLI, metric-provider clients
(Prometheus/Datadog/CloudWatch/NewRelic/Kayenta/Wavefront/Graphite/SkyWalking/…), and
controller event loops. cave-rollouts is a **pure-logic port**: state-machine engine,
metric-condition evaluation, traffic-split manifest rendering, experiment lifecycle, and
notification payloads. Per the ADR-RUNTIME-PARITY scope-cut policy, all
controller/replicaset/informer/CRD-plumbing/metric-provider-client/CLI tests are
**scope-cut** — cave has no analogue to test. This audit only flags gaps where cave
**already implements** the behavior in source but has **no test**.

## Behavior table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| Metric success/failure condition eval (>=, <, ==, between) | `analysis/evaluate_test.go::TestEvaluateResultWith{Success,Failure}` | `types::MetricCondition::evaluate`; `analysis::evaluate_metric` | partial — `analysis::evaluate_metric` tested; `MetricCondition::evaluate` only indirectly | **portable-coverage** | direct unit test of `MetricCondition::evaluate` for GreaterThan/LessThan/Between boundaries |
| Engine string-condition eval (`result >= 0.95`) | `analysis/evaluate_test.go` (rego/expr eval) | `engine::evaluate_metric` + private `evaluate_simple_condition` | NO test for `engine::evaluate_metric` | **portable-coverage** | test `engine::evaluate_metric` (string-condition path, distinct from `analysis::evaluate_metric`) |
| Assess metric failure-limit | `analysis/analysis_test.go::TestAssessMetricStatusFailureLimit` | `analysis::evaluate_metric` (failure_count vs limit) | yes (`test_metric_failure_limit_allows_transient_failures`) | covered | — |
| Overall run phase from metric results | `analysis/analysis_test.go::TestAssessRunStatus` | `analysis::compute_analysis_phase` | yes | covered | — |
| Canary first step → SetWeight | `rollout/canary_test.go` (step increment) | `engine::advance_canary` | yes (`canary_first_step_sets_weight`) | covered | — |
| Canary pause step | `rollout/pause_test.go` | `engine::advance_canary` Pause arm | yes (`canary_second_step_pauses`) | covered | — |
| Canary all-steps-complete → Promote | `rollout/canary_test.go::TestCanaryRolloutUpdateStatusWhenAtEndOfSteps` | `engine::advance_canary` | yes (`canary_completed_promotes`) | covered | — |
| Failed analysis aborts canary | `rollout/analysis_test.go` (abort on fail) | `engine::advance_canary` analysis-fail arm | yes (`failed_analysis_aborts`) | covered | — |
| Max-weight clamp on SetWeight step | `rollout/canary_test.go::TestCanaryRolloutWithMaxWeightInTrafficRouting` | `engine::advance_canary` (`weight.min(strategy.max_weight)`) | NO | **portable-coverage** | `advance_canary` with a step weight above `max_weight` → asserts clamp |
| Canary `SetMirrorWeight` does not change canary weight | `rollout/canary_test.go` (mirror step) | `engine::advance_canary` SetMirrorWeight arm | NO | **portable-coverage** | `advance_canary` on a `SetMirrorWeight` step → canary_weight unchanged, step index advances |
| Aborted/Healthy/Paused short-circuit | `rollout/canary_test.go::TestCanaryRolloutNoProgressWhilePaused` | `engine::advance_canary` early returns | NO | **portable-coverage** | `advance_canary` with status.phase = Aborted/Healthy/Paused → NoOp/Pause |
| Manual full promote | `pkg/.../promote` | `engine::apply_canary_action` PromoteFull | yes (`manual_full_promote`) | covered | — |
| Manual abort / pause / resume / retry / promote-step | `rollout/controller_test.go` (action handling) | `engine::apply_canary_action` Abort/Pause/Resume/Retry/Promote arms | NO (only PromoteFull) | **portable-coverage** | `apply_canary_action` for each of Abort/Pause/Resume/Retry/Promote → asserts phase + decision |
| Blue/Green pre-promotion analysis trigger | `rollout/bluegreen_test.go::TestBlueGreenHandlePause` | `engine::advance_blue_green` | NO test for `advance_blue_green` | **portable-coverage** | `advance_blue_green` with pre_promotion set + preview_ready → RunAnalysis |
| Blue/Green auto-promote after delay | `rollout/bluegreen_test.go::TestBlueGreenHandlePauseAutoPromoteWithConditions` | `engine::advance_blue_green` auto_promote arm | NO | **portable-coverage** | `advance_blue_green` with `auto_promote_seconds>0` → Promote, phase Healthy |
| Blue/Green abort on analysis fail | `rollout/bluegreen_test.go::TestBlueGreenAbort` | `engine::advance_blue_green` fail arm | NO | **portable-coverage** | `advance_blue_green` with failed pre/post analysis → Abort, phase Aborted |
| Blue/Green manual-pause (no auto-promote) | `rollout/bluegreen_test.go::TestBlueGreenHandlePause` | `engine::advance_blue_green` manual arm | NO | **portable-coverage** | `advance_blue_green` with auto_promote 0 → Pause, phase Paused |
| Initial status factory per strategy | `rollout/controller_test.go` (status init) | `engine::initial_status` | NO | **portable-coverage** | `initial_status` for Canary/BlueGreen → asserts service names + weights + step index |
| Weight at a given step | `rollout/canary_test.go` (`GetCanaryReplicasOrWeight`) | `types::CanaryStrategy::weight_at_step` | NO | **portable-coverage** | `weight_at_step` returns SetWeight value, None for non-weight step |
| Terminal-phase predicate | implicit in status reconciliation | `types::RolloutPhase::is_terminal` | NO | **portable-coverage** | `is_terminal` true for Healthy/Degraded/Error, false otherwise |
| Experiment total variant weight | `experiments/experiment_test.go::TestExperimentInfo` (weights) | `types::Experiment::total_weight` (in types.rs) | NO | **portable-coverage** | `Experiment::total_weight` sums variant weights |
| Experiment lifecycle (pending→running→successful/failed/inconclusive) | `experiments/experiment_test.go::TestExperiment*` | `experiment::Experiment::{start,evaluate,abort,total_replicas}` | yes (10 tests) | covered | — |
| Traffic split clamp + sum-to-100 | `trafficrouting/*` weight calc | `traffic_router::WeightSplit::new` | yes | covered | — |
| Per-provider patch rendering (Istio/SMI/NGINX/ALB/Apisix/Plugin) | `trafficrouting/{istio,smi,nginx,alb,apisix,plugin}_test.go` | `traffic_router::render_patch` | yes (6 provider tests) | covered | — |
| Notification payload / Slack / webhook / should_notify | `notification/*` (upstream notifications engine) | `notifications::*` | yes (5 tests) | covered | — |
| ReplicaSet reconcile, scale up/down, collision | `rollout/replicaset_test.go`, `experiments/replicaset_test.go` | none | n/a | **scope-cut** (controller/k8s) | — |
| Service/Ingress sync, AWS target-group verify | `rollout/service_test.go`, `ingress/*` | none | n/a | **scope-cut** (controller/k8s) | — |
| Informer/controller event loop, healthz, metrics collectors | `controller/*`, `*/controller_test.go` | none | n/a | **scope-cut** (infra) | — |
| Metric-provider clients (Prom/Datadog/CW/NewRelic/Job/secret refs) | `metricproviders/*`, `analysis/analysis_test.go::TestSecret*` | type-level enum only, no live client | n/a | **scope-cut** (vendor clients) | — |
| CLI / kubectl-argo-rollouts | `pkg/kubectl-argo-rollouts/*` | none | n/a | **scope-cut** (UI/CLI) | — |
| CRD validation / dry-run / TTL GC / measurement retention | `analysis/analysis_test.go::Test{InvalidDryRun,ExceededTtl,TrimMeasurementHistory}` | none (no measurement store) | n/a | **scope-cut** (CRD/controller) | — |

## Recommended TDD fills (portable-coverage first)

Each item names the exact public cave fn the new test must exercise. No source changes
required — all targets are already implemented; only tests are missing.

1. `cave_rollouts::engine::advance_blue_green` — four cases: pre-promotion-analysis trigger, auto-promote→Promote/Healthy, analysis-fail→Abort/Aborted, manual→Pause/Paused. (Currently zero tests for this fn.)
2. `cave_rollouts::engine::initial_status` — Canary path (stable weight 100, canary 0, step index 0) and BlueGreen path (active/preview service names).
3. `cave_rollouts::engine::evaluate_metric` — string-condition entrypoint (e.g. failure_limit boundary), distinct from the already-tested `analysis::evaluate_metric`.
4. `cave_rollouts::engine::apply_canary_action` — the five untested arms: `Abort`, `Pause`, `Resume`, `Retry`, `Promote` (only `PromoteFull` is currently tested).
5. `cave_rollouts::engine::advance_canary` — max-weight clamp (`step weight > max_weight`), `SetMirrorWeight` step (canary_weight unchanged), and early-return guards for Aborted/Healthy/Paused phases.
6. `cave_rollouts::types::MetricCondition::evaluate` — direct boundary tests for `GreaterThan`, `LessThan`, `Between` (lo/hi inclusivity). Currently only exercised indirectly through `analysis::evaluate_metric`.
7. `cave_rollouts::types::CanaryStrategy::weight_at_step` — returns SetWeight value at index, `None` for Pause/Analysis/non-weight step or out-of-range index.
8. `cave_rollouts::types::RolloutPhase::is_terminal` — true for Healthy/Degraded/Error; false for Pending/Progressing/Paused.
9. `cave_rollouts::types::Experiment::total_weight` — sums `ExperimentVariant.weight` across variants (note: `types::Experiment`, distinct from the already-tested `experiment::Experiment`).

## Notes

- `models.rs`, `routes.rs`, `store.rs` are data/HTTP/SQL plumbing with no pure behavior to
  unit-test in isolation (router construction + SQL migration string + serde structs);
  route handlers are integration-test territory and are scope-cut here.
- `analysis::evaluate_metric` and `engine::evaluate_metric` are **two different functions**
  with the same name in different modules; only the `analysis` one is tested.
- `types::Experiment` and `experiment::Experiment` are likewise two distinct types; the
  `experiment` module one is well covered, the `types` one (`total_weight`, variants) is not.
