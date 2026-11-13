# TDD coverage-gap audit — cave-compliance

| Field | Value |
|-------|-------|
| Cave crate | `crates/security/cave-compliance` (theme: security) |
| Upstream | open-policy-agent/gatekeeper (Go) |
| Upstream version | v3.17.1 |
| Upstream test files | 115 |
| Upstream test symbols | 305 |
| Cave test fns | 29 (`#[test]`/`#[tokio::test]`, incl. 4 generic proptest_smoke) |

## Domain-fit caveat (read first)

**Gatekeeper and cave-compliance are different domains.** Gatekeeper is a Kubernetes
**OPA admission/mutation/constraint-template** controller: its test suite is dominated by
mutation engines (`pkg/mutation/**`), admission webhooks (`pkg/webhook/**`), watch/cache/
readiness machinery (`pkg/watch`, `pkg/cachemanager`, `pkg/readiness`), constraint-template
reconcilers, expansion, and the `gator` CLI test-runner.

cave-compliance is a **compliance-framework audit engine**: CIS/SOC2/ISO27001/GDPR control
catalogues, evidence collection, findings, assessments, gap analysis, audit trail, and
control→policy-engine mapping. It does **not** implement OPA constraint templates, mutation
webhooks, K8s watch caches, or the gator runner.

