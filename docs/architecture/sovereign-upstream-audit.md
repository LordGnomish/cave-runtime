# Sovereign-Cloud-Profile → Cave Runtime Upstream Audit

**Framing.** Cave Runtime is **not** a separate third deployment profile of the Platform. Cave Runtime is the **Rust reimplementation track of the OSS products that the sovereign-cloud profile actually uses**. The sovereign-cloud-deployed products are this project's *upstreams*. Vocabulary: "upstream" is runtime-side terminology only — on the Platform side the right words are "OSS product used", "stack component", or "deployment decision".

This audit cross-checks each sovereign-cloud stack component against (a) `crates/cave-upstream/src/projects.rs` and (b) the actual cave-* crate scaffolds.

## Diff table

Legend: ✅ tracked / scaffolded · ❌ missing · ⚠ mismatch (mapping inconsistency)

| sovereign OSS product used | Cave Runtime upstream | In tracker? | Cave-side crate | Notes |
|---|---|---|---|---|
| Talos Linux | Talos | ❌ | ❌ | not tracked; OS layer |
| Kubernetes (apiserver, scheduler, kubelet) | kubernetes/kubernetes | ✅ | cave-apiserver, cave-scheduler, cave-kubelet | |
| containerd | containerd | ✅ | cave-cri | |
| etcd | etcd | ✅ | cave-etcd | |
| Cilium | Cilium | ✅ | cave-ebpf-common, cave-net | ⚠ tracker maps to `cave-ebpf-common` only; `cave-net` crate exists separately |
| Cilium Hubble | Cilium Hubble | ✅ | cave-hubble (also cave-forensics in tracker) | ⚠ duplicate mapping |
| Istio (ambient) | Istio | ✅ | cave-mesh | |
| Kong | Kong | ✅ | cave-gateway | |
| Keycloak | Keycloak | ✅ | cave-auth | |
| OpenBao | OpenBao | ✅ | cave-vault | |
| External Secrets Operator | external-secrets | ✅ | cave-external-secrets | ⚠ tracker maps to `cave-vault`; dedicated crate exists |
| vcluster | vcluster | ✅ | cave-vcluster, cave-cluster | ⚠ tracker maps to `cave-cluster`; dedicated `cave-vcluster` exists |
| ArgoCD | argo-cd | ✅ | cave-deploy | |
| Argo Rollouts | argo-rollouts | ✅ | cave-rollouts | |
| Argo Workflows | argo-workflows | ✅ | cave-workflows | |
| Prometheus | prometheus | ✅ | cave-metrics | |
| Thanos | thanos | ✅ | cave-metrics | shared crate |
| Loki | loki | ✅ | cave-logs | |
| Tempo | tempo | ✅ | cave-trace | |
| Grafana | grafana | ✅ | cave-dashboard | |
| Grafana OnCall | oncall | ✅ | cave-oncall (also cave-incidents in tracker) | ⚠ duplicate mapping |
| OpenTelemetry Collector | otel-collector | ✅ | cave-trace | |
| Velero | velero | ✅ | cave-backup | |
| Harbor | harbor | ✅ | cave-registry | |
| Pulp | pulp | ✅ | cave-registry | shared crate |
| OPA Gatekeeper | gatekeeper | ✅ | cave-policy | |
| OPA | opa | ✅ | cave-policy | |
| OPAL | opal | ✅ | cave-policy | |
| Cosign / Sigstore | cosign, policy-controller | ✅ | cave-sign, cave-admission | |
| Trivy | trivy | ✅ | cave-scan | |
| OWASP ZAP | zaproxy | ✅ | cave-dast | |
| SonarQube | sonarqube | ✅ | cave-scan | shared crate |
| Tetragon | tetragon | ✅ | cave-forensics | |
| DefectDojo | DefectDojo | ✅ | cave-vulns | |
| DependencyTrack | dependency-track | ✅ | cave-sbom | |
| Chaos Mesh | chaos-mesh | ✅ | cave-chaos | |
| OpenCost | opencost | ✅ | cave-cost | |
| DevLake | devlake | ✅ | cave-devlake | |
| Uptime Kuma | uptime-kuma | ✅ | cave-uptime | |
| Unleash | unleash | ✅ | cave-flags | |
| k6 | k6 | ✅ | cave-slo | |
| Pyroscope | pyroscope | ✅ | cave-profiler | |
| cert-manager | cert-manager | ✅ | cave-certs | |
| CoreDNS | coredns | ✅ | cave-dns | |
| Backstage | backstage | ✅ | cave-portal | |
| Apicurio | apicurio-registry | ✅ | cave-docs | |
| Gitea | gitea | ✅ | cave-registry | shared crate |
| CloudNativePG | cloudnative-pg | ✅ | cave-pg | |
| Valkey | valkey | ✅ | cave-cache | |
| OpenSearch | OpenSearch | ✅ | cave-search (no scaffold yet) | ⚠ no `cave-search` crate exists; tracker references it |
| Qdrant | qdrant | ✅ | cave-search | same gap |
| MinIO | minio | ✅ | cave-store | |
| Strimzi / Kafka | strimzi, apache/kafka | ✅ | cave-streams | |
| MLflow | mlflow | ✅ | cave-ai-obs | |
| LiteLLM | litellm | ✅ | cave-llm-gateway | |
| Ollama | ollama | ✅ | cave-llm-gateway, cave-local-llm | |
| Presidio | presidio | ✅ | cave-pii | |
| LibreChat | LibreChat | ✅ | cave-chat | |
| Langfuse | langfuse | ✅ | cave-ai-obs | |
| Crossplane v2 | crossplane | ✅ | cave-infra, cave-crossplane | ⚠ tracker maps to `cave-infra`; `cave-crossplane` crate also exists |
| Knative | knative | ✅ | cave-knative (cave-deploy in tracker) | ⚠ duplicate mapping |
| KEDA | keda | ✅ | cave-keda (cave-ha in tracker) | ⚠ duplicate mapping |
| **Buildah** | **buildah** | ❌ | ❌ no `cave-build` crate | gap |
| **OpenTofu** | **opentofu** | ❌ | ❌ no `cave-iac` / `cave-tofu` | gap |
| **Spark Operator** | **spark-operator** | ❌ | ❌ | gap |
| **JupyterHub** | **jupyterhub** | ❌ | ❌ | gap |
| **Renovate** | **renovate** | ❌ | ❌ | gap |
| **Gitleaks** | **gitleaks** | ❌ | ❌ (closest: cave-secrets, cave-scan) | gap |
| **n8n** | **n8n** | ❌ | ❌ | gap |
| **Teleport CE** | **teleport** | ❌ | ⚠ `cave-pam` exists (likely target) | scaffold present, no upstream entry |

