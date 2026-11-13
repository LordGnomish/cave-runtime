# cave-slo TDD coverage audit

- **Cave crate:** `cave-slo` (theme: observability)
- **Upstream:** [nobl9/nobl9-go](https://github.com/nobl9/nobl9-go) @ `v0.126.1`
- **Upstream test inventory:** 132 test files, 407 test/example symbols
- **Cave test functions:** 88 (`#[test]` in `src/engine.rs` + `src/models.rs`)
- **Date:** 2026-05-30

## Scope framing (honest)

`nobl9-go` is the **Go SDK / API client for the Nobl9 SaaS**. Its 407 test symbols are
overwhelmingly:

- `manifest/v1alpha/*` (89 test files) — CRD-style **manifest validation** for `SLO`,
  `Agent`, `AlertMethod`, `AlertPolicy`, `AlertSilence`, `Annotation`, `Project`, `RoleBinding`,
  `Service`, `DataExport`, `BudgetAdjustment`, plus **~40 metric-source provider specs**
  (Prometheus, Datadog, NewRelic, CloudWatch, Dynatrace, Splunk, BigQuery, Lightstep, …).
- `sdk/`, `sdk/endpoints/` (16 files) — HTTP request builders, auth, filters, paging against
  the Nobl9 REST API.
- `tests/` (22 files) — live integration tests against a running Nobl9 backend.

`cave-slo` is a **small computational backend**, not a re-implementation of the Nobl9 manifest
schema or SaaS client. It implements: error-budget math, burn-rate, multi-window Google-SRE
evaluation, composite SLO, an in-memory CRUD store, and a thin axum route layer.

Therefore the vast majority of upstream tests are **scope-cut** (manifest schema / provider
validation / SaaS API plumbing not modeled by cave). The genuine behavioral overlap is the
**error-budget + burn-rate + composite + objective-window arithmetic** and **status
aggregation** — and most of that is already tested. The few gaps below are public cave
functions that are implemented and exercised by route code but carry **no direct unit test**.

## Classification summary

| Class | Count | Notes |
|-------|------:|-------|
| **portable-coverage** (cave implements, no direct test — PRIORITY) | 6 | listed below |
| missing-impl (upstream behavior cave does not implement) | 0 | composite/budget concepts cave *does* model at engine level; cave intentionally does not model the manifest schema |
| scope-cut (manifest/CRD validation, ~40 metric providers, SDK/REST, live integration) | ~400 | `manifest/v1alpha/agent`, `…/alertmethod`, `…/slo/metrics_*`, `sdk/*`, `tests/*`, `internal/*` |

## Portable-coverage gaps (cave implements it, verified in source, no direct test)

| # | Upstream behavioral unit (nearest) | Cave public fn (file) | Why portable |
|---|-----------------------------------|-----------------------|--------------|
| 1 | `slo` two-window burn-rate alerting (Google-SRE multi-window; upstream models alerting windows in `alertpolicy` validation) | `evaluate_multi_window` (`src/engine.rs:143`) | Pure arithmetic over four `SloIndicator` windows → burn rates + `short_window_alert`/`long_window_alert` + derived status. No test exercises the 14.4×/6× dual-window firing logic. |
| 2 | `slo` composite SLO / `TestSpec_HasCompositeObjectives`, `TestValidate_CompositeSLO` (objective-weighted compliance) | `composite_slo_compliance` (`src/engine.rs:183`) | Pure weighted-average over `&[SloObjective]` + `&[f64]`; covers empty-list and zero-total-weight edge branches. Untested. |
| 3 | `slo` objective error-budget (`ExampleReport_errorBudgetStatus`, objective-level budgeting) | `SloObjective::allowed_bad_minutes` / `window_minutes` (`src/models.rs:87`, `:93`) | Per-objective budget-minute math (`window_days × 1440 × (1 − target/100)`). Untested. |
| 4 | SLI indicator → error-rate (Ratio/Threshold/Latency raw-metric handling, `…/slo/metrics_*`, `Objectives_RawMetric`) | `SloIndicator::error_rate_pct` (`src/models.rs:54`) | Pure conversion of the three indicator variants to error-rate %, incl. `total==0` guard. Only indirectly hit via `burn_rate_from_indicator`; the `Latency`/`Threshold` arms are untested. |
| 5 | SLO status reporting / aggregation (`tests/slostatusapi_test.go` GetSLO/GetSLOs) | `SloStore::compute_stats` + `list_by_status` (`src/store.rs:66`, `:55`) | Counts SLOs by `SloStatus` + mean `current_sli`; status-filtered listing. Mirrors the SLO-status API aggregation. Untested. |
| 6 | `Metadata.Annotations` map (`TestValidate_Metadata_Annotations` across agent/alert*/annotation) | `SloAnnotations::set`/`get`/`remove` (`src/models.rs:159`–`:169`) | Key/value annotation bag set/get/remove + missing-key `None`. Mirrors upstream metadata annotations. Untested. |

## Recommended TDD fills (portable-coverage first)

1. **`evaluate_multi_window`** — build an `SLO` at 99.9% and feed four `SloIndicator::Ratio`
   windows: one with a high 1h error rate (≥14.4× burn) and a 6h rate ≥6×; assert both
   `short_window_alert` and `long_window_alert` are `true` and `status == SloStatus::Breached`.
   Add a benign case (all windows healthy) asserting both alerts `false` and `status == Ok`.
2. **`composite_slo_compliance`** — assert weighted average of two objectives (e.g. weights
   0.7/0.3 over SLIs 99.0/95.0 → 97.8); assert `0.0` for empty objective slice; assert `0.0`
   when all weights are `0.0`.
3. **`SloObjective::allowed_bad_minutes` / `window_minutes`** — for `{target: 99.9, window_days: 30}`
   assert `window_minutes() == 43200.0` and `allowed_bad_minutes() == 43.2` (within 1e-6).
4. **`SloIndicator::error_rate_pct`** — cover the under-tested arms:
   `Latency { p99_ms: 200, threshold_ms: 150 } → 100.0`, `Latency { 100, 150 } → 0.0`,
   `Threshold { value: 5, threshold: 10 } → 0.0`, `Threshold { 10, 10 } → 100.0`,
   `Ratio { good: 0, total: 0 } → 0.0`.
5. **`SloStore::compute_stats` + `list_by_status`** — insert SLOs across all statuses, assert
   `SloStats { total, ok, at_risk, breaching, breached, avg_compliance }` counts and the mean,
   and that `list_by_status(SloStatus::Breached)` returns only breached SLOs.
6. **`SloAnnotations`** — `set` then `get` returns the value; `remove` then `get` returns
   `None`; `get` of an absent key returns `None`.
