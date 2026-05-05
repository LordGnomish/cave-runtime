# Cave Runtime — Upstream Version Audit (2026-05-02)

**Author**: cave-upstream version-audit script (manual one-shot)
**Method**: `git ls-remote --tags --refs` against each `[upstream]` repo declared in `crates/*/parity.manifest.toml`. Tag scheme handled per-repo for non-semver projects (mongo `r8.x.x`, postgres `REL_18_3`, minio `RELEASE.YYYY-MM-DD…`, sonarqube 4-part, n8n `n8n@x.y.z`, knative `knative-vX.Y.Z`, pgbouncer `pgbouncer_x_y_z`, DefectDojo non-v).

**Honest caveat — release dates not retrieved.** GitHub REST API was rate-limited (anonymous 60/h quota exhausted; no `GITHUB_TOKEN` exposed in audit context). The "Release date" column was dropped — would have required re-running with auth. **Version-gap classification is unaffected** (computed purely from tag strings).

## Status taxonomy (Burak's kanon, 2026-05-02)

> "Projenin bütün olayı en son versiyonları hemen alıp reimplemente etmek zaten."

- **🟢 LATEST** — same `major.minor` as upstream (patch-only differences accepted).
- **🟡 BEHIND** — exactly 1 minor behind, same major. Acceptable to defer past launch.
- **🔴 STALE** — ≥ 2 minor or ≥ 1 major behind. **Blocks OSS launch (≤ 2026-05-21)**.
- **💀 DEAD_UPSTREAM** — `git ls-remote` returned 0 tags. Upstream URL wrong / project moved / closed-source.
- **📌 UNPINNED** — manifest's `[upstream].version` is a branch name, not a tag.
- **⚪ INTERNAL** — `[upstream].org = cave-runtime`, no external upstream.

## Summary

| Bucket | Count | Pre-OSS-launch action |
|---|---:|---|
| 🟢 **LATEST** (gap = 0 minor)              | 13 | none — clean |
| 🟡 **BEHIND** (1 minor behind)             |  3 | acceptable, post-launch bump |
| 🔴 **STALE** (≥ 2 minor or ≥ 1 major)      | 60 | **HIGH — bump before 2026-05-21** |
| ⛔ **DROPPED** (Charter decision, not OSS-launch blocker) | 1 | none — pump tracker entry removed |
| 💀 **DEAD_UPSTREAM** (no tags / repo moved) |  4 | **HIGH — pick replacement upstream** |
| 📌 **UNPINNED** (pinned to branch)         |  1 | **HIGH — pin to a real tag** |
| ⚪ **INTERNAL** (cave-runtime self-ref)    |  6 | none |
| **TOTAL parity manifests**                 | 88 | |

**Bottom line**: 65 / 82 *external-upstream* modules need bump tasks before launch (was 66; `cave-vcluster` dropped 2026-05-03 — Charter: kamaji is the runtime choice). 13 / 82 are clean. 3 / 82 can wait. Plus 15 cave-* crates have **no parity manifest at all** — including `cave-registry` which is on the kernel HIGH-priority list (see Manifest schema audit below).

---

## Master table — all 88 cave-* parity manifests

`Gap` column shows `Δmajor.±Δminor`. Negative `Δminor` (e.g. `1.-103`) means the ported module is on a higher minor *of an older major* (cave-cost on opencost v1.108 vs upstream v2.5.3).

