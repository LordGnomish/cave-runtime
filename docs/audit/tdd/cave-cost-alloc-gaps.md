# TDD coverage gap report — cave-cost-alloc

| Field | Value |
|-------|-------|
| Crate | `crates/ops/cave-cost-alloc` (theme: ops) |
| Upstream | OpenCost (`https://github.com/opencost/opencost`) |
| Upstream version | v1.108.0 (Go) |
| Upstream test files | 82 |
| Upstream test symbols | 342 |
| cave test fns | 4 (all generic proptest smoke — zero behavioral) |

## Context / honest framing

cave-cost-alloc is a small clean-room FinOps reimplementation, **not** a port of OpenCost's
internals. OpenCost's 342 test symbols are dominated by cloud-vendor billing integrations
(AWS Athena/S3, Azure billing export, GCP BigQuery, Alibaba BOA), the kubecost
Allocation/Asset accumulation algebra, Prometheus query plumbing, and generic Go utilities.
The vast majority of those are **scope-cut** for cave's launch (vendor SDK plumbing, infra
utilities, binary codecs, prom rate-limiters).

cave's **live** impl (modules declared in `lib.rs`: `allocator`, `models`, `reporting`,
`routes`) implements a focused subset: tag-based allocation, shared-cost splitting,
idle detection, anomaly detection, showback/chargeback, budget compliance, forecasting,
unit economics. **None of these have a single behavioral test** — that is the real,
portable gap and the priority of this report.

> Note: `cost_model.rs`, `recommendations.rs`, and `store.rs` exist on disk but are **NOT
> declared as `mod` in `lib.rs`** (verified) and reference a `CostAllocation` shape that does
> not match `models.rs`. They are dead/uncompiled code, so behaviors only present there are
> classified `missing-impl`, not portable-coverage.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|-----------------|---------------|-----------|-----------|----------|--------------------------|
| Shared-cost proportional split (CPU/mem/request weighting) | TestComputeIdleCoefficients (totals_test.go) | yes — `allocator::split_shared_costs` | no | portable-coverage | `test_split_shared_costs_proportional_by_cpu` |
| Equal split + zero-usage fallback to equal | TestAllocation_Share (allocation_test.go) | yes — `allocator::split_shared_costs` Equal / zero-total branch | no | portable-coverage | `test_split_shared_costs_equal_and_zero_usage_fallback` |
| Custom-weight split normalization | TestAllocationSet_AggregateBy_SharedCostBreakdown | yes — `allocator::split_shared_costs` ByCustomWeights | no | portable-coverage | `test_split_shared_costs_custom_weights_normalized` |
| Tag-based allocation to cost centers (team/project match, drop unmatched) | TestLabelConfig_GetExternalAllocationName (config_test.go) | yes — `allocator::allocate_costs` + `find_cost_center` | no | portable-coverage | `test_allocate_costs_matches_team_project_tags` |
| Idle/waste detection below threshold + monthly waste math | Test_getContainerAllocation / efficiency tests | yes — `allocator::calculate_idle_costs` | no | portable-coverage | `test_calculate_idle_costs_threshold_and_waste` |
| Idle recommendation tiering (terminate/downsize/review) | (n/a direct; heuristic) | yes — `allocator::idle_recommendation` | no | portable-coverage | `test_idle_recommendation_tiers_by_utilization` |
| Anomaly detection vs historical mean + severity tiers | TestScaleHourlyCostData (aggregation_test.go) | yes — `allocator::detect_anomalies` | no | portable-coverage | `test_detect_anomalies_deviation_and_severity` |
| Budget compliance over/warning/healthy status | TestComputeIdleCoefficients / budget thresholds | yes — `reporting::budget_compliance` | no | portable-coverage | `test_budget_compliance_status_thresholds` |
| Linear-regression forecast + confidence by sample count | TestScaleHourlyCostData (trend scaling) | yes — `reporting::forecast_spending` / `linear_regression` | no | portable-coverage | `test_forecast_spending_linear_trend_and_confidence` |
| Showback report aggregation + savings tips | TestSummaryAllocationSet_TotalEfficiency | yes — `reporting::generate_showback` / `identify_savings` | no | portable-coverage | `test_generate_showback_aggregates_and_tips` |
| Chargeback invoice line items + totals | TestAllocation_Add (cost summing) | yes — `reporting::generate_chargeback` | no | portable-coverage | `test_generate_chargeback_line_items_and_total` |
| Unit economics (cost/request, /user, /deployment, safe div) | TestFormatFloat64ForResponse | yes — `reporting::unit_economics` / `safe_div` | no | portable-coverage | `test_unit_economics_safe_division` |
| Allocation accumulation by window (hour/day/week/month) | TestAllocationSetRange_AccumulateBy_* (×7) | no — cave has no window-accumulation algebra | n/a | missing-impl | (defer — no impl) |
| Allocation set AggregateBy properties | TestAllocationSet_AggregateBy | no — cave aggregates only via showback/chargeback grouping | n/a | missing-impl | (defer — no impl) |
| Resource right-sizing recommendations (CPU/mem/spot/delete) | TestSummaryAllocationSet_RAMEfficiency/CPUEfficiency | impl only in ORPHAN `recommendations.rs` (not compiled) | n/a | missing-impl | (wire module first, then test) |
| AWS Athena/S3 cloud-cost retrieval | TestAthenaIntegration_GetCloudCost / TestS3Integration_GetCloudCost | no | n/a | scope-cut | vendor SDK billing integration — out of launch scope |
| Azure/GCP/Alibaba billing parsers + pricing | TestBigQueryIntegration_GetCloudCost, Test_NewBillingExportParser, provider_test.go | no | n/a | scope-cut | cloud-vendor-specific billing plumbing |
| Window parse/round/offset string algebra | TestParseWindowUTC, TestRoundBack, TestWindow_Expand | no | n/a | scope-cut | prom-query time-window utility, not in cave model |
| Binary codecs for Allocation/Asset | TestAllocation_BinaryEncoding (×16) | no | n/a | scope-cut | vendored gob/binary serialization infra |
| Prom rate-limited client / retry / worker pool | TestRateLimited*, TestWorkerPool*, retry_test.go | no | n/a | scope-cut | infra utilities (HTTP/concurrency), not domain |