## Crates with no Hetzner-product mapping (review)

These crates exist in `crates/` but have no clear sovereign-cloud-deployed-product upstream — they may be custom CAVE components, premature scaffolds, or mappings that need correction:

`cave-alerts`, `cave-artifacts`, `cave-changelog`, `cave-compliance`, `cave-container-scan`, `cave-cost-alloc`, `cave-datafusion`, `cave-db`, `cave-docdb`, `cave-docs-site`, `cave-erp`, `cave-gitops-config`, `cave-iceberg`, `cave-kamaji`, `cave-kernel`, `cave-ledger`, `cave-lint`, `cave-pam`, `cave-permission`, `cave-pipelines`, `cave-portal-api`, `cave-rdbms`, `cave-runbook`, `cave-scaffold`, `cave-secrets`, `cave-security`, `cave-spire`, `cave-status`, `cave-techdocs`, `cave-tracker`.

Notable: `cave-iceberg` and `cave-datafusion` are pre-built but sovereign-cloud profile has no lakehouse/query-engine deployment decision yet — likely speculative. `cave-kamaji` exists but Hetzner picked **vcluster** for multi-tenancy, not Kamaji — this scaffold is dead unless the profile changes.

## Summary

- sovereign-cloud stack components inventoried: **~60**
- ✅ Tracked + scaffolded: **51**
- ❌ Missing from `cave-upstream/src/projects.rs` (tracker): **8** — Talos, Buildah, OpenTofu, Spark Operator, JupyterHub, Renovate, Gitleaks, n8n
- ❌ Missing crate scaffold: **7** — `cave-build` (Buildah), `cave-iac`/`cave-tofu` (OpenTofu), `cave-spark`, `cave-jupyter`, `cave-renovate`, `cave-secret-scan` (Gitleaks), `cave-automation` (n8n). Teleport target appears to be `cave-pam` (scaffold exists, tracker entry missing).
- ⚠ Mapping inconsistencies (tracker module ≠ existing crate): **8** — Cilium, Cilium Hubble, External Secrets, vcluster, OnCall, Crossplane, Knative, KEDA. Tracker should be reconciled with the dedicated crates that already exist.
- Speculative crates with no Hetzner product backing them: `cave-iceberg`, `cave-datafusion`, `cave-kamaji` (Hetzner uses vcluster).

## Memory note: corrected framing

> **Cave Runtime is NOT a third Platform profile.** Platform has two profiles (Azure managed-default + OSS opt-in / sovereign OSS). Cave Runtime is a separate project that takes the **OSS products used** by the sovereign-cloud profile and reimplements them line-by-line in Rust. The word "upstream" is reserved for runtime-side terminology — never used for Platform stack components.

No file in `cave-runtime/.claude/`, `cave-runtime/docs/`, `platform/docs/`, or any auto-memory directory currently defines Cave Runtime as a "third profile" or "trinity". This note is recorded here as the canonical framing.