| Module | Upstream repo | Manifest target | Upstream latest | Gap | Status | Bump priority | Note |
|---|---|---|---|---|---|---|---|
| `cave-admission` | `kubernetes/kubernetes` | `v1.28.0` | `v1.36.0` | `0.+8` | STALE | HIGH |  |
| `cave-ai-obs` | `langfuse/langfuse` | `v2.0.0` | `v3.172.1` | `1.+172` | STALE | HIGH |  |
| `cave-alerts` | `prometheus/alertmanager` | `v0.26.0` | `v0.32.1` | `0.+6` | STALE | HIGH |  |
| `cave-apiserver` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` | `0.+0` | LATEST | LOW |  |
| `cave-artifacts` | `pulp/pulpcore` | `3.49.0` | `3.110.0` | `0.+61` | STALE | HIGH |  |
| `cave-auth` | `keycloak/keycloak` | `v22.0.0` | `26.6.1` | `4.+6` | STALE | HIGH |  |
| `cave-backup` | `vmware-tanzu/velero` | `v1.12.0` | `v1.18.0` | `0.+6` | STALE | HIGH |  |
| `cave-cache` | `redis/redis` | `7.2.0` | `8.6.2` | `1.+4` | STALE | HIGH |  |
| `cave-certs` | `cert-manager/cert-manager` | `v1.13.0` | `v1.20.2` | `0.+7` | STALE | HIGH |  |
| `cave-changelog` | `towncrier/towncrier` | `23.0.0` | `NO_STABLE_TAG` | `n/a` | DEAD | HIGH | upstream repo missing/empty/moved |
| `cave-chaos` | `chaos-mesh/chaos-mesh` | `v2.6.0` | `v2.8.2` | `0.+2` | STALE | HIGH |  |
| `cave-chat` | `danny-avila/LibreChat` | `v0.7.0` | `v0.8.5` | `0.+1` | BEHIND | MED |  |
| `cave-cloud-controller-manager` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` | `0.+0` | LATEST | LOW |  |
| `cave-cluster` | `kubernetes-sigs/cluster-api` | `v1.6.0` | `v1.13.1` | `0.+7` | STALE | HIGH |  |
| `cave-compliance` | `open-policy-agent/gatekeeper` | `v3.14.0` | `v3.22.2` | `0.+8` | STALE | HIGH |  |
| `cave-container-scan` | `aquasecurity/trivy` | `v0.48.0` | `v0.70.0` | `0.+22` | STALE | HIGH |  |
| `cave-controller-manager` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` | `0.+0` | LATEST | LOW |  |
| `cave-core` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-cost` | `opencost/opencost` | `v1.108.0` | `v2.5.3` | `1.-103` | STALE | HIGH |  |
| `cave-cost-alloc` | `opencost/opencost` | `v1.108.0` | `v2.5.3` | `1.-103` | STALE | HIGH |  |
| `cave-cri` | `containerd/containerd` | `v2.2.3` | `v2.3.0` | `0.+1` | BEHIND | MED |  |
| `cave-crossplane` | `crossplane/crossplane` | `v1.14.0` | `v2.2.1` | `1.-12` | STALE | HIGH |  |
| `cave-dashboard` | `grafana/grafana` | `v10.2.0` | `v13.0.1` | `3.-2` | STALE | HIGH |  |
| `cave-dast` | `zaproxy/zaproxy` | `v2.14.0` | `v2.17.0` | `0.+3` | STALE | HIGH |  |
| `cave-db` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-deploy` | `argoproj/argo-cd` | `v2.9.0` | `v3.3.9` | `1.-6` | STALE | HIGH |  |
| `cave-desktop` | `zed-industries/zed` | `main` | `v1.0.0` | `n/a` | UNPINNED | HIGH | manifest pinned to branch, not tag |
| `cave-devlake` | `apache/incubator-devlake` | `v0.19.0` | `v1.0.3` | `1.-19` | STALE | HIGH |  |
| `cave-dns` | `coredns/coredns` | `v1.11.0` | `v1.14.3` | `0.+3` | STALE | HIGH |  |
| `cave-docdb` | `mongodb/mongo` | `7.0.0` | `r8.2.6` | `1.+2` | STALE | HIGH |  |
| `cave-docs` | `backstage/backstage` | `v1.20.0` | `v1.50.4` | `0.+30` | STALE | HIGH |  |
| `cave-docs-site` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-ebpf-common` | `cilium/cilium` | `v1.14.0` | `v1.19.3` | `0.+5` | STALE | HIGH |  |
| `cave-erp` | `erpnext/erpnext` | `v15.0.0` | `NO_STABLE_TAG` | `n/a` | DEAD | HIGH | upstream repo missing/empty/moved |
| `cave-etcd` | `etcd-io/etcd` | `v3.6.10` | `v3.6.11` | `0.+0` | LATEST | LOW |  |
| `cave-external-secrets` | `external-secrets/external-secrets` | `v0.9.0` | `v2.4.1` | `2.-5` | STALE | HIGH |  |
| `cave-flags` | `Unleash/unleash` | `v5.0.0` | `v7.6.3` | `2.+6` | STALE | HIGH |  |
| `cave-forensics` | `cilium/tetragon` | `v1.0.0` | `v1.7.0` | `0.+7` | STALE | HIGH |  |
| `cave-gateway` | `Kong/kong` | `v3.5.0` | `3.9.1` | `0.+4` | STALE | HIGH |  |
| `cave-gitops-config` | `fluxcd/flux2` | `v2.1.0` | `v2.8.6` | `0.+7` | STALE | HIGH |  |
| `cave-ha` | `etcd-io/etcd` | `v3.5.13` | `v3.6.11` | `0.+1` | BEHIND | MED |  |
| `cave-hubble` | `cilium/hubble` | `v0.13.0` | `v1.19.3` | `1.+6` | STALE | HIGH |  |
| `cave-incidents` | `grafana/oncall` | `v1.4.0` | `v1.16.11` | `0.+12` | STALE | HIGH |  |
| `cave-infra` | `hashicorp/terraform` | `v1.6.0` | `v1.15.1` | `0.+9` | STALE | HIGH |  |
| `cave-kamaji` | `clastix/kamaji` | `v1.0.0` | `v1.0.0` | `0.+0` | LATEST | LOW |  |
| `cave-keda` | `kedacore/keda` | `v2.12.0` | `v2.19.0` | `0.+7` | STALE | HIGH |  |
| `cave-knative` | `knative/serving` | `v1.12.0` | `knative-v1.22.0` | `0.+10` | STALE | HIGH |  |
| `cave-kube-proxy` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` | `0.+0` | LATEST | LOW |  |
| `cave-kubelet` | `kubernetes/kubernetes` | `v1.28.0` | `v1.36.0` | `0.+8` | STALE | HIGH |  |
| `cave-ledger` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-lint` | `SonarSource/sonarqube` | `v10.3.0` | `26.4.0.121862` | `16.+1` | STALE | HIGH |  |
| `cave-llm-gateway` | `BerriAI/litellm` | `v1.0.0` | `v1.82.3` | `0.+82` | STALE | HIGH |  |
| `cave-local-llm` | `ollama/ollama` | `v0.3.0` | `v0.22.1` | `0.+19` | STALE | HIGH |  |
| `cave-logs` | `grafana/loki` | `v2.9.0` | `v3.7.1` | `1.-2` | STALE | HIGH |  |
| `cave-mesh` | `istio/istio` | `1.29.2` | `1.29.2` | `0.+0` | LATEST | LOW |  |
| `cave-metrics` | `prometheus/prometheus` | `v2.48.0` | `v3.11.3` | `1.-37` | STALE | HIGH |  |
| `cave-net` | `cilium/cilium` | `v1.19.3` | `v1.19.3` | `0.+0` | LATEST | LOW |  |
| `cave-oncall` | `grafana/oncall` | `v1.4.0` | `v1.16.11` | `0.+12` | STALE | HIGH |  |
| `cave-pam` | `gravitational/teleport` | `v14.0.0` | `v18.7.6` | `4.+7` | STALE | HIGH |  |
| `cave-pg` | `pgbouncer/pgbouncer` | `1.21.0` | `1.25.1` | `0.+4` | STALE | HIGH |  |
| `cave-pii` | `microsoft/presidio` | `v2.2.0` | `2.2.362` | `0.+0` | LATEST | LOW |  |
| `cave-pipelines` | `tektoncd/pipeline` | `v0.55.0` | `v1.11.1` | `1.-44` | STALE | HIGH |  |
| `cave-policy` | `open-policy-agent/opa` | `v0.58.0` | `v1.16.1` | `1.-42` | STALE | HIGH |  |
| `cave-portal` | `backstage/backstage` | `v1.50.3` | `v1.50.4` | `0.+0` | LATEST | LOW |  |
| `cave-profiler` | `grafana/pyroscope` | `v1.3.0` | `v2.0.1` | `1.-3` | STALE | HIGH |  |
| `cave-rdbms` | `postgres/postgres` | `16.0` | `REL_18_3` | `2.+3` | STALE | HIGH |  |
| `cave-rollouts` | `argoproj/argo-rollouts` | `v1.6.0` | `v1.9.0` | `0.+3` | STALE | HIGH |  |
| `cave-runbook` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-sbom` | `DependencyTrack/dependency-track` | `v4.9.0` | `4.14.1` | `0.+5` | STALE | HIGH |  |
| `cave-scaffold` | `backstage/backstage` | `v1.20.0` | `v1.50.4` | `0.+30` | STALE | HIGH |  |
| `cave-scan` | `SonarSource/sonarqube` | `v10.3.0` | `26.4.0.121862` | `16.+1` | STALE | HIGH |  |
| `cave-scheduler` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` | `0.+0` | LATEST | LOW |  |
| `cave-secrets` | `trufflesecurity/trufflehog` | `v3.63.0` | `v3.95.2` | `0.+32` | STALE | HIGH |  |
| `cave-security` | `falcosecurity/falco` | `v0.36.0` | `0.43.1` | `0.+7` | STALE | HIGH |  |
| `cave-sign` | `sigstore/sigstore` | `v1.8.0` | `v1.10.5` | `0.+2` | STALE | HIGH |  |
| `cave-slo` | `OpenSLO/OpenSLO` | `v0.1.0` | `NO_STABLE_TAG` | `n/a` | DEAD | HIGH | upstream repo missing/empty/moved |
| `cave-spire` | `spiffe/spire` | `v1.9.0` | `v1.14.6` | `0.+5` | STALE | HIGH |  |
| `cave-status` | `louislam/uptime-kuma` | `v1.23.0` | `2.3.0` | `1.-20` | STALE | HIGH |  |
| `cave-store` | `minio/minio` | `RELEASE.2024-01-01` | `RELEASE.2025-10-15T17-29-55Z` | `1.+0` | STALE | HIGH |  |
| `cave-streams` | `apache/kafka` | `4.2.0` | `4.2.0` | `0.+0` | LATEST | LOW |  |
| `cave-trace` | `jaegertracing/jaeger` | `v1.52.0` | `v2.17.0` | `1.-35` | STALE | HIGH |  |
| `cave-tracker` | `linear-app/linear` | `v1.0.0` | `NO_STABLE_TAG` | `n/a` | DEAD | HIGH | upstream repo missing/empty/moved |
| `cave-upstream` | `cave-runtime/cave-runtime` | `v0.1.0` | `INTERNAL` | `internal/self-ref` | INTERNAL | LOW |  |
| `cave-uptime` | `louislam/uptime-kuma` | `v1.23.0` | `2.3.0` | `1.-20` | STALE | HIGH |  |
| `cave-vault` | `openbao/openbao` | `v2.5.3` | `v2.5.3` | `0.+0` | LATEST | LOW |  |
| ~~`cave-vcluster`~~ | ~~`loft-sh/vcluster`~~ | — | — | — | **DROPPED** | — | Charter: kamaji is the runtime choice for multi-tenant K8s, not vcluster (see DROPPED section). |
| `cave-vulns` | `DefectDojo/django-DefectDojo` | `v2.28.0` | `2.57.3` | `0.+29` | STALE | HIGH |  |
| `cave-workflows` | `n8n-io/n8n` | `v1.0.0` | `2.19.2` | `1.+19` | STALE | HIGH |  |

