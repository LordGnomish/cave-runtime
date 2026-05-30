# cave-cost — TDD coverage gap report

| field | value |
|-------|-------|
| crate | `crates/ops/cave-cost` |
| upstream | https://github.com/opencost/opencost |
| upstream version | v1.108.0 (Go) |
| upstream test symbols | 342 (across 82 test files) |
| cave test fns | 30 (`#[test]`, excluding 4 generic proptest_smoke invariants) |

## Scope note

OpenCost is a large Kubernetes cost-monitoring system whose test suite is dominated by
cloud-vendor billing integrations (AWS Athena/S3, GCP BigQuery, Azure price-sheet, Alibaba),
Prometheus query plumbing, time-window `AssetSet`/`AllocationSet` accumulation, binary codecs,
and a filter DSL. cave-cost is a deliberately scoped reimplementation covering the **core cost
math**: per-resource cost calculation, CPU/memory efficiency, allocation aggregation,
idle/shared-cost distribution, budgets + alerts, rightsizing/orphan recommendations,
showback/chargeback reports, trend forecasting, and provider pricing defaults. The bulk of
upstream's 342 symbols map to behaviors cave does not implement (cloud-vendor-specific,
Prometheus, vendored infra) and are correctly out of launch scope.

The gaps below focus on behaviors cave **already implements** but does **not yet test**
(portable-coverage) plus a couple of honest missing-impl items that have clear upstream analogues.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Aggregate allocations by non-namespace dimension (controller/pod/label/annotation) | `TestAllocationSet_AggregateBy` (allocation_test.go) | yes — `aggregate_costs` matches `AggregateBy::{Controller,Pod,Label,Annotation}` | only Namespace dim | portable-coverage | `test_aggregate_by_controller_and_label` |
| Efficiency rollup inside aggregation (non-idle / total, clamped 0..1) | `TestSummaryAllocationSet_TotalEfficiency` (summaryallocation_test.go) | yes — efficiency loop in `aggregate_costs` | no (only standalone `overall_efficiency` tested) | portable-coverage | `test_aggregate_computes_efficiency` |
| Showback / chargeback report by team label | `TestLabelConfig_GetExternalAllocationName` (config_test.go) | yes — `build_showback_report` groups by `team` label, `unallocated` fallback | no | portable-coverage | `test_build_showback_report_groups_by_team` |
| Cost calculation scales by window duration (hours) | `Test_getContainerAllocation` / `TestScaleHourlyCostData` (costmodel/aggregation_test.go) | yes — `calculate_resource_cost` multiplies by `hours` derived from window | only 1h window, cpu/mem asserted | portable-coverage | `test_calculate_cost_scales_with_window_hours` |
| Storage / network / GPU cost components | `Test_getContainerAllocation`, `TestBuildGPUCostMap` (cluster_helpers_test.go) | yes — `calculate_resource_cost` computes storage_cost, network_cost, gpu_cost | no (basic test uses 0 storage/net/gpu) | portable-coverage | `test_calculate_cost_storage_network_gpu` |
| Trend forecast points + projected monthly cost value | `TestProfileDataSeries_Series` (timeutil/profile_test.go) | yes — `generate_trend` emits 7 forecast points at avg-daily and `projected_monthly_cost = avg*30` | forecast count tested empty-case only; projected value untested for populated input | portable-coverage | `test_generate_trend_forecast_and_projection` |
| Report window bounds for week/month/custom | `TestParseWindowUTC` / `TestWindow_GetWindows` (window_test.go) | yes — `window_bounds` handles `LastWeek`/`LastMonth`/`Custom` | only `LastDay` | portable-coverage | `test_window_bounds_week_month_custom` |
| Rightsizing confidence tiers (0.9 vs 0.7) | `Test_getContainerAllocation` (costmodel_test.go) | yes — `rightsizing_recommendations` sets confidence by `LOW_UTILIZATION_THRESHOLD` | flag/no-flag tested, confidence value not asserted | portable-coverage | `test_rightsizing_confidence_high_when_very_low_util` |
| Merge / dedup recommendations | (no direct upstream symbol; analog to recommendation merge paths) | yes — `merge_recommendations` (currently concat only) | no | portable-coverage | `test_merge_recommendations_concatenates` |
| Budget percent-used / status thresholds (boundary at exactly threshold and 100%) | `TestComputeIdleCoefficients` (totals_test.go) analog for boundary math | yes — `evaluate_budget` boundary `>=` logic | over/under tested, exact-boundary not | portable-coverage | `test_budget_status_at_exact_threshold` |
| Idle-cost coefficient computation across allocations | `TestComputeIdleCoefficients` (kubecost/totals_test.go) | partial — cave has per-resource `calculate_idle_cost` but no set-wide idle coefficient distribution | n/a | missing-impl | (impl idle-coefficient first) |
| Time-window accumulate-by (None/Hour/Day/Week/Month) over allocation ranges | `TestAllocationSetRange_AccumulateBy_*` (allocation_test.go, 7 symbols) | no — cave has no AllocationSetRange / time-bucket accumulation | n/a | scope-cut | — (no time-series range model; out of launch scope) |
| Binary codec encode/decode round-trip for Allocation/Asset | `TestAllocation_BinaryEncoding` + 15 (kubecost_codecs_test.go) | no — cave uses serde JSON over HTTP, no custom binary codec | n/a | scope-cut | — (no binary wire format) |
| Cloud-vendor billing retrieval (AWS/GCP/Azure/Alibaba) | `TestAthenaIntegration_GetCloudCost`, `TestBigQueryIntegration_GetCloudCost`, etc. | no | n/a | scope-cut | — (cloud-vendor-specific; out of scope) |
| Filter DSL parse/match (filter21 lexer/parser/matcher) | `TestParse`, `TestLexer`, `TestCompileAndMatch` (filter21/*) | no — cave aggregates via enum dims, no query filter language | n/a | scope-cut | — (no filter DSL) |

## Recommended TDD fills (portable-coverage first)

These exercise behavior cave **already ships** but does not test. Each is cheap and high-value.

1. **`test_aggregate_by_controller_and_label`** — `allocation.rs::aggregate_costs`. Feed
   `ResourceCost`s with distinct `controller` / `labels`, aggregate by `AggregateBy::Controller`
   then `AggregateBy::Label`, assert grouping keys + summed `total_cost`. Today only the
   `Namespace` branch of the `match by` is covered.

2. **`test_aggregate_computes_efficiency`** — `allocation.rs::aggregate_costs`. Set non-zero
   `idle_cost` on inputs and assert the post-loop `efficiency = (total-idle)/total` is clamped to
   `0.0..=1.0`. The efficiency loop is currently unexercised.

3. **`test_build_showback_report_groups_by_team`** — `reports.rs::build_showback_report`. Provide
   allocations with and without a `team` label; assert line items group by team, the missing-team
   item falls back to `"unallocated"`, and `total_cost` sums line items. Function has zero tests.

4. **`test_calculate_cost_scales_with_window_hours`** — `calculator.rs::calculate_resource_cost`.
   Same inputs over a 1h vs 2h window; assert `cpu_cost`/`memory_cost` roughly double. The `hours`
   scaling factor is untested.

5. **`test_calculate_cost_storage_network_gpu`** — `calculator.rs::calculate_resource_cost`.
   Non-zero `storage_bytes`, `network_egress_bytes`, `gpu_cores`; assert each component cost is
   positive and `total_cost` includes them. `test_calculate_cost_basic` passes 0 for all three.

6. **`test_generate_trend_forecast_and_projection`** — `reports.rs::generate_trend`. Multi-day
   input; assert `forecast_points` length is 7, each equals avg-daily, and
   `projected_monthly_cost == avg_daily * 30.0`. The populated projection value is currently
   asserted only for the empty case (0.0).

7. **`test_window_bounds_week_month_custom`** — `reports.rs::window_bounds`. Assert `LastWeek`
   spans ~7 days, `LastMonth` ~30 days, and `Custom` honours explicit start/end (and falls back to
   7 days when `None`). Only `LastDay` is tested.

8. **`test_rightsizing_confidence_high_when_very_low_util`** — `recommendations.rs::rightsizing_recommendations`.
   CPU util below `LOW_UTILIZATION_THRESHOLD` (0.25) → assert `confidence == 0.9`; util between
   0.25 and 0.50 → `0.7`. The confidence tiering branch is untested.

9. **`test_merge_recommendations_concatenates`** — `recommendations.rs::merge_recommendations`.
   Merge two vecs, assert combined length. (Note: the doc-comment claims highest-confidence dedup
   but the impl only concatenates — the test will document actual behavior and flag the doc drift.)

10. **`test_budget_status_at_exact_threshold`** — `budget.rs::evaluate_budget`. spend == threshold%
    → `Warning`; spend == 100% → `Exceeded`. Verifies the inclusive `>=` boundary logic that the
    over/under tests skip.

### Honest non-portable notes
- **Idle-coefficient distribution** (`missing-impl`): upstream `TestComputeIdleCoefficients`
  distributes idle cost across allocations proportionally; cave only computes per-resource
  `allocated - used`. Would require new impl — not a free test fill.
- **AccumulateBy time ranges, binary codecs, cloud-vendor billing, filter DSL** (`scope-cut`):
  no corresponding cave model. Correctly out of launch scope; not gaps to fill.
