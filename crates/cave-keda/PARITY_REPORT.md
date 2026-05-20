<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-keda — Charter v2 Parity Report

**Upstream:** [kedacore/keda](https://github.com/kedacore/keda) pinned **v2.16.1**.
**Upstream license:** Apache-2.0 (Copyright 2024 The KEDA Authors).
**cave-keda license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.

---

## 1 · Fill-ratio (honest, measured)

```
impl_lines              = 889    (cave-keda src/, excl #[cfg(test)] + blanks + comments)
upstream_in_scope_lines = 1 360  (sum of per-subsystem in-scope LOC)
fill_ratio              = 0.6537
honest_ratio            = 0.6537 (no [[partial]] entries; honest == fill)
parity_ratio_source     = "manifest"
```

`docs/parity/parity-index.json` reads these fields directly from
`parity.manifest.toml` — no external audit doc drift.

## 2 · Per-subsystem LOC table

| Upstream file                                              | upstream LOC | in-scope LOC | local file                | status |
|------------------------------------------------------------|-------------:|-------------:|---------------------------|--------|
| `pkg/apis/keda/v1alpha1/scaledobject_types.go`             | 220          | 200          | `src/scaledobject.rs`     | mapped |
| `pkg/apis/keda/v1alpha1/scaledjob_types.go`                | 175          | 150          | `src/scaledjob.rs`        | mapped |
| `pkg/apis/keda/v1alpha1/triggerauthentication_types.go`    |  65          |  60          | `src/trigger_authentication.rs` | mapped |
| `pkg/scaling/scaledobject_controller.go` (cooldown subset) | 600          | 200          | `src/scaledobject.rs`     | mapped |
| `pkg/scaling/scaledjob_controller.go` (strategy subset)    | 400          | 150          | `src/scaledjob.rs`        | mapped |
| `pkg/scalers/scaler.go`                                    |  80          |  80          | `src/scaler.rs`           | mapped |
| `pkg/scalers/cpu_memory_scaler.go`                         | 150          | 100          | `src/cpu_memory_scaler.rs`| mapped |
| `pkg/scalers/cron_scaler.go`                               | 200          | 120          | `src/cron_scaler.rs`      | mapped |
| `pkg/scalers/kafka_scaler.go` (lag-only)                   | 600          |  80          | `src/kafka_scaler.rs`     | mapped |
| `pkg/scalers/prometheus_scaler.go` (value-only)            | 250          |  60          | `src/prometheus_scaler.rs`| mapped |
| `pkg/scalers/redis_scaler.go` (list/stream length)         | 350          |  80          | `src/redis_scaler.rs`     | mapped |
| `pkg/scalers/external_scaler.go` (HTTP add-on)             | 200          |  80          | `src/http_scaler.rs`      | mapped |
| **Total**                                                  | **3 290**    | **1 360**    |                           |        |

## 3 · Mapped subsystems (12)

1. **scaledobject-crd** — `ScaledObject` struct with min/max/idle/cooldown/pause/triggers parity vs `scaledobject_types.go`.
2. **scaledjob-crd** — `ScaledJob` with `Default`/`Custom`/`Accurate` `ScalingStrategy` enum + `successful_jobs_history_limit` + `failed_jobs_history_limit`.
3. **triggerauth-crd** — `SecretTargetRef` + `EnvTargetRef` + inline-override fallback resolution.
4. **scaledobject-controller** — `ScaledObject::reconcile` mirrors KEDA's cooldown-aware active/inactive transitions; `scale_to_zero` follows idle→min→0 precedence.
5. **scaledjob-controller** — `ScaledJob::jobs_to_spawn` dispatches per strategy: Default caps at `max_replica_count`, Custom subtracts `running_jobs`, Accurate floors at zero.
6. **scaler-trait** — `Scaler` + `ScalerTrait` + `replicas_from_metric` ceiling math, zero-target safe.
7. **cpu-memory-scaler** — `CpuScaler` and `MemoryScaler` with `ResourceMetricType` (Utilization / AverageValue), default 80% CPU.
8. **cron-scaler** — `CronScaler` schedule-based active/inactive + built-in `validate_cron` (5-field, 0–59/0–23/1–31/1–12/0–6 range checks).
9. **kafka-scaler** — `KafkaScaler::record_lag` + `total_lag` + `recommended_replicas` with per-partition capping.
10. **prometheus-scaler** — `PrometheusScaler::observe` with NaN guard + activation threshold gating.
11. **redis-scaler** — `RedisScaler` with `RedisDataType::List` / `Stream` and activation threshold.
12. **http-scaler** — `HttpScaler::observe` clamps negatives; `metric_value` returns pending-request count.

## 4 · Skipped subsystems (12 — out-of-MVP)

| Surface                       | Reason for deferral                                                                 |
|-------------------------------|-------------------------------------------------------------------------------------|
| AWS scalers                   | CloudWatch/SQS/Kinesis/DynamoDB — autoscale-cloud Phase 2 alongside cave-ccm AWS.   |
| Azure scalers                 | Monitor/ServiceBus/EventHub/Blob — autoscale-cloud Phase 2.                          |
| GCP scalers                   | Pub/Sub + Stackdriver — autoscale-cloud Phase 2.                                     |
| DataDog scaler                | Observability vendor — deferred.                                                     |
| New Relic scaler              | Observability vendor — deferred.                                                     |
| Splunk scaler                 | Observability vendor — deferred.                                                     |
| Dynatrace scaler              | Observability vendor — deferred.                                                     |
| WASM scaler runtime           | Phase 2 — requires wasmtime integration.                                             |
| ScalingModifiers CEL          | Phase 2 — reuse cave-apiserver CEL engine.                                           |
| HPA-direct integration        | cave-controller-manager owns HPA path; we expose replica recommendations directly.   |
| CloudEvents source scaler     | Phase 2 — gated on cave-knative eventing surface.                                    |
| Selenium Grid scaler          | Niche QA workload — deferred.                                                        |

## 5 · 4-track status

| Track          | Status     | Evidence                                                                        |
|----------------|------------|---------------------------------------------------------------------------------|
| Backend        | **GREEN**  | This crate (cave-keda) — 12 mapped surfaces, 26 lib tests + 9 parity_self_audit. |
| Portal         | GREEN      | cave-portal `/admin/keda/` (since 2026-05-12, see portal-keda-real-ui memory).  |
| cavectl        | GREEN      | `cavectl keda` 6 subcommands (since 2026-05-10, portal-keda-3pkg memory).       |
| Observability  | GREEN      | KEDA alert pack (3 alerts) + 4 dashboard panels (since 2026-05-10).             |

## 6 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                  | Status |
|---|-----------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS           | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file in this crate              | ✅      |
| 3 | `[upstream] source_sha` pinned to `v2.16.1`                           | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in src/  | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims              | ✅      |
| 6 | Always-latest — KEDA v2.16.1 (latest stable as of 2026-05-19)         | ✅      |
| 7 | 4-track — Backend + Portal + cavectl + Observ. all GREEN (above)      | ✅      |
| 8 | Honest measured `fill_ratio = 0.6537` (>= 0.55 MVP floor)             | ✅      |

## 7 · Reproducibility

```bash
# Verify fill_ratio derivation
cargo test -p cave-keda --test parity_self_audit

# Workspace-wide parity-index regeneration
python3 scripts/build-parity-index.py
```

The on-disk `parity.manifest.toml` is canonical. `docs/parity/parity-index.json`
mirrors `fill_ratio` and the count fields via `scripts/build-parity-index.py`.