## Recommended TDD fills (portable-coverage first)

These exercise **already-compiled** cave functions and require no new source. Write them as
`#[cfg(test)] mod tests` blocks in the respective module, or a `tests/behavior.rs` integration file.

1. `test_split_shared_costs_proportional_by_cpu` — `allocator::split_shared_costs` with
   `SplitStrategy::ByCpu`; assert each center gets `shared * usage/total`.
2. `test_split_shared_costs_equal_and_zero_usage_fallback` — Equal strategy splits evenly;
   ByCpu with total usage 0.0 falls back to equal shares; empty centers returns `[]`.
3. `test_split_shared_costs_custom_weights_normalized` — `ByCustomWeights`; assert weights
   normalize against `total_weight` and zero total returns `[]`.
4. `test_allocate_costs_matches_team_project_tags` — `allocator::allocate_costs`; resource
   tagged `team=X` maps to center X, untagged resource is dropped, `split_percentage == 100.0`.
5. `test_calculate_idle_costs_threshold_and_waste` — `allocator::calculate_idle_costs`;
   resource below threshold flagged, `wasted_cost_usd == hourly*24*30*(1-util/100)`, above-threshold excluded.
6. `test_idle_recommendation_tiers_by_utilization` — `allocator::idle_recommendation`;
   util<5 → "terminate", <20 → "downsize", else "review".
7. `test_detect_anomalies_deviation_and_severity` — `allocator::detect_anomalies`; <2 reports → `[]`,
   a report >threshold from mean flagged, severity buckets (>200 Critical, >100 High, >50 Medium).
8. `test_budget_compliance_status_thresholds` — `reporting::budget_compliance`; util>100 → Over,
   ≥alert_threshold → Warning, else Healthy; missing policy yields no entry.
9. `test_forecast_spending_linear_trend_and_confidence` — `reporting::forecast_spending`;
   monotonic-increasing history yields positive slope, `months_ahead` points, confidence 0.85 (≥6) / 0.65 (≥3) / 0.40.
10. `test_generate_showback_aggregates_and_tips` — `reporting::generate_showback`;
    `actual_cost_usd == showback_cost_usd == sum(reports)`, tip emitted when total >90% budget.
11. `test_generate_chargeback_line_items_and_total` — `reporting::generate_chargeback`;
    invoice `total_usd == sum(line_items.total_usd)`, status `Draft`.
12. `test_unit_economics_safe_division` — `reporting::unit_economics`; cost/request etc. correct,
    and zero denominators return 0.0 (no panic / no NaN).

### Deferred (not portable — need impl first)
- Right-sizing recommendations: `recommendations.rs` must be wired into `lib.rs` (and reconciled
  with the live `models.rs` `CostAllocation`) before it can be tested.
- Window-accumulation algebra and AllocationSet AggregateBy: no cave equivalent; out of current scope.
