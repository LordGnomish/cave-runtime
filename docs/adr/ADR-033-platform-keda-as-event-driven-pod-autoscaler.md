# ADR-033: Platform KEDA as Event-Driven Pod Autoscaler

**Status:** Accepted

**Scope:** Universal (Platform; Runtime sovereign reimpl `cave-keda` covered by the blanket ADR-RUNTIME-UPSTREAM-MIRROR-001 — no separate Runtime override ADR)

**Category:** Platform / Workload Autoscaling

**Date:** 2026-04-26

**Related ADRs:** 021 (Streams Kafka/Pulsar), 032 (Karpenter), 038 (Argo Workflows), 040 (ARC Runner Scaling), 075 (Knative + KEDA Phase 4), 095 (Reflex Engine), 115 (OIDC Identity Federation)

## Context

Cave Platform must scale workloads on **events**, not just CPU/memory:

- Kafka consumer groups must scale on partition lag.
- Redis Streams / List consumers must scale on stream depth.
- HTTP queue workers must scale on queue length, not request rate.
- Reflex Engine (ADR-095) reacts to Prometheus alerts and Kafka topics — replicas must spin from `0 → N` within seconds.
- Knative serving (ADR-075) needs scale-to-zero across non-HTTP triggers (Kafka, Pulsar, NATS).
- Cron-driven batch jobs must materialize replicas at `T-0` and drop to zero immediately after.
- Tenant apps (multi-tenant SaaS pattern) need per-tenant scale-to-zero to control cost.

The native Kubernetes Horizontal Pod Autoscaler (HPA) can scale on CPU/memory and on a small set of External / Object metrics, but **HPA cannot scale to zero**. A Deployment with HPA always sustains at least one replica, which is unacceptable for the cost-optimization and cold-start patterns above.

KEDA fills this gap by:

1. Authoring `ScaledObject` and `ScaledJob` CRDs that wrap a Deployment / StatefulSet / Job.
2. Driving an **internally managed HPA** when the desired replica count is `>= 1`.
3. Suspending the workload to **zero replicas** when no events are present, bypassing HPA's `minReplicas: 1` floor.
4. Sourcing triggers from a 40+ scaler ecosystem (Prometheus, Kafka, Redis, RabbitMQ, AWS SQS, Azure Service Bus, NATS, Pulsar, cron, External / gRPC, HTTP add-on).
5. Binding to sovereign secrets via `TriggerAuthentication` + `ClusterTriggerAuthentication`, which integrate with OpenBao references and OIDC-federated workload identity (ADR-115) rather than raw `Secret` objects.

KEDA is CNCF **Graduated** (since 2023), under Apache 2.0, and is the de-facto event-driven autoscaler for Kubernetes.

ADR-095 (Reflex Engine) and ADR-075 (Knative) already reference KEDA implicitly. This ADR formalizes the standalone Platform-side decision so:

- The scaler primitive has a citable, audited home of its own.
- Other ADRs depend on a stable contract instead of inheriting KEDA via two different upstream documents.
- Runtime sovereign reimpl (`cave-keda`) inherits this contract directly under the blanket ADR-RUNTIME-UPSTREAM-MIRROR-001 charter principle, with no need for a per-feature override ADR.

## Candidates

| Criteria | KEDA | HPA only | Knative event broker (stand-alone) | Custom controller per source |
|---|---|---|---|---|
| Scale-to-zero | ✅ Native | ❌ `minReplicas >= 1` | ✅ HTTP only | ⚠️ Implementer's responsibility |
| Kafka / Pulsar / NATS triggers | ✅ Built-in scalers | ❌ Out of scope | ❌ HTTP-only | ⚠️ Per source |
| Cron triggers | ✅ Cron scaler | ❌ | ❌ | ⚠️ |
| Prometheus metric triggers | ✅ Prometheus scaler | ⚠️ External Metric (limited) | ❌ | ⚠️ |
| External / gRPC scaler | ✅ Pluggable | ❌ | ❌ | ⚠️ |
| K8s native | ✅ CRDs (`ScaledObject`, `ScaledJob`, `TriggerAuthentication`) | ✅ Built-in | ✅ CRDs | ⚠️ Custom CRDs |
| Sovereign secret / identity binding | ✅ `TriggerAuthentication` → OpenBao via External Secrets / CSI; OIDC workload identity (ADR-115) | ⚠️ `Secret` only | ⚠️ | ⚠️ |
| HPA coexistence | ✅ Manages an internally created HPA | n/a | n/a | ⚠️ Conflicts likely |
| License | Apache 2.0 | Apache 2.0 (in-tree) | Apache 2.0 | n/a |
| Maturity | CNCF Graduated | Stable in-tree | CNCF Incubating | n/a |