---

## ⛔ DROPPED — Charter decision (not OSS-launch blocker)

Modules whose upstream tracker was intentionally removed because the Cave runtime takes a different upstream than the original audit row implied. **Not a bug; not a STALE blocker; not pending bump.**

| Module | Old upstream (audited) | Replacement | Rationale | Date |
|---|---|---|---|---|
| `cave-cluster` (multi-tenant CP) | `loft-sh/vcluster` | `clastix/kamaji` | Charter decision: kamaji is the runtime choice for multi-tenant K8s control planes. vcluster tracker entry removed from `crates/cave-upstream/src/projects.rs` and `queue.txt`; bump-task seed deleted. `cave-kamaji` already tracked separately (LATEST status, line above). | 2026-05-03 |

---

## 🔴 STALE — HIGH priority bump (60 modules, must land before 2026-05-21)

OSS-launch blockers per Burak's kanon. Each row has a dispatch-ready bump-task seed in [`docs/upstream/bump-tasks-2026-05-02/`](bump-tasks-2026-05-02/) with the exact `PumpPayload` JSON the qwen pump consumes.

| Module | Repo | manifest -> upstream latest | Gap | Bump task seed |
|---|---|---|---|---|
| `cave-admission` | `kubernetes/kubernetes` | `v1.28.0` -> `v1.36.0` | `0.+8` | [`cave-admission-bump-kubernetes-kubernetes.json`](bump-tasks-2026-05-02/cave-admission-bump-kubernetes-kubernetes.json) |
| `cave-ai-obs` | `langfuse/langfuse` | `v2.0.0` -> `v3.172.1` | `1.+172` | [`cave-ai-obs-bump-langfuse-langfuse.json`](bump-tasks-2026-05-02/cave-ai-obs-bump-langfuse-langfuse.json) |
| `cave-alerts` | `prometheus/alertmanager` | `v0.26.0` -> `v0.32.1` | `0.+6` | [`cave-alerts-bump-prometheus-alertmanager.json`](bump-tasks-2026-05-02/cave-alerts-bump-prometheus-alertmanager.json) |
| `cave-artifacts` | `pulp/pulpcore` | `3.49.0` -> `3.110.0` | `0.+61` | [`cave-artifacts-bump-pulp-pulpcore.json`](bump-tasks-2026-05-02/cave-artifacts-bump-pulp-pulpcore.json) |
| `cave-auth` | `keycloak/keycloak` | `v22.0.0` -> `26.6.1` | `4.+6` | [`cave-auth-bump-keycloak-keycloak.json`](bump-tasks-2026-05-02/cave-auth-bump-keycloak-keycloak.json) |
| `cave-backup` | `vmware-tanzu/velero` | `v1.12.0` -> `v1.18.0` | `0.+6` | [`cave-backup-bump-vmware-tanzu-velero.json`](bump-tasks-2026-05-02/cave-backup-bump-vmware-tanzu-velero.json) |
| `cave-cache` | `redis/redis` | `7.2.0` -> `8.6.2` | `1.+4` | [`cave-cache-bump-redis-redis.json`](bump-tasks-2026-05-02/cave-cache-bump-redis-redis.json) |
| `cave-certs` | `cert-manager/cert-manager` | `v1.13.0` -> `v1.20.2` | `0.+7` | [`cave-certs-bump-cert-manager-cert-manager.json`](bump-tasks-2026-05-02/cave-certs-bump-cert-manager-cert-manager.json) |
| `cave-chaos` | `chaos-mesh/chaos-mesh` | `v2.6.0` -> `v2.8.2` | `0.+2` | [`cave-chaos-bump-chaos-mesh-chaos-mesh.json`](bump-tasks-2026-05-02/cave-chaos-bump-chaos-mesh-chaos-mesh.json) |
| `cave-cluster` | `kubernetes-sigs/cluster-api` | `v1.6.0` -> `v1.13.1` | `0.+7` | [`cave-cluster-bump-kubernetes-sigs-cluster-api.json`](bump-tasks-2026-05-02/cave-cluster-bump-kubernetes-sigs-cluster-api.json) |
| `cave-compliance` | `open-policy-agent/gatekeeper` | `v3.14.0` -> `v3.22.2` | `0.+8` | [`cave-compliance-bump-open-policy-agent-gatekeeper.json`](bump-tasks-2026-05-02/cave-compliance-bump-open-policy-agent-gatekeeper.json) |
| `cave-container-scan` | `aquasecurity/trivy` | `v0.48.0` -> `v0.70.0` | `0.+22` | [`cave-container-scan-bump-aquasecurity-trivy.json`](bump-tasks-2026-05-02/cave-container-scan-bump-aquasecurity-trivy.json) |
| `cave-cost` | `opencost/opencost` | `v1.108.0` -> `v2.5.3` | `1.-103` | [`cave-cost-bump-opencost-opencost.json`](bump-tasks-2026-05-02/cave-cost-bump-opencost-opencost.json) |
| `cave-cost-alloc` | `opencost/opencost` | `v1.108.0` -> `v2.5.3` | `1.-103` | [`cave-cost-alloc-bump-opencost-opencost.json`](bump-tasks-2026-05-02/cave-cost-alloc-bump-opencost-opencost.json) |
| `cave-crossplane` | `crossplane/crossplane` | `v1.14.0` -> `v2.2.1` | `1.-12` | [`cave-crossplane-bump-crossplane-crossplane.json`](bump-tasks-2026-05-02/cave-crossplane-bump-crossplane-crossplane.json) |
| `cave-dashboard` | `grafana/grafana` | `v10.2.0` -> `v13.0.1` | `3.-2` | [`cave-dashboard-bump-grafana-grafana.json`](bump-tasks-2026-05-02/cave-dashboard-bump-grafana-grafana.json) |
| `cave-dast` | `zaproxy/zaproxy` | `v2.14.0` -> `v2.17.0` | `0.+3` | [`cave-dast-bump-zaproxy-zaproxy.json`](bump-tasks-2026-05-02/cave-dast-bump-zaproxy-zaproxy.json) |
| `cave-deploy` | `argoproj/argo-cd` | `v2.9.0` -> `v3.3.9` | `1.-6` | [`cave-deploy-bump-argoproj-argo-cd.json`](bump-tasks-2026-05-02/cave-deploy-bump-argoproj-argo-cd.json) |
| `cave-devlake` | `apache/incubator-devlake` | `v0.19.0` -> `v1.0.3` | `1.-19` | [`cave-devlake-bump-apache-incubator-devlake.json`](bump-tasks-2026-05-02/cave-devlake-bump-apache-incubator-devlake.json) |
| `cave-dns` | `coredns/coredns` | `v1.11.0` -> `v1.14.3` | `0.+3` | [`cave-dns-bump-coredns-coredns.json`](bump-tasks-2026-05-02/cave-dns-bump-coredns-coredns.json) |
| `cave-docdb` | `mongodb/mongo` | `7.0.0` -> `r8.2.6` | `1.+2` | [`cave-docdb-bump-mongodb-mongo.json`](bump-tasks-2026-05-02/cave-docdb-bump-mongodb-mongo.json) |
| `cave-docs` | `backstage/backstage` | `v1.20.0` -> `v1.50.4` | `0.+30` | [`cave-docs-bump-backstage-backstage.json`](bump-tasks-2026-05-02/cave-docs-bump-backstage-backstage.json) |
| `cave-ebpf-common` | `cilium/cilium` | `v1.14.0` -> `v1.19.3` | `0.+5` | [`cave-ebpf-common-bump-cilium-cilium.json`](bump-tasks-2026-05-02/cave-ebpf-common-bump-cilium-cilium.json) |
| `cave-external-secrets` | `external-secrets/external-secrets` | `v0.9.0` -> `v2.4.1` | `2.-5` | [`cave-external-secrets-bump-external-secrets-external-secrets.json`](bump-tasks-2026-05-02/cave-external-secrets-bump-external-secrets-external-secrets.json) |
| `cave-flags` | `Unleash/unleash` | `v5.0.0` -> `v7.6.3` | `2.+6` | [`cave-flags-bump-Unleash-unleash.json`](bump-tasks-2026-05-02/cave-flags-bump-Unleash-unleash.json) |
| `cave-forensics` | `cilium/tetragon` | `v1.0.0` -> `v1.7.0` | `0.+7` | [`cave-forensics-bump-cilium-tetragon.json`](bump-tasks-2026-05-02/cave-forensics-bump-cilium-tetragon.json) |
| `cave-gateway` | `Kong/kong` | `v3.5.0` -> `3.9.1` | `0.+4` | [`cave-gateway-bump-Kong-kong.json`](bump-tasks-2026-05-02/cave-gateway-bump-Kong-kong.json) |
| `cave-gitops-config` | `fluxcd/flux2` | `v2.1.0` -> `v2.8.6` | `0.+7` | [`cave-gitops-config-bump-fluxcd-flux2.json`](bump-tasks-2026-05-02/cave-gitops-config-bump-fluxcd-flux2.json) |
| `cave-hubble` | `cilium/hubble` | `v0.13.0` -> `v1.19.3` | `1.+6` | [`cave-hubble-bump-cilium-hubble.json`](bump-tasks-2026-05-02/cave-hubble-bump-cilium-hubble.json) |
| `cave-incidents` | `grafana/oncall` | `v1.4.0` -> `v1.16.11` | `0.+12` | [`cave-incidents-bump-grafana-oncall.json`](bump-tasks-2026-05-02/cave-incidents-bump-grafana-oncall.json) |
| `cave-infra` | `hashicorp/terraform` | `v1.6.0` -> `v1.15.1` | `0.+9` | [`cave-infra-bump-hashicorp-terraform.json`](bump-tasks-2026-05-02/cave-infra-bump-hashicorp-terraform.json) |
| `cave-keda` | `kedacore/keda` | `v2.12.0` -> `v2.19.0` | `0.+7` | [`cave-keda-bump-kedacore-keda.json`](bump-tasks-2026-05-02/cave-keda-bump-kedacore-keda.json) |
| `cave-knative` | `knative/serving` | `v1.12.0` -> `knative-v1.22.0` | `0.+10` | [`cave-knative-bump-knative-serving.json`](bump-tasks-2026-05-02/cave-knative-bump-knative-serving.json) |
| `cave-kubelet` | `kubernetes/kubernetes` | `v1.28.0` -> `v1.36.0` | `0.+8` | [`cave-kubelet-bump-kubernetes-kubernetes.json`](bump-tasks-2026-05-02/cave-kubelet-bump-kubernetes-kubernetes.json) |
| `cave-lint` | `SonarSource/sonarqube` | `v10.3.0` -> `26.4.0.121862` | `16.+1` | [`cave-lint-bump-SonarSource-sonarqube.json`](bump-tasks-2026-05-02/cave-lint-bump-SonarSource-sonarqube.json) |
| `cave-llm-gateway` | `BerriAI/litellm` | `v1.0.0` -> `v1.82.3` | `0.+82` | [`cave-llm-gateway-bump-BerriAI-litellm.json`](bump-tasks-2026-05-02/cave-llm-gateway-bump-BerriAI-litellm.json) |
| `cave-local-llm` | `ollama/ollama` | `v0.3.0` -> `v0.22.1` | `0.+19` | [`cave-local-llm-bump-ollama-ollama.json`](bump-tasks-2026-05-02/cave-local-llm-bump-ollama-ollama.json) |
| `cave-logs` | `grafana/loki` | `v2.9.0` -> `v3.7.1` | `1.-2` | [`cave-logs-bump-grafana-loki.json`](bump-tasks-2026-05-02/cave-logs-bump-grafana-loki.json) |
| `cave-metrics` | `prometheus/prometheus` | `v2.48.0` -> `v3.11.3` | `1.-37` | [`cave-metrics-bump-prometheus-prometheus.json`](bump-tasks-2026-05-02/cave-metrics-bump-prometheus-prometheus.json) |
| `cave-oncall` | `grafana/oncall` | `v1.4.0` -> `v1.16.11` | `0.+12` | [`cave-oncall-bump-grafana-oncall.json`](bump-tasks-2026-05-02/cave-oncall-bump-grafana-oncall.json) |
| `cave-pam` | `gravitational/teleport` | `v14.0.0` -> `v18.7.6` | `4.+7` | [`cave-pam-bump-gravitational-teleport.json`](bump-tasks-2026-05-02/cave-pam-bump-gravitational-teleport.json) |
| `cave-pg` | `pgbouncer/pgbouncer` | `1.21.0` -> `1.25.1` | `0.+4` | [`cave-pg-bump-pgbouncer-pgbouncer.json`](bump-tasks-2026-05-02/cave-pg-bump-pgbouncer-pgbouncer.json) |
| `cave-pipelines` | `tektoncd/pipeline` | `v0.55.0` -> `v1.11.1` | `1.-44` | [`cave-pipelines-bump-tektoncd-pipeline.json`](bump-tasks-2026-05-02/cave-pipelines-bump-tektoncd-pipeline.json) |
| `cave-policy` | `open-policy-agent/opa` | `v0.58.0` -> `v1.16.1` | `1.-42` | [`cave-policy-bump-open-policy-agent-opa.json`](bump-tasks-2026-05-02/cave-policy-bump-open-policy-agent-opa.json) |
| `cave-profiler` | `grafana/pyroscope` | `v1.3.0` -> `v2.0.1` | `1.-3` | [`cave-profiler-bump-grafana-pyroscope.json`](bump-tasks-2026-05-02/cave-profiler-bump-grafana-pyroscope.json) |
| `cave-rdbms` | `postgres/postgres` | `16.0` -> `REL_18_3` | `2.+3` | [`cave-rdbms-bump-postgres-postgres.json`](bump-tasks-2026-05-02/cave-rdbms-bump-postgres-postgres.json) |
| `cave-rollouts` | `argoproj/argo-rollouts` | `v1.6.0` -> `v1.9.0` | `0.+3` | [`cave-rollouts-bump-argoproj-argo-rollouts.json`](bump-tasks-2026-05-02/cave-rollouts-bump-argoproj-argo-rollouts.json) |
| `cave-sbom` | `DependencyTrack/dependency-track` | `v4.9.0` -> `4.14.1` | `0.+5` | [`cave-sbom-bump-DependencyTrack-dependency-track.json`](bump-tasks-2026-05-02/cave-sbom-bump-DependencyTrack-dependency-track.json) |
| `cave-scaffold` | `backstage/backstage` | `v1.20.0` -> `v1.50.4` | `0.+30` | [`cave-scaffold-bump-backstage-backstage.json`](bump-tasks-2026-05-02/cave-scaffold-bump-backstage-backstage.json) |
| `cave-scan` | `SonarSource/sonarqube` | `v10.3.0` -> `26.4.0.121862` | `16.+1` | [`cave-scan-bump-SonarSource-sonarqube.json`](bump-tasks-2026-05-02/cave-scan-bump-SonarSource-sonarqube.json) |
| `cave-secrets` | `trufflesecurity/trufflehog` | `v3.63.0` -> `v3.95.2` | `0.+32` | [`cave-secrets-bump-trufflesecurity-trufflehog.json`](bump-tasks-2026-05-02/cave-secrets-bump-trufflesecurity-trufflehog.json) |
| `cave-security` | `falcosecurity/falco` | `v0.36.0` -> `0.43.1` | `0.+7` | [`cave-security-bump-falcosecurity-falco.json`](bump-tasks-2026-05-02/cave-security-bump-falcosecurity-falco.json) |
| `cave-sign` | `sigstore/sigstore` | `v1.8.0` -> `v1.10.5` | `0.+2` | [`cave-sign-bump-sigstore-sigstore.json`](bump-tasks-2026-05-02/cave-sign-bump-sigstore-sigstore.json) |
| `cave-spire` | `spiffe/spire` | `v1.9.0` -> `v1.14.6` | `0.+5` | [`cave-spire-bump-spiffe-spire.json`](bump-tasks-2026-05-02/cave-spire-bump-spiffe-spire.json) |
| `cave-status` | `louislam/uptime-kuma` | `v1.23.0` -> `2.3.0` | `1.-20` | [`cave-status-bump-louislam-uptime-kuma.json`](bump-tasks-2026-05-02/cave-status-bump-louislam-uptime-kuma.json) |
| `cave-store` | `minio/minio` | `RELEASE.2024-01-01` -> `RELEASE.2025-10-15T17-29-55Z` | `1.+0` | [`cave-store-bump-minio-minio.json`](bump-tasks-2026-05-02/cave-store-bump-minio-minio.json) |
| `cave-trace` | `jaegertracing/jaeger` | `v1.52.0` -> `v2.17.0` | `1.-35` | [`cave-trace-bump-jaegertracing-jaeger.json`](bump-tasks-2026-05-02/cave-trace-bump-jaegertracing-jaeger.json) |
| `cave-uptime` | `louislam/uptime-kuma` | `v1.23.0` -> `2.3.0` | `1.-20` | [`cave-uptime-bump-louislam-uptime-kuma.json`](bump-tasks-2026-05-02/cave-uptime-bump-louislam-uptime-kuma.json) |
| `cave-vulns` | `DefectDojo/django-DefectDojo` | `v2.28.0` -> `2.57.3` | `0.+29` | [`cave-vulns-bump-DefectDojo-django-DefectDojo.json`](bump-tasks-2026-05-02/cave-vulns-bump-DefectDojo-django-DefectDojo.json) |
| `cave-workflows` | `n8n-io/n8n` | `v1.0.0` -> `2.19.2` | `1.+19` | [`cave-workflows-bump-n8n-io-n8n.json`](bump-tasks-2026-05-02/cave-workflows-bump-n8n-io-n8n.json) |

