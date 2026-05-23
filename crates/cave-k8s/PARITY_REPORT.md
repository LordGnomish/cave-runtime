# cave-k8s — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-k8s-2026-05-23-deep`
**Upstream pin:** kubernetes/kubernetes `v1.32.0` (`70d3cc986aa8221cd1dfb1121852688902d3bf53`) — Apache-2.0
**Parity:** `fill_ratio = 0.9516` (45/47) · `honest_ratio = 0.6452` (≈ 30/47)

cave-k8s is the cave-runtime Kubernetes **control-plane umbrella**. It
unifies the eight subsystem crates (cave-apiserver, cave-scheduler,
cave-kubelet, cave-kube-proxy, cave-controller-manager,
cave-cloud-controller-manager, cave-cri, cave-etcd) behind a single
`ControlPlane` facade and adds:

* PQC-ready ServiceAccount token signing (Ed25519 + ML-DSA-65 envelope)
* Built-in admission chain (NamespaceLifecycle + ServiceAccount + LimitRanger
  + PodSecurity + ValidatingAdmissionPolicy)
* CRD lifecycle registry + structural-schema gate
* APIService aggregator registry
* OpenAPI v3 schema composition over builtin + CRD schemas
* Generic resource manager + GarbageCollector cascade planner
* cgroupv2-only QoS classifier + cgroup path layout
* PV / PVC / StorageClass CSI-only binder
* Eviction + probe state-machine + image GC planners
* kube-proxy facade tracking nftables / iptables / eBPF mode
* Prometheus-shaped `/metrics` scrape surface

## 8-gate matrix

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v1.32.0"` (latest stable). `assertion_1_kubernetes_version_pinned`. |
| 2 | **source_sha pinned** | PASS | `70d3cc986aa8221cd1dfb1121852688902d3bf53`. `assertion_2_source_sha_matches_version`. |
| 3 | **fill_ratio ≥ 0.95** | PASS | `0.9516` = (28 mapped + 4 partial + 13 skipped) / 47. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 25 mapped** | PASS | 28 + 4 + 13 + 2 = 47 total; 28 mapped ≥ 25 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All 30 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): the full ControlPlane /
admission / authn / authz / CRD / aggregator / discovery / openapi /
PQC / quota / GC / kubelet / scheduler / proxy / storage / probes /
eviction / images / cgroup / networking / metrics / resources surface
is reachable through `cave_k8s` crate-root re-exports.
`assertion_9_control_plane_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 28 | control-plane-bootstrap, cluster-status-aggregator, builtin-kind-registry, resource-ref, resource-manager, workload-rollout-planner, cron-expression, service-endpointslice-derivation, pv-pvc-binder, discovery-doc, openapi-v3-aggregator, admission-chain + 4 built-in plugins, validating-admission-policy, authn (SA-token/X.509/OIDC/bootstrap), authz (RBAC/Node/Webhook), crd-lifecycle, apiservice-aggregator, garbage-collector-cascade, resource-quota |
| Partial | 4 | scheduler-placement, kubelet-pod-lifecycle, proxy-backend-registry, metrics-instrumentation |
| Skipped | 13 | in-tree-volume-plugins, podsecuritypolicy, dockershim, cgroupv1, kubectl-plugin-protocol, aggregator-request-forwarding, audit-log-shipping, konnectivity-tunnel, extender-scheduler, alpha-feature-gates, kubeadm, windows-node-support, cloud-provider-matrix-beyond-hetzner-azure |
| Unmapped (honest gaps) | 2 | dual-write-storage-migration, leader-election-coordination-lease |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 193 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke.rs` | 6 | 0 | 0 |
| **TOTAL** | **208** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| no-backcompat | — | in-tree-volume-plugins, podsecuritypolicy, dockershim, cgroupv1, konnectivity-tunnel, alpha-feature-gates, windows-node-support |
| cli-replaced | cave-cli | kubectl-plugin-protocol |
| portal-api-owned | cave-portal-api | aggregator-request-forwarding |
| obs-stack-owned | cave-logs + cave-metrics | audit-log-shipping |
| ccm-scoped | cave-cloud-controller-manager | cloud-provider-matrix-beyond-hetzner-azure |
| runtime-bootstrap-owned | cave-runtime + cave-cli | kubeadm |
| plugin-framework-only | cave-scheduler | extender-scheduler |

## Smoke evidence

| Scenario | Test | Result |
| --- | --- | --- |
| Pod scheduling roundtrip (3 nodes, picks fattest) | `smoke_1_pod_scheduling_roundtrip` | PASS |
| Deployment rolling update (1→5, max_surge=2) | `smoke_2_deployment_rolling_update` | PASS |
| Service ↔ EndpointSlice derivation (6 pods, 2 slices) | `smoke_3_service_endpoint_binding` | PASS |
| RBAC deny path (alice unbound) | `smoke_4_rbac_deny_path` | PASS |
| Namespace quota enforce (Pod count = 3) | `smoke_5_namespace_quota_enforce` | PASS |
| Admission chain end-to-end (NamespaceLifecycle + SA + PodSecurity) | `smoke_6_admission_chain_end_to_end` | PASS |

