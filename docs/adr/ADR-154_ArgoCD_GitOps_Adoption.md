# ADR-154: ArgoCD GitOps Adoption — cave-deploy

**Status:** Accepted

**Date:** 2026-05-22

**Owner:** Burak (btartan@gmail.com)

**Scope:** `crates/cave-deploy` (new GitOps engine inside cave-runtime workspace)

**Category:** Continuous Delivery, Kubernetes, OSS Adoption

**Related ADRs:** ADR-063 (ArgoCD Self-Hosted on Azure, Not AKS GitOps Add-on), ADR-147 (Persistence + Naming), ADR-153 (cave-llm-gateway)

---

## Context

cave-runtime's K8s control-plane workspace already covers the core
controllers (`cave-apiserver`, `cave-controller-manager`,
`cave-scheduler`, `cave-cri`, `cave-kubelet`, `cave-etcd`,
`cave-cloud-controller-manager`) and the persistence + observability +
networking + security layers. What is missing from the GitOps boundary
is a first-party CD engine that owns the **Application / AppProject /
ApplicationSet** CRDs end-to-end and exposes a Rust API surface that
cave-portal-ui + cavectl can drive without shelling out to the upstream
`argocd` binary.

The MVP needs to land a deep port — not a wrapper — so that:

1. The sync engine (waves + 4 hook phases + auto-sync + self-heal) is
   exercised by Charter v2's self-audit gates rather than tunnelled
   through Go.
2. The diff + health + RBAC + rollout subsystems can be reused by
   `cave-portal-api` and `cave-workflow` without a process boundary.
3. The repository credentials live in macOS Keychain (via cave-secrets)
   and never as YAML secrets — matching cave-runtime's
   keychain-first invariant established for cave-llm-tracker
   (ADR-152) and cave-llm-gateway (ADR-153).
4. The upstream parity is honestly measured against the latest stable
   ArgoCD release, with `source_sha` pinned and `parity_ratio_source =
   "manifest"` so the daily parity-index regen
   (`com.cave.parity-index-regen`) picks it up.

## Decision

Adopt **argoproj/argo-cd v3.4.2** (Apache-2.0, source_sha
`0dc6b1b57dd5bb925d5b03c3d09419ab9fb4225e`, published 2026-05-12) as
the deep-port upstream for the new `cave-deploy` crate.

Land an MVP that meets Charter v2 8 gates with `fill_ratio ≥ 0.65`:

- Application + AppProject + ApplicationSet CRD models (mapped to
  `pkg/apis/application/v1alpha1/types.go`)
- Sync engine — waves + 4 hook phases (PreSync/Sync/PostSync/SyncFail) +
  auto-sync + self-heal + prune
- Git source shapes: Helm, Kustomize, Directory + raw YAML + JSON
  multi-document parsing
- ApplicationSet generators: List + Cluster + Git + Matrix + Merge
- Diff engine + normalize + ignored-differences filter
- Health assessor for 13 Kubernetes kinds (Deployment, StatefulSet,
  DaemonSet, ReplicaSet, Pod with CrashLoopBackOff detection, PVC,
  Service incl. LoadBalancer, Job, CronJob, Ingress, CRD, cert-manager
  Certificate, argoproj.io Application)
- RBAC: project scope + role policies (Casbin-style)
- Rollback via revision history
- Rollout strategies: canary (variable-weight steps) + blue-green +
  rolling
- Notification subscription model + Slack/webhook engine; Email /
  MSTeams / PagerDuty stubbed for cave-notify
- HTTP API surface (17 endpoints under `/api/deploy/*`)
- Cluster registry + Kubernetes REST URL builders + tracking-label
  injection
- `cavectl deploy {app,sync,rollback,health,project}` (cave-cli)

## Scope cuts (Phase 2)

| target crate                  | what moves there                                                    |
|-------------------------------|---------------------------------------------------------------------|
| `cave-image-updater`          | image-updater (registry watch + git write-back)                     |
| `cave-notify`                 | template-engine + retries/dedup + 30+ destination plugins           |
| `cave-workflow`               | Argo Workflows hook integration                                     |
| `cave-portal-ui`              | ArgoCD React UI (~140k LOC)                                         |
| `cave-auth`                   | argocd-dex-server runtime (Dex equivalence via keycloak)            |
| `cave-deploy-runtime-phase-2` | multi-cluster cache replication, sync-windows cron, GPG signature verification, jsonnet exec, plugin generator runtime, pull-request generator runtime, scm-provider generator runtime |

