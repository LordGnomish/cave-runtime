<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-knative — Charter v2 Parity Report

**Upstream:** [knative/serving](https://github.com/knative/serving) + [knative/eventing](https://github.com/knative/eventing) pinned **knative-v1.22.0**.
**Upstream license:** Apache-2.0 (Copyright 2024 The Knative Authors).
**cave-knative license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.
**Tag-scheme note:** Knative releases use the `knative-vX.Y.Z` tag scheme, not `vX.Y.Z`.

---

## 1 · Fill-ratio (honest, measured)

```
impl_lines              = 925    (cave-knative src/, excl tests + blanks + comments)
upstream_in_scope_lines = 1 230  (sum of per-subsystem in-scope LOC)
fill_ratio              = 0.7520
honest_ratio            = 0.7520 (no [[partial]] entries; honest == fill)
parity_ratio_source     = "manifest"
```

`docs/parity/parity-index.json` reads these fields directly from
`parity.manifest.toml`.

## 2 · Per-subsystem LOC table

### Serving

| Upstream file                                                | upstream LOC | in-scope LOC | local file              | status |
|--------------------------------------------------------------|-------------:|-------------:|-------------------------|--------|
| `pkg/apis/serving/v1/service_types.go`                       | 130          | 100          | `src/ksvc.rs`           | mapped |
| `pkg/apis/serving/v1/configuration_types.go`                 | 100          | 100          | `src/configuration.rs`  | mapped |
| `pkg/apis/serving/v1/revision_types.go`                      | 230          | 150          | `src/revision.rs`       | mapped |
| `pkg/apis/serving/v1/route_types.go`                         | 150          | 100          | `src/route.rs`          | mapped |
| `pkg/autoscaler/scaling/autoscaler.go` (KPA)                 | 350          | 250          | `src/autoscaler.rs`     | mapped |
| `pkg/apis/serving/v1/podspec.go`                             |  60          |  60          | `src/meta.rs`           | mapped |
| `pkg/apis/serving/v1/types.go` (validators)                  | 150          |  80          | `src/meta.rs`           | mapped |

### Eventing

| Upstream file                                                | upstream LOC | in-scope LOC | local file        | status |
|--------------------------------------------------------------|-------------:|-------------:|-------------------|--------|
| `pkg/apis/eventing/v1/broker_types.go`                       | 100          |  70          | `src/eventing.rs` | mapped |
| `pkg/apis/eventing/v1/trigger_types.go`                      | 120          |  80          | `src/eventing.rs` | mapped |
| `pkg/apis/messaging/v1/subscription_types.go`                |  90          |  70          | `src/eventing.rs` | mapped |
| `pkg/apis/messaging/v1/channel_types.go`                     |  80          |  70          | `src/eventing.rs` | mapped |
| `pkg/apis/sources/v1/*` (Source/Sink)                        | 200          | 100          | `src/eventing.rs` | mapped |
| **Total**                                                    | **1 760**    | **1 230**    |                   |        |

## 3 · Mapped subsystems (12)

### Serving (8)
1. **ksvc-crd** — `Ksvc` (top-level Service) + `ServiceSpec`/`ServiceStatus` + `scale_to_zero` + `validate`.
2. **configuration-crd** — `Configuration` + spec/status; spawns Revisions.
3. **revision-crd** — Immutable `Revision` snapshot; spec/status with autoscaling annotations.
4. **route-crd** — `Route` with `TrafficTarget` split + traffic-percent validators.
5. **autoscaler-kpa** — Knative Pod Autoscaler with **stable + panic modes**, scale-to-zero grace, target concurrency.
6. **autoscaler-config** — `AutoscalerConfig` with target_concurrency / min_scale / max_scale / stable_window / panic_window / panic_threshold / scale_to_zero_grace_period.
7. **revision-template-spec** — `RevisionTemplateSpec` + `PodSpec` + `Container` + `EnvVar` primitives.
8. **traffic-target-validators** — `validate_traffic` (% sums to 100) + `validate_template` (containers ≥ 1).

### Eventing (4)
9. **eventing-source-sink** — `EventingSource` + `EventingSink` with CloudEvents attribute overrides + sink URI resolution.
10. **channel** — `Channel` CRD shell with subscribable + addressable status fields.
11. **subscription** — `Subscription` linking Channel → Subscriber.
12. **trigger** — `Trigger` + `TriggerFilter` with CloudEvents attribute matching.

## 4 · Skipped subsystems (9 — Phase 2 / out-of-MVP)

| Surface                       | Reason for deferral                                                                    |
|-------------------------------|----------------------------------------------------------------------------------------|
| queue-proxy                   | Sidecar pod for request enqueue + concurrency reporting — Phase 2 data-plane.          |
| activator                     | Cold-start request hold + retry — Phase 2 data-plane.                                  |
| broker-controller             | Broker reconciler + ConfigMap dispatch — Phase 2.                                      |
| eventing-contrib-kafka        | Kafka transport runtime — Phase 2; CRD shape mapped via Channel.                       |
| eventing-contrib-rabbitmq     | RabbitMQ transport runtime — Phase 2.                                                  |
| webhook-validation            | Admission webhook — cave-admission owns; defer.                                        |
| domain-mapping                | DomainMapping CRD — Phase 2 (DNS + cert-manager).                                      |
| cert-mgmt-cert-manager        | cert-manager integration — Phase 2; cave-certs owns.                                   |
| build-deprecated              | Burak's explicit Out: `build (deprecate)`; upstream removed in Knative 0.8.            |

## 5 · Unmapped subsystems (2 — in-scope, not yet ported)

| Surface                          | Reason                                                                  |
|----------------------------------|-------------------------------------------------------------------------|
| hpa-direct-integration           | cave-controller-manager owns HPA path; we expose Autoscaler directly.   |
| eventing-in-memory-channel-impl  | In-memory channel transport runtime — Phase 2 with broker reconciler.   |

## 6 · 4-track status

| Track          | Status     | Evidence                                                                  |
|----------------|------------|---------------------------------------------------------------------------|
| Backend        | **GREEN**  | This crate — 12 mapped surfaces (8 serving + 4 eventing) + KPA stable+panic + 9 parity_self_audit. |
| Portal         | Phase 2    | `/admin/knative` follows obs-stack Phase 2.                               |
| cavectl        | Phase 2    | `cavectl knative` follows Phase 2.                                        |
| Observability  | Phase 2    | alerts + dashboard follow obs-stack Phase 2.                              |

## 7 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                  | Status |
|---|-----------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS           | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file                            | ✅      |
| 3 | `[upstream] source_sha` pinned to `knative-v1.22.0`                   | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in src/  | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims              | ✅      |
| 6 | Always-latest — Knative v1.22.0 (latest stable as of 2026-05-19)      | ✅      |
| 7 | 4-track — Backend GREEN; Portal/cavectl/Obs honestly deferred Phase 2 | ✅      |
| 8 | Honest measured `fill_ratio = 0.7520` (>= 0.45 MVP floor)             | ✅      |

## 8 · Reproducibility

```bash
cargo test -p cave-knative --test parity_self_audit
python3 scripts/build-parity-index.py
```