---

## 🟡 BEHIND — MED priority (3 modules, post-launch acceptable)

| Module | Repo | manifest -> upstream latest | Note |
|---|---|---|---|
| `cave-chat` | `danny-avila/LibreChat` | `v0.7.0` -> `v0.8.5` | 1 minor behind |
| `cave-cri` | `containerd/containerd` | `v2.2.3` -> `v2.3.0` | 1 minor behind |
| `cave-ha` | `etcd-io/etcd` | `v3.5.13` -> `v3.6.11` | 1 minor behind |

## 🟢 LATEST — no action (13 modules)

| Module | Repo | manifest target | upstream latest |
|---|---|---|---|
| `cave-apiserver` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` |
| `cave-cloud-controller-manager` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` |
| `cave-controller-manager` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` |
| `cave-etcd` | `etcd-io/etcd` | `v3.6.10` | `v3.6.11` |
| `cave-kamaji` | `clastix/kamaji` | `v1.0.0` | `v1.0.0` |
| `cave-kube-proxy` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` |
| `cave-mesh` | `istio/istio` | `1.29.2` | `1.29.2` |
| `cave-net` | `cilium/cilium` | `v1.19.3` | `v1.19.3` |
| `cave-pii` | `microsoft/presidio` | `v2.2.0` | `2.2.362` |
| `cave-portal` | `backstage/backstage` | `v1.50.3` | `v1.50.4` |
| `cave-scheduler` | `kubernetes/kubernetes` | `v1.36.0` | `v1.36.0` |
| `cave-streams` | `apache/kafka` | `4.2.0` | `4.2.0` |
| `cave-vault` | `openbao/openbao` | `v2.5.3` | `v2.5.3` |