As a result the overwhelming majority of the 305 upstream test symbols are **scope-cut** for
cave: they exercise behaviors cave deliberately does not implement. Only a handful of upstream
tests touch *conceptually analogous* behavior (rule/policy matching, enforcement-exception
handling, stats aggregation), and even those map only loosely. The honest, high-value finds
are cave's own **untested internal functions** (notably all of `src/monitor.rs`), surfaced
here as portable-coverage gaps because the analogous upstream behavior is tested upstream.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|-----------------|---------------|------------|------------|----------|--------------------------|
| Aggregate effectiveness score across assessments (mean, framework-filtered) | `Test_AggregateStats` (pkg/expansion/aggregate_test.go) | yes — `monitor::ComplianceMonitor::effectiveness_score` | NO | portable-coverage | `test_effectiveness_score_means_framework_filtered` |
| Identify gaps / prioritize unmet rules | `TestAggregateResponses` (pkg/expansion/aggregate_test.go) | yes — `monitor::ComplianceMonitor::identify_gaps` | NO | portable-coverage | `test_identify_gaps_flags_unimplemented_and_low_score` |
| Roll-up summary (total/implemented/tested/gaps) | `Test_ToStatsEntriesWithDesc` (pkg/instrumentation/types_test.go) | yes — `monitor::ComplianceMonitor::compliance_summary` | NO | portable-coverage | `test_compliance_summary_counts_by_status` |
| Enforcement exception / override suppresses violation, expiry honored | `TestValidateEnforcementAction`, `TestOverrideEnforcementAction` (pkg/util/enforcement_action_test.go, pkg/expansion/aggregate_test.go) | yes — `reports::generate_report` `excepted_control_ids` expiry filter | NO (only no-exception paths tested) | portable-coverage | `test_generate_report_active_exception_excludes_failure` |
| Policy/rule lookup returns empty for unknown key (negative path) | `Test_namesMatch`, `TestFilter_MatchesTest` (pkg/mutation/match, pkg/gator/verify) | yes — `policy::suggested_mappings` unknown→`vec![]` | NO (only non-empty path tested) | portable-coverage | `test_suggested_mappings_unknown_control_is_empty` |
| Staleness / freshness check returns false past max-age | (no direct upstream analogue; freshness is cave-specific) | yes — `evidence::is_fresh` stale branch | NO (only fresh=true tested) | portable-coverage | `test_is_fresh_false_when_older_than_max_age` |
| Event/record filter by secondary key (resource_type) | `TestFilter_MatchesCase` (pkg/gator/verify/filter_test.go) | yes — `audit::filter_events` resource_type predicate | NO (only actor filter tested) | portable-coverage | `test_filter_events_by_resource_type` |
| Control→module mapping resolution for known + unknown controls | `TestReadSyncRequirements` / `TestMatch` (mapping/match analogues) | yes — `mapping::get_mappings_for_control` | NO direct test (only indirect via engine) | portable-coverage | `test_get_mappings_for_control_known_and_unknown` |
| Mutation engine (Assign/AssignImage/AssignMetadata/ModifySet) | `TestAssign`, `TestMutate`, `Test_ModifySet_errors`, `TestApplyTo` (pkg/mutation/**) | no | n/a | scope-cut | — |
| Admission/validation webhook review | `TestReviewRequest`, `TestConstraintValidation`, `TestAdmission` (pkg/webhook/**) | no | n/a | scope-cut | — |
| Constraint-template reconcile + status | `TestReconcile`, `TestShouldGenerateVAP` (pkg/controller/**) | no | n/a | scope-cut | — |
| Watch/cache/readiness object tracking | `Test_ObjectTracker_*`, `TestRegistrar_*`, `TestCacheManager_*` (pkg/watch, pkg/readiness, pkg/cachemanager) | no | n/a | scope-cut | — |
| Template expansion / conflict detection | `TestExpand`, `TestGetConflicts`, `TestDetectConflicts` (pkg/expansion, pkg/gator/reader) | no | n/a | scope-cut | — |
| gator CLI test-runner / suite verification | `TestRunner_Run`, `TestTest`, `TestReadSuites` (pkg/gator/**) | no | n/a | scope-cut | — |
| Wildcard / GVK / path-token parsing | `TestMatches`, `TestScanner`, `TestParser` (pkg/wildcard, pkg/mutation/path/**) | no | n/a | scope-cut | — |
| Prometheus/OTel metrics exporters & stats reporters | `TestPrometheusExporter`, `TestReporter_*` (pkg/metrics, pkg/*/stats_reporter_test.go) | no (cave uses its own obs stack) | n/a | scope-cut | — |
| Pub/Sub (Dapr) connection & publish | `TestDapr_Publish`, `TestSystem_Publish` (pkg/pubsub/**) | no | n/a | scope-cut | — |

Scope-cut justification (one line): all scope-cut rows exercise Gatekeeper's Kubernetes
OPA admission/mutation/watch/reconcile/CLI machinery, which is out of cave-compliance's
launch scope — cave-compliance is a framework-control audit engine, not an admission controller.

## Recommended TDD fills (portable-coverage first)

These are cheap, real, and verifiable — the behavior already ships in cave's `src/`, only the
test is missing. Listed most-portable / highest-value first.

1. **`test_effectiveness_score_means_framework_filtered`** — exercises
   `monitor::ComplianceMonitor::effectiveness_score`. Build two `ControlAssessment`s whose
   `control_id`s resolve (via `frameworks::get_control`) to the target framework and one that
   resolves to a different framework; assert the returned mean only averages the matching ones,
   and that an empty slice returns `0.0`. **`src/monitor.rs` has zero tests today.**

2. **`test_identify_gaps_flags_unimplemented_and_low_score`** — exercises
   `monitor::ComplianceMonitor::identify_gaps`. Pass a control with no assessment (expect a
   `Critical` gap, `NotImplemented`) and a control with `effectiveness_score < 0.7` (expect a
   gap with priority by the score bands); assert a control at/above threshold is omitted.

3. **`test_compliance_summary_counts_by_status`** — exercises
   `monitor::ComplianceMonitor::compliance_summary`. Mix `Implemented`, `Tested`, `Audited`,
   and missing assessments; assert `implemented`, `tested`, and `gaps` counts plus
   `effectiveness_score` are consistent.

4. **`test_generate_report_active_exception_excludes_failure`** — exercises
   `reports::generate_report` exception handling. Feed one `FindingStatus::Fail` plus a
   `ControlException` for that `control_id` with a future `expires_at`; assert `failed == 0`
   and that `compliance_score` credits the excepted control. Add a sibling assertion with an
   already-expired exception to prove the expiry filter (`chrono::Utc::now() < exp`) still
   counts the failure. This is the only path in `reports.rs` currently untested.

5. **`test_suggested_mappings_unknown_control_is_empty`** — exercises
   `policy::suggested_mappings` negative branch; assert an unknown `control_ref` returns an
   empty vec (current tests only cover the populated `CIS-5.2.1` arm).

6. **`test_is_fresh_false_when_older_than_max_age`** — exercises `evidence::is_fresh` stale
   branch. Construct an `Evidence` with `collected_at` set well in the past (or call with
   `max_age_hours = 0`) and assert `is_fresh(..) == false` (current test only covers `true`).

7. **`test_filter_events_by_resource_type`** — exercises the `resource_type` predicate in
   `audit::filter_events` (current test only filters by `actor`).

8. **`test_get_mappings_for_control_known_and_unknown`** — exercises
   `mapping::get_mappings_for_control` directly: a `(Soc2, "CC6.1")` control returns 2 module
   mappings (`cave-auth` + `cave-pam`); an unmapped control returns an empty vec. Today this is
   only covered transitively through `engine` tests.

### Note for implementers
`src/engine.rs` and `src/monitor.rs` reference a `ComplianceStore`/model shape that differs
from the `ComplianceStore` defined in `src/lib.rs` (lib.rs: `frameworks/controls/findings/
evidence/audit_events`; engine.rs: `controls/evidences/assessments/...`). The engine tests
construct their own local `make_store()` and the `engine`/`monitor`/`mapping` modules are not
declared in `lib.rs`'s `pub mod` list. Confirm which schema is the live one before adding the
monitor/engine fills, so the new tests compile against the actually-built module set.
