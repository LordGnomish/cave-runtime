# Parity index — `cave-runtime` workspace

**Updated:** 2026-05-24T20:17:06Z  
**Source:** `docs/parity/parity-index.json` (generated_at `2026-05-01`)  
**Crates tracked:** 112  
**Infra-only:** 27  
**fill_ratio ≥ 0.95 (excl. infra-only):** 57  
**honest_ratio ≥ 0.95 (excl. infra-only):** 11  
**adr_justified_ratio = 1.00:** 20

## Tier distribution

| Tier | Count |
|------|------:|
| 100 | 5 |
| A | 3 |
| B | 5 |
| C | 67 |
| D1 | 18 |
| D2 | 9 |
| E | 5 |

## Per-crate table

Sorted by fill_ratio descending then crate name. `—` means the
field is absent from `parity-index.json`. Source SHAs are the
first 12 chars from each crate's `parity.manifest.toml`.

| Crate | fill | honest | adr_just | tier | upstream | version | sha |
|-------|-----:|-------:|---------:|:----:|----------|---------|-----|
| `cave-auth` | 1.0000 | 0.9773 | — | C | keycloak/keycloak | `v22.0.0` | `v22.0.0` |
| `cave-bench` | 1.0000 | 0.7273 | — | C | — | `—` | `13c5a2bed634` |
| `cave-cache` | 1.0000 | 0.8947 | — | C | redis/redis | `7.2.0` | `8.0.0` |
| `cave-cloud-controller-manager` | 1.0000 | 0.9565 | 1.0000 | A | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-cri` | 1.0000 | 0.9118 | 1.0000 | 100 | containerd/containerd | `v1.7.0` | `v2.2.3` |
| `cave-crm` | 1.0000 | 0.5135 | — | C | — | `—` | `bad1f2001276` |
| `cave-dast` | 1.0000 | 0.9231 | — | D1 | zaproxy/zaproxy | `v2.14.0` | `v2.14.0` |
| `cave-datafusion` | 1.0000 | 0.4848 | 1.0000 | D2 | — | `—` | `eae7bf4fa1c0` |
| `cave-falco` | 1.0000 | 0.7308 | 1.0000 | C | — | `—` | `2c5f1ee9a4f3` |
| `cave-gitleaks` | 1.0000 | 0.9000 | — | C | — | `—` | `v8.29.1` |
| `cave-iceberg` | 1.0000 | 0.6667 | 1.0000 | D2 | — | `—` | `96cde57d9463` |
| `cave-identity` | 1.0000 | 0.7200 | 1.0000 | C | — | `—` | `b7db9650aa98` |
| `cave-kamaji` | 1.0000 | 0.8235 | — | D1 | clastix/kamaji | `v1.0.0` | `v1.0.0` |
| `cave-karpenter` | 1.0000 | 0.8636 | — | C | — | `—` | `v1.4.0` |
| `cave-knative` | 1.0000 | 1.0000 | — | C | knative/serving | `v1.12.0` | `knative-v1.2` |
| `cave-kube-proxy` | 1.0000 | 0.9412 | 1.0000 | B | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-kubevirt` | 1.0000 | 1.0000 | — | C | — | `—` | `v1.8.2` |
| `cave-llm-gateway` | 1.0000 | 0.5000 | — | C | BerriAI/litellm | `v1.0.0` | `—` |
| `cave-llm-tracker` | 1.0000 | 0.6471 | — | C | — | `—` | `—` |
| `cave-local-llm` | 1.0000 | 0.9259 | — | B | huggingface/transformers | `v4.36` | `v0.3.0` |
| `cave-mesh` | 1.0000 | 0.9730 | — | B | istio/istio | `1.20.0` | `badd809ed7d5` |
| `cave-oncall` | 1.0000 | 0.8889 | — | C | grafana/oncall | `v1.4.0` | `—` |
| `cave-rdbms-operator` | 1.0000 | 1.0000 | — | C | — | `—` | `1.24.0` |
| `cave-sandbox` | 1.0000 | 0.7458 | 1.0000 | C | — | `—` | `d8751e5ab677` |
| `cave-vault` | 1.0000 | 0.5625 | — | A | hashicorp/vault | `v1.15.0` | `4f6d47246a05` |
| `cave-net` | 0.9851 | 0.9851 | 1.0000 | C | cilium/cilium | `v1.19.3` | `v1.19.3` |
| `cave-docdb` | 0.9808 | 0.9231 | 1.0000 | C | mongodb/mongo | `7.0.0` | `v2.0.0` |
| `cave-lakehouse` | 0.9783 | 0.9348 | 1.0000 | C | — | `—` | `0.4.0` |
| `cave-crossplane` | 0.9750 | 0.6750 | — | C | crossplane/crossplane | `v1.14.0` | `41c6f9c47291` |
| `cave-kubelet` | 0.9744 | 0.9487 | 1.0000 | 100 | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-deploy` | 0.9737 | 0.6316 | — | C | argoproj/argo-cd | `v2.9.0` | `0dc6b1b57dd5` |
| `cave-rdbms` | 0.9710 | 0.9130 | 1.0000 | C | postgres/postgres | `16.0` | `REL_16_0` |
| `cave-flags` | 0.9692 | 0.9231 | — | C | Unleash/unleash | `v5.0.0` | `v5.0.0` |
| `cave-secrets` | 0.9688 | 0.4375 | — | C | trufflesecurity/trufflehog | `v3.63.0` | `v3.63.7` |
| `cave-rollouts` | 0.9677 | 0.7097 | — | C | argoproj/argo-rollouts | `v1.6.0` | `838d4e792be6` |
| `cave-gateway` | 0.9667 | 0.7333 | 1.0000 | C | Kong/kong | `v3.5.0` | `b724fc7154de` |
| `cave-metrics` | 0.9667 | 0.9000 | 1.0000 | C | prometheus/prometheus | `v2.48.0` | `—` |
| `cave-scheduler` | 0.9655 | 0.9655 | 0.9655 | 100 | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-scan` | 0.9637 | 0.9171 | — | C | SonarSource/sonarqube | `v10.3.0` | `—` |
| `cave-container-scan` | 0.9615 | 0.7115 | — | C | aquasecurity/trivy | `v0.48.0` | `8a3177aedf7e` |
| `cave-policy` | 0.9615 | 0.5769 | — | C | open-policy-agent/opa | `v0.58.0` | `85f6d990d190` |
| `cave-apiserver` | 0.9608 | 0.9412 | 1.0000 | 100 | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-dns` | 0.9583 | 0.7500 | — | C | coredns/coredns | `v1.11.0` | `17fceec6d93f` |
| `cave-forensics` | 0.9583 | 0.6818 | — | D1 | cilium/tetragon | `v1.0.0` | `1de2ed8ebea1` |
| `cave-logs` | 0.9583 | 0.8750 | 1.0000 | C | grafana/loki | `v2.9.0` | `—` |
| `cave-workflows` | 0.9583 | 0.6667 | — | C | n8n-io/n8n | `v1.0.0` | `0ab1452144d8` |
| `cave-etcd` | 0.9577 | 0.9296 | 0.9859 | 100 | etcd-io/etcd | `v3.5.13` | `v3.6.10` |
| `cave-artifacts` | 0.9571 | 0.3286 | — | C | sonatype/nexus-public | `v3.0.0` | `—` |
| `cave-controller-manager` | 0.9556 | 0.9556 | 1.0000 | B | kubernetes/kubernetes | `v1.28.0` | `v1.36.0` |
| `cave-streams` | 0.9556 | 0.9556 | — | A | apache/kafka | `3.6.0` | `—` |
| `cave-keda` | 0.9545 | 0.7500 | 1.0000 | C | kedacore/keda | `v2.12.0` | `v2.16.1` |
| `cave-hermes` | 0.9531 | 0.9531 | 1.0000 | C | — | `—` | `v2026.5.16` |
| `cave-dashboard` | 0.9524 | 0.8095 | — | C | grafana/grafana | `v10.2.0` | `—` |
| `cave-scan-db` | 0.9524 | 0.8571 | — | C | — | `—` | `2034dd8a` |
| `cave-portal` | 0.9519 | 0.8750 | — | B | backstage/backstage | `v1.20.0` | `v1.50.3` |
| `cave-sbom` | 0.9500 | 0.7333 | — | D1 | DependencyTrack/dependency-track | `v4.9.0` | `—` |
| `cave-vulns` | 0.9500 | 0.9000 | — | D1 | DefectDojo/django-DefectDojo | `v2.28.0` | `—` |
| `cave-sign` | 0.9487 | 0.5385 | — | D1 | sigstore/sigstore | `v1.8.0` | `f1ad3ee95231` |
| `cave-trace` | 0.9474 | 0.6053 | — | C | jaegertracing/jaeger | `v1.52.0` | `9866eba85aed` |
| `cave-acme` | 0.0000 | 0.0000 | — | D2 (infra) | — | `—` | `—` |
| `cave-admission` | 0.0000 | 0.0000 | — | C | kubernetes/kubernetes | `v1.28.0` | `—` |
| `cave-ai-obs` | 0.0000 | 0.0000 | — | D1 | langfuse/langfuse | `v2.0.0` | `—` |
| `cave-alerts` | 0.0000 | 0.0000 | — | C | prometheus/alertmanager | `v0.26.0` | `—` |
| `cave-backup` | 0.0000 | 0.0000 | — | C | vmware-tanzu/velero | `v1.12.0` | `—` |
| `cave-cdc` | 0.0000 | 0.0000 | — | D2 | — | `—` | `—` |
| `cave-certs` | 0.0000 | 0.0000 | — | C | cert-manager/cert-manager | `v1.13.0` | `—` |
| `cave-changelog` | 0.0000 | 0.0000 | — | D1 (infra) | towncrier/towncrier | `23.0.0` | `—` |
| `cave-chaos` | 0.0000 | 0.0000 | — | C | chaos-mesh/chaos-mesh | `v2.6.0` | `—` |
| `cave-chat` | 0.0000 | 0.0000 | — | D1 | danny-avila/LibreChat | `v0.7.0` | `—` |
| `cave-cli` | 0.0000 | 0.0000 | — | E (infra) | — | `—` | `v0.1.0` |
| `cave-cluster` | 0.0000 | 0.0000 | — | C | kubernetes-sigs/cluster-api | `v1.6.0` | `—` |
| `cave-compliance` | 0.0000 | 0.0000 | — | C | open-policy-agent/gatekeeper | `v3.14.0` | `—` |
| `cave-core` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-cost` | 0.0000 | 0.0000 | — | C | opencost/opencost | `v1.108.0` | `—` |
| `cave-cost-alloc` | 0.0000 | 0.0000 | — | C (infra) | opencost/opencost | `v1.108.0` | `—` |
| `cave-db` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-desktop` | 0.0000 | 0.0000 | — | D1 (infra) | zed-industries/zed | `main` | `—` |
| `cave-devlake` | 0.0000 | 0.0000 | — | C | apache/incubator-devlake | `v0.19.0` | `—` |
| `cave-docs` | 0.0000 | 0.0000 | — | D1 (infra) | backstage/backstage | `v1.20.0` | `—` |
| `cave-docs-site` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-ebpf-common` | 0.0000 | 0.0000 | — | D1 (infra) | cilium/cilium | `v1.14.0` | `—` |
| `cave-erp` | 0.0000 | 0.0000 | — | C | erpnext/erpnext | `v15.0.0` | `—` |
| `cave-gitops-config` | 0.0000 | 0.0000 | — | C | fluxcd/flux2 | `v2.1.0` | `—` |
| `cave-ha` | 0.0000 | 0.0000 | — | C | etcd-io/etcd | `v3.5.13` | `—` |
| `cave-incidents` | 0.0000 | 0.0000 | — | C | grafana/oncall | `v1.4.0` | `—` |
| `cave-infra` | 0.0000 | 0.0000 | — | C | hashicorp/terraform | `v1.6.0` | `—` |
| `cave-kernel` | 0.0000 | 0.0000 | — | E (infra) | — | `—` | `—` |
| `cave-ledger` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-lint` | 0.0000 | 0.0000 | — | C (infra) | SonarSource/sonarqube | `v10.3.0` | `—` |
| `cave-pam` | 0.0000 | 0.0000 | — | D1 | gravitational/teleport | `v14.0.0` | `—` |
| `cave-permission` | 0.0000 | 0.0000 | — | D2 | — | `—` | `—` |
| `cave-pii` | 0.0000 | 0.0000 | — | D1 (infra) | microsoft/presidio | `v2.2.0` | `—` |
| `cave-pipelines` | 0.0000 | 0.0000 | — | C | tektoncd/pipeline | `v0.55.0` | `—` |
| `cave-pki` | 0.0000 | 0.0000 | — | D2 (infra) | — | `—` | `—` |
| `cave-portal-api` | 0.0000 | 0.0000 | — | E (infra) | — | `—` | `—` |
| `cave-portal-web` | 0.0000 | 0.0000 | — | E (infra) | — | `—` | `—` |
| `cave-profiler` | 0.0000 | 0.0000 | — | D1 (infra) | grafana/pyroscope | `v1.3.0` | `—` |
| `cave-registry` | 0.0000 | 0.0000 | — | C (infra) | goharbor/harbor | `v2.10.0` | `—` |
| `cave-runbook` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-runtime` | 0.0000 | 0.0000 | — | E (infra) | — | `—` | `—` |
| `cave-scaffold` | 0.0000 | 0.0000 | — | C (infra) | backstage/backstage | `v1.20.0` | `—` |
| `cave-search` | 0.0000 | 0.0000 | — | D2 | — | `—` | `—` |
| `cave-security` | 0.0000 | 0.0000 | — | C | falcosecurity/falco | `v0.36.0` | `—` |
| `cave-slo` | 0.0000 | 0.0000 | — | D1 | OpenSLO/OpenSLO | `v0.1.0` | `—` |
| `cave-status` | 0.0000 | 0.0000 | — | D1 (infra) | louislam/uptime-kuma | `v1.23.0` | `—` |
| `cave-store` | 0.0000 | 0.0000 | — | C | minio/minio | `—` | `—` |
| `cave-techdocs` | 0.0000 | 0.0000 | — | D2 (infra) | — | `—` | `—` |
| `cave-tracing` | 0.0000 | 0.0000 | — | D2 (infra) | — | `—` | `—` |
| `cave-tracker` | 0.0000 | 0.0000 | — | C | linear-app/linear | `v1.0.0` | `—` |
| `cave-upstream` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `—` |
| `cave-upstream-watchd` | 0.0000 | 0.0000 | — | C (infra) | — | `—` | `v0.1.0` |
| `cave-uptime` | 0.0000 | 0.0000 | — | D1 | louislam/uptime-kuma | `v1.23.0` | `—` |

## Notes

- `fill_ratio` = `(mapped + partial + skipped) / total` per crate
  parity manifest. `1.0` means every upstream surface is either
  ported, partially ported, or explicitly skipped via an ADR-justified
  scope-cut.
- `honest_ratio` = `mapped / total` — the strictest measure (only counts
  fully-ported surfaces).
- `adr_justified_ratio` was introduced in ADR-RUNTIME-PARITY-100-PCT-001
  (2026-05-24) and tracks the share of *skipped* surfaces that are
  cited by an explicit ADR scope-cut category.

## Regenerate

```bash
cd $(git rev-parse --show-toplevel)
# The JSON itself is rebuilt by tools/parity/build-parity-index.py
python3 tools/parity/build-parity-index.py
# This markdown table is rebuilt by the doc-sync ray.
```