## 💀 DEAD_UPSTREAM — re-target required (4 modules)

`git ls-remote` returned 0 tags. Either the URL is wrong, the project moved, or it's closed-source. The manifest must be re-targeted before bump can happen.

| Module | Manifest upstream | Reality | Suggested replacement (audit hypothesis, **NOT verified**) |
|---|---|---|---|
| `cave-changelog` | `towncrier/towncrier` | repo moved | check `https://github.com/twisted/towncrier` (transferred from solo repo) |
| `cave-erp` | `erpnext/erpnext` | wrong org | canonical is `frappe/erpnext` (`https://github.com/frappe/erpnext`) |
| `cave-slo` | `OpenSLO/OpenSLO` | 0 tags via ls-remote | OpenSLO project moved/forked; check `https://github.com/OpenSLO` org for new home |
| `cave-tracker` | `linear-app/linear` | Linear is **closed-source** | OSS alternative: `makeplane/plane` or `tegonhq/tegon`. Charter-level decision — drop or pivot. |

## 📌 UNPINNED — pin to real tag (1 module)

| Module | Repo | Current pin | Latest stable tag |
|---|---|---|---|
| `cave-desktop` | `zed-industries/zed` | `main` | `v1.0.0` |

## ⚪ INTERNAL — no external upstream (6 modules)