## Decision

**KEDA is the Platform-wide event-driven pod autoscaler.** Every workload that needs:

- scale-to-zero, or
- a non-CPU / non-memory trigger (Kafka, Redis, Prometheus, cron, queue, …),

must use a `ScaledObject` (long-lived workloads) or `ScaledJob` (batch / per-event Job pattern). Manual `HorizontalPodAutoscaler` resources for the same target are **not allowed** and are rejected by admission policy (ADR-030 OPA Gatekeeper).

KEDA runs in the `keda-system` namespace, deployed via Helm chart pinned to a specific upstream release. `TriggerAuthentication` resources reference OpenBao secrets through the existing OIDC workload-identity chain (ADR-115). Per-tenant `ClusterTriggerAuthentication` is forbidden by Gatekeeper to keep the secret blast radius scoped to a single tenant namespace.

KEDA is **load-bearing**: the Reflex Engine production go-live depends on it, and Knative scale-to-zero across non-HTTP triggers depends on it. Outage of `keda-operator` causes new triggers to stall; existing replicas keep serving.

## Scaler Matrix (Platform-supported subset)

| Scaler | Use case | Trigger source |
|---|---|---|
| `prometheus` | SLO-driven autoscaling, Reflex Engine | Prometheus (ADR-029) |
| `kafka` | Consumer group lag → replicas | Strimzi / Confluent (ADR-021) |
| `pulsar` | Subscription backlog → replicas | Pulsar (ADR-021) |
| `nats-jetstream` | Stream consumer backlog | NATS |
| `redis-streams`, `redis-list` | Queue depth → replicas | Valkey / Redis |
| `rabbitmq` | Queue depth | RabbitMQ |
| `aws-sqs-queue` | SQS depth | AWS provider tenants |
| `azure-servicebus` | Service Bus queue / topic | Azure provider tenants |
| `cron` | Time-window batch | Cluster cron |
| `cpu`, `memory` | Fallback / hybrid trigger only | kubelet metrics |
| `external` (gRPC) | Custom Cave-internal scaler | Tenant-supplied |
| `http-add-on` | Tenant SaaS HTTP request rate | KEDA HTTP add-on (Cave-critical for tenant-app pattern) |

`http-add-on` is called out separately because it **is** how Cave delivers HTTP scale-to-zero on stock Deployments without forcing tenants onto Knative.

## Rejected Options

### HPA only — Insufficient

HPA cannot scale to zero (`minReplicas >= 1`) and supports only CPU/memory and limited External Metrics. Reflex Engine, Knative non-HTTP triggers, and tenant SaaS scale-to-zero all require true zero. HPA stays in the stack — KEDA delegates to it for the `1..N` range — but HPA alone cannot meet requirements.

### Knative event broker stand-alone — HTTP-bound

Knative provides excellent HTTP scale-to-zero, but its broker / trigger model is biased toward HTTP and CloudEvents. Using Knative alone forces every Kafka / cron / Prometheus trigger through a CloudEvents adapter, which is slow, opaque, and adds an unnecessary delivery step. KEDA scales the workload directly off the source.

### Custom controller per source — Operationally toxic

Writing a controller per event source duplicates the lag-querying, leader election, HPA-coexistence, and authentication logic 10× over. Each controller becomes its own oncall surface. Already attempted on prior teams; outcome was a long tail of misbehaving controllers with conflicting backoffs.

## Consequences

### Positive

- **Scale-to-zero across the Platform** — Reflex Engine workers, Knative consumers, tenant SaaS apps, batch jobs all pay zero idle cost.
- **Single autoscaler primitive** — one CRD family to learn, one operator to monitor, one set of audit hooks.
- **Broad scaler ecosystem** — 40+ upstream scalers cover every event source the Platform ships today (Kafka, Pulsar, NATS, Prometheus, Redis, cron) plus likely future ones without writing code.
- **Knative + Argo Workflows alignment** — Phase 4 Knative stack (ADR-075) and Reflex Engine (ADR-095) both name KEDA explicitly; this ADR gives them a stable, citable parent.
- **Sovereign secret hygiene** — `TriggerAuthentication` + OpenBao binding plus OIDC workload identity (ADR-115) keeps Kafka SASL, Redis ACL, and webhook tokens out of plain `Secret` objects.

### Negative