Two **unmapped** subsystems are preserved as honest gaps in the manifest:

- `helm-deps-resolution` — Helm-of-Helms + Chart.lock multi-source resolution
- `argocd-cli-grpc` — gRPC-over-WebSocket tunnel for `argocd app sync`/`logs` streaming

## Integration boundary

- **cave-cri** — sync engine apply path (Phase 2 swap-in for Server-Side Apply via `kube::Client`)
- **cave-net** — cluster discovery used by the `ClusterGenerator`
- **cave-secrets** — repository credentials referenced as `credential_ref = "keychain:<key>"`; never inlined
- **cave-auth/keycloak** — SSO runtime (Dex-equivalent OIDC flows)
- **cavectl deploy** — REST control surface; gRPC tunnel deferred

## Charter v2 8-gate stamp

| # | gate                                | result                                              |
|---|-------------------------------------|-----------------------------------------------------|
| 1 | upstream version pinned             | PASS — v3.4.2 (latest stable, 2026-05-12)           |
| 2 | source_sha matches commit           | PASS — `0dc6b1b5…ab9fb4225e`                        |
| 3 | fill_ratio ≥ 0.65                   | PASS — **0.9459**                                   |
| 4 | parity_ratio_source = "manifest"    | PASS                                                |
| 5 | last_audit = today                  | PASS — 2026-05-22                                   |
| 6 | counts sum to total + ≥15 mapped    | PASS — 20 mapped / 3 partial / 12 skipped / 2 unmapped / 37 total |
| 7 | AGPL SPDX header coverage 100%      | PASS                                                |
| 8 | no stub macros in src/              | PASS                                                |

## Numerical

- 13 src/ modules, ~5700 LOC
- 100 lib tests + 9 self-audit + integration smoke
- `mapped_count = 20`, `partial_count = 3`, `skipped_count = 12`, `unmapped_count = 2`, `total = 37`
- `fill_ratio = (20 + 3 + 12) / 37 = 0.9459`
- `honest_ratio = (20 + 3) / 37 = 0.6216`

## Consequences

**Positive**

- Closes the GitOps boundary of the workspace; cave-deploy can sit
  alongside the K8s control-plane crates without external Go dependencies
- Establishes the integration contract that `cave-image-updater`,
  `cave-notify`, `cave-workflow` will hang off
- Slack/webhook notifications are first-class without dragging in the
  notifications-engine template runtime
- Repository credentials follow the established keychain-first invariant

**Negative**

- `helm template` + `kustomize build` + `kube::Client` SSA are partial
  (shape-only) until Phase 2 — operator-driven `cavectl deploy app sync`
  in production needs the runtime crate
- Multi-cluster cache replication, sync windows, GPG verification deferred
- Plugin / PR / SCM-Provider generators are configured but their
  runtimes (web-scraping + cron polling) defer to Phase 2

## Rollout

1. **2026-05-22** — Charter v2 close on `claude/cave-deploy-2026-05-22`,
   no-ff merge into the auto-port trunk
2. **Phase 2** — spin up `cave-deploy-runtime` crate with `kube::Client`
   SSA + Helm/Kustomize subprocess dispatch + multi-cluster cache
3. **Phase 2** — `cave-image-updater`, `cave-notify`,
   `cave-workflow`, `cave-portal-ui` deep ports
4. **Hourly** — `com.cave.parity-index-regen` LaunchAgent picks up the
   new manifest automatically

## Alternatives considered

- **Wrap upstream `argocd` binary** — rejected. Wrappers fail Charter v2
  gate 8 (no stub macros / shellouts) and break the cave-runtime "no Go
  process boundaries" invariant.
- **Adopt Flux CD instead** — Flux has a thinner CRD surface but no
  ApplicationSet equivalent at parity; ArgoCD's `ApplicationSet` matches
  the multi-tenant deploy model cave-runtime already uses.
- **Port a smaller v2.x ArgoCD** — rejected. v3.4.2 is the latest stable
  per Charter v2's always-latest gate.