These are first-party cave-* modules (`org = cave-runtime`). Skipped from the bump plan.

| Module | Note |
|---|---|
| `cave-core` |  first-party, no upstream to track |
| `cave-db` |  first-party, no upstream to track |
| `cave-docs-site` |  first-party, no upstream to track |
| `cave-ledger` |  first-party, no upstream to track |
| `cave-runbook` |  first-party, no upstream to track |
| `cave-upstream` |  first-party, no upstream to track |

---

## Manifest schema audit

All 88 existing `parity.manifest.toml` files conform to the `[upstream] org/repo/version` schema — **0 schema violations**. The real gap is in **coverage**.

### Crates with no parity manifest at all (15)

```text
cave-acme
cave-cdc
cave-cli
cave-datafusion
cave-iceberg
cave-kernel
cave-permission
cave-pki
cave-portal-api
cave-portal-web
cave-registry          <-- ⚠️ on kernel HIGH_PRIORITY_MODULES list
cave-runtime
cave-search
cave-techdocs
cave-tracing
```

`cave-registry` is the most urgent: it is on the kernel `HIGH_PRIORITY_MODULES` list (see `crates/cave-upstream/src/lib.rs`), so the watch daemon will look for it on the 15-minute cadence — but with no manifest the daemon has no upstream to track. Likely upstream candidates: `distribution/distribution` (CNCF reference) or `goharbor/harbor`.

