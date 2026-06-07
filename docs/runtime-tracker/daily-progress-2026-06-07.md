# cave-runtime-tracker — daily progress (2026-06-07T19:23:58.318437+00:00)

**81** subsystems tracked — ✅ 8 in-sync · ⚠️ 46 behind · ❔ 27 unknown. Phase 0: **report only, no auto-bump**.

## ⚠️ Behind upstream (46)

| Subsystem | cave module | upstream | ported | latest |
|-----------|-------------|----------|--------|--------|
| Apache Kafka | `cave-streams` | [apache/kafka](https://github.com/apache/kafka) | 4.2.0 | 4.3.0 |
| Apache Pulsar | `cave-streams` | [apache/pulsar](https://github.com/apache/pulsar) | v4.2.0 | v4.2.1 |
| ArgoCD | `cave-deploy` | [argoproj/argo-cd](https://github.com/argoproj/argo-cd) | v3.4.2 | v3.4.3 |
| Backstage | `cave-portal` | [backstage/backstage](https://github.com/backstage/backstage) | v1.50.4 | v1.51.1 |
| Chaos Mesh | `cave-chaos` | [chaos-mesh/chaos-mesh](https://github.com/chaos-mesh/chaos-mesh) | v2.7.0 | v2.8.2 |
| Cilium | `cave-net` | [cilium/cilium](https://github.com/cilium/cilium) | v1.19.3 | v1.19.4 |
| DependencyTrack | `cave-sbom` | [DependencyTrack/dependency-track](https://github.com/DependencyTrack/dependency-track) | v4.11.6 | 5.0.0 |
| DevLake | `cave-devlake` | [apache/incubator-devlake](https://github.com/apache/incubator-devlake) | v0.21.1 | v1.0.3-beta12 |
| Falco | `cave-falco` | [falcosecurity/falco](https://github.com/falcosecurity/falco) | 0.43.1 | 0.44.0 |
| FerretDB | `cave-docdb` | [FerretDB/FerretDB](https://github.com/FerretDB/FerretDB) | v2.0.0 | v2.7.0 |
| Grafana | `cave-dashboard` | [grafana/grafana](https://github.com/grafana/grafana) | v11.5.0 | v13.0.1+security-01 |
| Grafana OnCall | `cave-incidents` | [grafana/oncall](https://github.com/grafana/oncall) | v1.10.0 | v1.16.11 |
| Istio | `cave-mesh` | [istio/istio](https://github.com/istio/istio) | 1.30.0 | 1.30.1 |
| KEDA | `cave-keda` | [kedacore/keda](https://github.com/kedacore/keda) | v2.16.1 | v2.20.0 |
| Kamaji | `cave-kamaji` | [clastix/kamaji](https://github.com/clastix/kamaji) | v1.0.0 | 26.6.1-edge |
| Karpenter | `cave-karpenter` | [kubernetes-sigs/karpenter](https://github.com/kubernetes-sigs/karpenter) | v1.4.0 | v1.12.1 |
| Keycloak | `cave-auth` | [keycloak/keycloak](https://github.com/keycloak/keycloak) | v22.0.0 | 26.6.3 |
| Knative + Tekton (Pipelines) | `cave-pipelines` | [tektoncd/pipeline](https://github.com/tektoncd/pipeline) | v0.55.0 | v1.13.0 |
| Knative Serving | `cave-knative` | [knative/serving](https://github.com/knative/serving) | knative-v1.22.0 | knative-v1.22.1 |
| Kong | `cave-gateway` | [Kong/kong](https://github.com/Kong/kong) | 3.9.1 | 3.9.2 |
| KubeVirt | `cave-kubevirt` | [kubevirt/kubevirt](https://github.com/kubevirt/kubevirt) | v1.8.2 | v1.8.3 |
| Langfuse | `cave-ai-obs` | [langfuse/langfuse](https://github.com/langfuse/langfuse) | v3.75.1 | v3.178.0 |
| LiteLLM | `cave-llm-gateway` | [BerriAI/litellm](https://github.com/BerriAI/litellm) | v1.85.1 | v1.88.0 |
| Loki | `cave-logs` | [grafana/loki](https://github.com/grafana/loki) | v3.4.0 | v3.7.2 |
| MinIO | `cave-store` | [minio/minio](https://github.com/minio/minio) | RELEASE.2025-04-22T22-12-26Z | RELEASE.2025-10-15T17-29-55Z |
| OPA | `cave-policy` | [open-policy-agent/opa](https://github.com/open-policy-agent/opa) | v1.16.2 | v1.17.0 |
| OPA Gatekeeper | `cave-policy` | [open-policy-agent/gatekeeper](https://github.com/open-policy-agent/gatekeeper) | v3.17.1 | v3.22.2 |
| Ollama | `cave-local-llm` | [ollama/ollama](https://github.com/ollama/ollama) | v0.3.0 | v0.30.6 |
| OpenCost | `cave-cost` | [opencost/opencost](https://github.com/opencost/opencost) | v1.108.0 | v1.120.3 |
| Presidio | `cave-pii` | [microsoft/presidio](https://github.com/microsoft/presidio) | v2.2.0 | 2.2.362 |
| Prometheus | `cave-metrics` | [prometheus/prometheus](https://github.com/prometheus/prometheus) | v3.3.0 | v3.12.0 |
| Pyroscope | `cave-profiler` | [grafana/pyroscope](https://github.com/grafana/pyroscope) | v1.3.0 | v2.0.3 |
| SPIFFE/SPIRE | `cave-identity` | [spiffe/spire](https://github.com/spiffe/spire) | v1.15.0 | v1.15.1 |
| Trivy | `cave-scan` | [aquasecurity/trivy](https://github.com/aquasecurity/trivy) | v0.70.0 | v0.71.0 |
| Twenty | `cave-crm` | [twentyhq/twenty](https://github.com/twentyhq/twenty) | v2.6.0 | sdk/v2.9.1 |
| Unleash | `cave-flags` | [Unleash/unleash](https://github.com/Unleash/unleash) | v5.0.0 | v7.6.4 |
| Uptime Kuma | `cave-uptime` | [louislam/uptime-kuma](https://github.com/louislam/uptime-kuma) | v1.23.13 | 2.4.0 |
| Valkey | `cave-cache` | [valkey-io/valkey](https://github.com/valkey-io/valkey) | 8.0.0 | 9.1.0 |
| cert-manager | `cave-certs` | [cert-manager/cert-manager](https://github.com/cert-manager/cert-manager) | v1.17.2 | v1.20.2 |
| containerd | `cave-cri` | [containerd/containerd](https://github.com/containerd/containerd) | v2.2.3 | v2.3.1 |
| etcd | `cave-etcd` | [etcd-io/etcd](https://github.com/etcd-io/etcd) | v3.6.10 | v3.6.12 |
| kube-apiserver | `cave-apiserver` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | v1.36.0 | v1.36.1 |
| kube-controller-manager | `cave-controller-manager` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | v1.36.0 | v1.36.1 |
| kube-proxy | `cave-kube-proxy` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | v1.36.0 | v1.36.1 |
| kube-scheduler | `cave-scheduler` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | v1.36.0 | v1.36.1 |
| kubelet | `cave-kubelet` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | v1.36.0 | v1.36.1 |

## ✅ In-sync (8)

Tetragon, CoreDNS, Argo Rollouts, Argo Workflows, Crossplane, OpenBao, Apache Iceberg, Apache DataFusion

## 📏 Port depth — LOC (tokei)

Ratio = cave-crate code ÷ upstream code. A *focused* re-implementation, not a 1:1 line translation — read as a trend, not a parity score.

| cave module | upstream | upstream LOC | cave LOC | depth |
|-------------|----------|-------------:|---------:|------:|
| `cave-apiserver` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | 5357039 | 31717 | 0.59% |
| `cave-scheduler` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | 5357039 | 13155 | 0.25% |
| `cave-kubelet` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | 5357039 | 17903 | 0.33% |
| `cave-controller-manager` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | 5357039 | 23888 | 0.45% |
| `cave-kube-proxy` | [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) | 5357039 | 2596 | 0.05% |
| `cave-kamaji` | [clastix/kamaji](https://github.com/clastix/kamaji) | 48551 | 2308 | 4.75% |
| `cave-net` | [cilium/cilium](https://github.com/cilium/cilium) | 4510790 | 45236 | 1.00% |
| `cave-keda` | [kedacore/keda](https://github.com/kedacore/keda) | 4134461 | 3175 | 0.08% |
| `cave-karpenter` | [kubernetes-sigs/karpenter](https://github.com/kubernetes-sigs/karpenter) | 406147 | 3317 | 0.82% |
| `cave-vault` | [openbao/openbao](https://github.com/openbao/openbao) | 582099 | 18462 | 3.17% |
| `cave-docdb` | [FerretDB/FerretDB](https://github.com/FerretDB/FerretDB) | 77328 | 5820 | 7.53% |
| `cave-streams` | [apache/kafka](https://github.com/apache/kafka) | 1192991 | 28721 | 2.41% |
| `cave-streams` | [apache/pulsar](https://github.com/apache/pulsar) | 721459 | 28721 | 3.98% |
| `cave-crm` | [twentyhq/twenty](https://github.com/twentyhq/twenty) | 1750889 | 3301 | 0.19% |

> 1 repo(s) unresolved this run (offline / rate-limited): NousResearch/Hermes-Function-Calling