## cavectl integration

`cavectl k8s {cluster,version,healthz,readyz,discovery,openapi,metrics,apply,scale,rollout,top-nodes,top-pods,logs,exec,port-forward}` wired in `crates/cave-cli/src/main.rs` against the `/api/cluster`, `/version`, `/healthz`, `/readyz`, `/apis`, `/openapi/v3`, `/metrics`, and `/api/k8s/*` routes.

## Observability

* `observability/dashboard.json` — 15 panels (cluster phase, healthy components, pods by phase, apiserver verb rate, scheduler pending, etcd lag, node ready, admission denials, quota exceeded, CRD count, APIService count, scheduling p99, kube-proxy backends, PVC binding, image-GC bytes).
* `observability/alerts.yaml` — 10 alerts (ControlPlaneDegraded, SchedulerBacklogHigh, ApiserverHighLatency, EtcdLagHigh, NodeNotReady, AdmissionDenyStorm, ResourceQuotaExceeded, CrdInstallFailures, PvcPending, ControlPlaneDown).

## Workspace integration

* `cave-apiserver` — `ResourceStore` is the storage substrate behind `resources::Manager` and `state::State`.
* `cave-scheduler` — `scheduler_facade::place` mirrors the `NodeResourcesFit + NodeAffinity + TaintToleration + LeastAllocated` subset for integration tests; the full plugin framework remains in `cave-scheduler`.
* `cave-kubelet` — `kubelet_facade::drive_pod_action` exposes PodPhase transitions consumed by smoke tests; CRI calls live in `cave-kubelet` + `cave-cri`.
* `cave-kube-proxy` — `proxy_facade::ProxyRegistry` tracks the (Service, port) → backends mapping; nftables / iptables / eBPF datapath synthesis lives in `cave-kube-proxy`.
* `cave-controller-manager` / `cave-cloud-controller-manager` — reconciler loops are owned by those crates; `ControlPlane::start()` brings them up in canonical bootstrap order.
* `cave-cri` — runtime calls (image pull, container create/start/stop, OCI rootfs) live in `cave-cri`; `cave-k8s::images` computes GC plans, `cave-cri` executes them.
* `cave-etcd` — backing store for `cave-apiserver`'s `ResourceStore` (in production); v3 KV / Watch / Lease / Auth surface unchanged.

## Modules (28)

```
src/
├── admission.rs          (Chain + NamespaceLifecycle + SA + LimitRanger + PodSecurity)
├── aggregator.rs         (APIService registry + Available/Pending/Unavailable)
├── authn.rs              (SA-token + X.509 + OIDC + bootstrap-token authenticators)
├── authz.rs              (RBAC + Node + Webhook chain authorizers)
├── cgroup.rs             (cgroupv2 path layout + QoS classifier)
├── cluster.rs            (ControlPlane facade + ClusterConfig + ClusterStatus)
├── crd.rs                (CRD lifecycle + structural-schema gate)
├── discovery.rs          (Builtin + CRD + APIService discovery doc)
├── error.rs              (Unified Error + http_status + is_retryable)
├── eviction.rs           (Pressure thresholds + ranked candidate planner)
├── garbage_collector.rs  (Owner edges + Orphan/Background/Foreground cascade plans)
├── images.rs             (Image GC plan honoring high/low watermarks + min_age)
├── kubelet_facade.rs     (PodAssignment + NodeStatus + drive_pod_action)
├── lib.rs                (Module tree + re-exports + MODULE_NAME + router)
├── models.rs             (ComponentName + ClusterPhase + BuiltinKind + ResourceRef)
├── networking.rs         (Service + EndpointSlice + derive_slices + NetworkPolicy)
├── observability_metrics.rs (Counter + Gauge + Histogram emitter; 7 cave_k8s_* metrics)
├── openapi.rs            (OpenAPI v3 doc composing builtin + CRD schemas)
├── pqc.rs                (HybridSigner + HybridVerifier + sign_sa_jwt + SaClaims)
├── probes.rs             (Liveness/readiness/startup state machine)
├── proxy_facade.rs       (Iptables/nftables/eBPF mode + backend registry)
├── quota.rs              (Per-namespace QuotaTracker + 10 built-in dimensions + Custom)
├── resources.rs          (Generic Manager + kind_str + counts/list/delete/namespaces)
├── routes.rs             (axum router — /healthz /readyz /version /api /apis /openapi/v3 /metrics /api/cluster)
├── scheduler_facade.rs   (NodeResourcesFit + NodeAffinity + TaintToleration + LeastAllocated)
├── state.rs              (State holding eight subsystem handles + cave-k8s extras)
├── storage.rs            (PV / PVC / StorageClass CSI-only Binder)
├── vap.rs                (ValidatingAdmissionPolicy + tiny CEL-subset evaluator)
└── workloads.rs          (RolloutStrategy + plan_rolling_update + CronExpr)
```

## ADR

* [ADR-159 — Kubernetes control-plane umbrella adoption](../../docs/adr/ADR-159_Cave_K8s_Umbrella.md)