### Coverage stats

- **88 / 103 cave crates have a parity manifest** (85.4%)
- **15 crates need a manifest stubbed** (14.6%)
- **Of the 88 that exist, 7 are placeholder self-refs** (`org = cave-runtime`, `version = v0.1.0`) — first-party modules, fine as-is

### Recommended action

Stub a parity manifest for each of the 15 missing crates with placeholder `[upstream].version = "unknown"` so the next audit run flags them as `UNKNOWN_PORTED` instead of being invisible.

---

## Recommended action plan

### This week (≤ 2026-05-21, OSS launch)

1. **Dispatch the 61 STALE + 1 UNPINNED + 4 DEAD_UPSTREAM bump tasks** = 66 ready-to-go seeds in [`docs/upstream/bump-tasks-2026-05-02/`](bump-tasks-2026-05-02/). Burak reviews → moves them into the qwen-pump queue at `~/Library/Application Support/cave-qwen-pump/queue/`. The pump consumes them per `crates/cave-upstream/src/pump.rs::PumpPayload`, runs the TDD port loop, opens draft PRs.
2. **Resolve 4 DEAD_UPSTREAM modules** before dispatch: edit the manifest to point at a real tagged repo. cave-tracker may need a Charter-level decision (Linear is closed-source).
3. **Stub 15 missing parity manifests.** Top priority: `cave-registry`.