- **HPA conflict surface** — KEDA owns the HPA it creates. A tenant or operator who hand-writes an HPA on the same Deployment will see flapping replicas. Mitigation: Gatekeeper policy rejects manual HPA on Deployments that already have a `ScaledObject`.
- **Node-side scale-to-zero requires Karpenter** — KEDA scales pods to zero, but absent Karpenter (ADR-032), the node fleet stays warm and the cost win is partial. Both ADRs are jointly load-bearing for the cost story.
- **`TriggerAuthentication` rotation** — Secret rotation policy must include the `TriggerAuthentication` references; otherwise rotated credentials silently stop driving scaler authentication. Mitigation: rotation runbook + monitoring on scaler activation errors.
- **Cold-start latency** — Going from `0 → 1` adds pod-start latency. Workloads that cannot tolerate this must set `minReplicaCount: 1` and forfeit the scale-to-zero saving.
- **Single operator failure domain** — `keda-operator` outage stalls new `ScaledObject` reconciliation. Existing HPAs created by KEDA continue to function from the kube-controller-manager side. Mitigation: PDB + 2 replicas + critical-priority class.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Manual HPA + ScaledObject collision causes flapping | Medium | Medium | Gatekeeper rejects manual HPA when ScaledObject targets the same workload. Audit mode for first 30 days. |
| `TriggerAuthentication` references stale secret after rotation | Medium | High (silent scaling failure) | Rotation runbook updates references. Prometheus alert on scaler activation errors > 1% / 5m. |
| Tenant scaler queries DoS upstream Prometheus | Low | Medium | Per-tenant scaler quotas; Prometheus query budget enforced via OPA. |
| KEDA upstream version skew vs Kubernetes 1.36 | Low | Medium | Pinned chart version; bump validated against ADR-RUNTIME-UPSTREAM-MIRROR-001 cadence. |
| Scaler thrash on noisy signals (Kafka lag spikes) causes oscillation | Medium | Low | `cooldownPeriod` + `pollingInterval` tuned per scaler; smoothing window required when scaler source is high-variance. |
| Operator OOM at scale (many `ScaledObject` reconciles) | Low | Medium | Dedicated PriorityClass + resource requests sized per cluster; horizontal sharding via `--enable-prometheus-metrics-server` replicas. |

## Implementation Reference

**Implementation Status:** Accepted; rollout staged.

- **Helm chart:** `kedacore/keda` pinned in `cave-platform` chart repo. Bumps go through the Platform staging cluster before promotion.
- **Namespace:** `keda-system` with restricted PodSecurity, NetworkPolicy isolating outbound to scaler targets only (Prometheus, Kafka brokers, OpenBao for `TriggerAuthentication` resolution).
- **CRDs:** `ScaledObject`, `ScaledJob`, `TriggerAuthentication`, `ClusterTriggerAuthentication`.
- **Admission policy:** Gatekeeper Constraint forbids manual HPA on `ScaledObject`-managed Deployments and forbids `ClusterTriggerAuthentication` to enforce per-tenant secret scoping.
- **Telemetry:** `keda_scaler_active`, `keda_scaler_errors`, `keda_metrics_adapter_scaler_*` scraped by Prometheus (ADR-029); dashboard exposes per-`ScaledObject` activation lag.
- **Identity & secret integration:** `TriggerAuthentication` resolves credentials via External Secrets-managed Secrets backed by OpenBao, with workload identity federated through OIDC (ADR-115). Direct `Secret` references are rejected at admission to keep credential rotation in one path.
- **Cold-start mitigations:** workloads with strict latency SLOs declare `minReplicaCount: 1` (no scale-to-zero) or pair with KEDA HTTP add-on's interceptor to absorb the first request. Default Cave tenant SaaS pattern uses `minReplicaCount: 0` and the HTTP add-on interceptor.

## Rollout Phasing

| Phase | Trigger | Workloads onboarded | Gate |
|---|---|---|---|
| 1 — Bootstrap | Helm install in `keda-system`, CRDs registered | None (operator only) | `keda-operator` healthy in staging cluster for 7 days. |
| 2 — Reflex Engine (alongside Knative) | Prometheus + Kafka scalers on Reflex worker pools (ADR-095) | Reflex playbook executors | All Reflex SLO dashboards green for 14 days. **Pull-forward proposal: KEDA moves to Knative Phase 2 here, ahead of ADR-075's original Phase 4.** |
| 3 — Tenant Kafka / Pulsar consumers | Per-tenant `ScaledObject` for stream consumers | First-mover tenants opted in via Backstage scaffolder | Gatekeeper manual-HPA rejection in enforce mode. |
| 4 — Knative + HTTP add-on | Tenant SaaS HTTP scale-to-zero on stock Deployments | Tenant apps that don't run on Knative | HTTP add-on interceptor latency p99 ≤ 200 ms cold start. |
| 5 — Cron + batch ScaledJobs | `ScaledJob` for time-window batches feeding Argo Workflows (ADR-038) | Reflex playbook batch executors, tenant cron pipelines | ScaledJob completion telemetry stable for 14 days. |