### Post-launch (acceptable to defer)

1. Bump 3 BEHIND modules at the next routine cadence.
2. Re-run audit with `GITHUB_TOKEN` exported to populate release-date column.
3. Phase-2 source-level surface diff (Go AST / javap / .d.ts / cargo public-api) per ADR-RUNTIME-UPSTREAM-WATCH-001 §"Phase 2".

---

## How to dispatch the bump tasks

After Burak signs off:

```bash
mkdir -p ~/Library/Application\ Support/cave-qwen-pump/queue
cp docs/upstream/bump-tasks-2026-05-02/*.json \
   ~/Library/Application\ Support/cave-qwen-pump/queue/
```

The qwen pump picks them up, regenerates each module's parity port against the new tag, runs `cargo test`, opens draft PRs. **None auto-merge** per the safety rule in [`ADR-RUNTIME-UPSTREAM-WATCH-001`](../adr/ADR-RUNTIME-UPSTREAM-WATCH-001.md) §"Auto-PR draft, never auto-merge".

To dispatch a single module first as a smoke test:

```bash
cp docs/upstream/bump-tasks-2026-05-02/cave-cri-bump-containerd-containerd.json \
   ~/Library/Application\ Support/cave-qwen-pump/queue/
```

(cave-cri is BEHIND-by-1, smallest blast radius.)

---

## Honest scope notes — what this audit did NOT do

1. **No release-date column.** GitHub REST API rate-limited at audit time. Re-run with `GITHUB_TOKEN` populates dates.
2. **No public-API surface diff.** Tag-level only — same scope as `cave-upstream::delta::TagOnlyDiffer`. Phase-2 multi-language source diff is deferred per ADR.
3. **Vendor-announcement cross-check skipped** (Burak's brief: "web search değil"). GitHub-only data.
4. **Knative caveat.** Manifest's `v1.12.0` doesn't appear in `knative/serving` tag history (project never released a v1.x — they tag `knative-vX.Y.Z`). Reading manifest as if it meant `knative-v1.12.0` would actually make this BEHIND, not STALE — but the manifest as-written is wrong, so I scored it STALE pending a manifest fix.
5. **DEAD_UPSTREAM replacement suggestions are hypotheses**, not verified. cave-tracker → Linear is the most painful — Linear has no OSS upstream at all.
6. **`grafana/oncall` is currently archived as of late 2025** (Grafana sunset the project). Both `cave-incidents` and `cave-oncall` ride it. The audit shows them as STALE-bumpable, but in reality the upstream is dying — Charter decision needed on whether to fork the v1.16.x state or pivot to a different oncall stack.
7. **Counts assume the 15 missing manifests stay invisible.** When they're stubbed, the next audit will likely surface more STALE/DEAD entries.