## Definition of Done

The Platform-side rollout is complete when:

- `keda-operator` runs in `keda-system` on every Platform cluster (sovereign + Azure enterprise) with PDB + 2 replicas + critical `PriorityClass`.
- Reflex Engine workers (ADR-095) scale on Prometheus + Kafka triggers via `ScaledObject` exclusively; no manual HPA on those workloads.
- Gatekeeper rejects (a) manual HPA on `ScaledObject`-targeted Deployments and (b) any `ClusterTriggerAuthentication` resource.
- `TriggerAuthentication` resolution path through External Secrets + OpenBao + OIDC workload identity validated by a synthetic credential rotation in staging.
- Backstage scaffolder template "tenant-app + KEDA HTTP add-on" available for tenant onboarding.
- Per-`ScaledObject` activation lag and scaler error rate dashboards published in Grafana.
- Runbook entries for: scaler error spike, operator pod crashloop, secret-rotation desync, manual HPA collision detection.

## License

**KEDA:** Apache 2.0 — `https://github.com/kedacore/keda/blob/main/LICENSE`

KEDA HTTP add-on (`kedacore/http-add-on`): Apache 2.0.

## Compliance Mapping

- **SOC2 CC7.2** — Monitoring and event-driven response: KEDA reacts to Prometheus alerts and queue depth signals as part of the automated remediation surface (with ADR-095).
- **ISO/IEC 27001 A.12.1.3** — Capacity management: scale-to-zero plus event-driven scale-out enforces tenant-fair capacity allocation.
- **NIS2 Directive Article 21** — Operational resilience: deterministic, declarative autoscaling reduces human error in incident response.

## Out of Scope

To keep the autoscaling surface coherent, KEDA is **not** used for:

- **Cluster (node) autoscaling** — Karpenter (ADR-032) owns node provisioning; KEDA is strictly pod-side.
- **Vertical pod autoscaling** — VPA / Goldilocks owns request/limit recommendations; KEDA only changes replica count.
- **In-pod concurrency tuning** — application-internal worker pool sizing remains the workload's responsibility.
- **Workflow orchestration** — Argo Workflows (ADR-038) remains the DAG / step engine; KEDA only triggers replica-count or `ScaledJob` lifecycle, not workflow steps.

## Notes / Roadmap

- ADR-095 (Reflex Engine) names "KEDA + Argo Workflows" as a paired stack. This ADR is the standalone Platform-side parent for the **KEDA** half; ADR-038 is the standalone parent for the **Argo Workflows** half. Reflex Engine continues to be the umbrella consumer.
- ADR-075 (Knative + KEDA) currently positions KEDA at **Phase 4**. Reflex Engine production go-live depends on KEDA, so a follow-up proposes pulling KEDA forward to **Phase 2** of the Knative rollout schedule. Tracked separately.
- ADR-040 (ARC) ships its own `HorizontalRunnerAutoscaler`. KEDA-driven ARC scaling is supported but optional; today the default is ARC HRA, with KEDA as an alternative when scaling on a non-runner signal (e.g., Kafka trigger feeding workflow runners).
- **No separate Runtime override ADR.** Runtime sovereign reimpl is the existing `cave-keda` crate scaffold and is governed entirely by the blanket charter rule **ADR-RUNTIME-UPSTREAM-MIRROR-001** — single-upstream reimpl with the same CRDs, scaler matrix, and admission contract documented here. Inheritance contract:
  - Same CRDs (`ScaledObject`, `ScaledJob`, `TriggerAuthentication`) so workloads are portable Platform ↔ Runtime without re-authoring.
  - Same scaler matrix (Prometheus, Kafka, Pulsar, NATS, Redis, cron, External, HTTP add-on).
  - Same admission policies (manual HPA forbidden when `ScaledObject` present; `ClusterTriggerAuthentication` forbidden).
  - Runtime-only additions (PQC-ready secrets, single-binary distribution, Apache 2.0 mirror-principle source bindings) live in `cave-keda` source comments, not in a separate ADR.
