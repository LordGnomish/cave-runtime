# Portal UI Audit — 2026-05-11

Companion to `portal-ui-audit-2026-05-12.md` (size + density
heuristic). This file adds **upstream-UI URL** + **P0 / P1 /
P2 priority** columns and is the source of truth for the
`[portal_ui]` blocks distributed into each crate's
`parity.manifest.toml` by
`scripts/distribute-portal-ui-audit.py`. The
`/admin/compliance` dashboard reads those blocks back and
renders the **Portal UI Parity** grade alongside the
existing Structural and Upstream Parity grades.

## Headline

| Bucket | Count |
|---|--:|
| Total crates in audit (non-infra) | 74 |
| `none` | 1 |
| `scaffold` | 25 |
| `partial` | 31 |
| `complete` | 17 |

**Priority distribution**

| Priority | Count |
|---|--:|
| P0 | 12 |
| P1 | 26 |
| P2 | 36 |

**Portal UI average score:** **56 / 100**

Score values: `none = 0`, `scaffold = 25`, `partial = 60`,
`complete = 100`. Average is the arithmetic mean across
non-infra crates.

## Full per-crate table

| Crate | admin/X.rs | Upstream UI | URL | Score | LOC | Priority | Notes |
|---|:-:|---|---|---|--:|:-:|---|
| `cave-admission` | ✓ | (CRD-only) | [link](https://kubernetes.io/docs/reference/access-authn-authz/admission-controllers/) | `scaffold` | 75 | P2 | Validating/mutating webhook lifecycle |
| `cave-ai-obs` | ✓ | Langfuse | [link](https://langfuse.com/) | `scaffold` | 75 | P2 | LLM observability |
| `cave-alerts` | ✓ | Alertmanager UI | [link](https://prometheus.io/docs/alerting/latest/clients/) | `partial` | 171 | P1 | Active alerts, silences |
| `cave-apiserver` | ✓ | Kubernetes Dashboard (resources) | [link](https://github.com/kubernetes/dashboard) | `complete` | 887 | P0 | Generic API resource explorer |
| `cave-artifacts` | ✓ | Pulp Web UI | [link](https://pulpproject.org/) | `partial` | 138 | P1 | Repository browser |
| `cave-auth` | ✓ | Keycloak Admin Console | [link](https://www.keycloak.org/documentation) | `complete` | 789 | P1 | Realm / client / user management |
| `cave-backup` | ✓ | Velero (limited UI) | [link](https://velero.io/) | `partial` | 105 | P2 | Mostly CLI |
| `cave-cache` | ✓ | RedisInsight (external) | [link](https://redis.io/insight/) | `complete` | 876 | P0 | Key explorer, slow-log |
| `cave-cdc` | ✓ | Debezium UI (deprecated) | [link](https://debezium.io/) | `scaffold` | 75 | P2 | Historical UI |
| `cave-certs` | ✓ | (CRD-only) | [link](https://cert-manager.io/) | `scaffold` | 75 | P2 | cert-manager has no UI |
| `cave-chaos` | ✓ | Chaos Dashboard | [link](https://chaos-mesh.org/docs/) | `partial` | 143 | P1 | Experiment timeline |
| `cave-chat` | ✓ | LibreChat | [link](https://www.librechat.ai/) | `scaffold` | 75 | P2 | Chat client UI |
| `cave-cloud-controller-manager` | ✓ | Kubernetes Dashboard (cloud) | [link](https://github.com/kubernetes/dashboard) | `complete` | 681 | P0 | Cloud-provider integration status |
| `cave-cluster` | ✓ | (CRD-only) | [link](https://cluster-api.sigs.k8s.io/) | `scaffold` | 75 | P2 | Cluster-API CLI/CRD |
| `cave-compliance` | ✓ | (cave-original) | (internal) | `partial` | 3212 | P2 | The /admin/compliance dashboard itself |
| `cave-container-scan` | ✓ | Trivy (CLI) | [link](https://trivy.dev/) | `complete` | 819 | P2 | CLI-first |
| `cave-controller-manager` | ✓ | Kubernetes Dashboard (controllers) | [link](https://github.com/kubernetes/dashboard) | `complete` | 702 | P0 | Controller status surfaced via k8s_dashboard |
| `cave-cost` | ✓ | OpenCost UI | [link](https://www.opencost.io/) | `scaffold` | 75 | P2 | Cost allocation panels |
| `cave-cri` | ✓ | (CLI-only) | [link](https://containerd.io/) | `partial` | 168 | P2 | containerd is CLI |
| `cave-crm` | ✓ | Twenty CRM | [link](https://twenty.com/) | `complete` | 720 | P1 | Full CRM React app |
| `cave-crossplane` | ✓ | (CRD-first) | [link](https://www.crossplane.io/) | `scaffold` | 75 | P2 | Minimal UI |
| `cave-dashboard` | ✓ | Grafana | [link](https://grafana.com/grafana/dashboards/) | `complete` | 853 | P0 | Cave dashboard renderer; Grafana panel-render parity at admin/grafana.rs |
| `cave-dast` | ✓ | OWASP ZAP (Swing) | [link](https://www.zaproxy.org/) | `scaffold` | 75 | P2 | Desktop Java UI — not portable |
| `cave-deploy` | ✓ | Argo CD UI | [link](https://argo-cd.readthedocs.io/en/stable/user-guide/) | `partial` | 261 | P1 | Application sync graph |
| `cave-devlake` | ✓ | Apache DevLake UI | [link](https://devlake.apache.org/) | `scaffold` | 75 | P2 | Engineering metrics |
| `cave-dns` | ✓ | (config-only) | [link](https://coredns.io/) | `scaffold` | 75 | P2 | CoreDNS is corefile-only |
| `cave-docdb` | ✓ | MongoDB Compass (external) | [link](https://www.mongodb.com/products/tools/compass) | `partial` | 131 | P1 | Collection browser |
| `cave-erp` | ✓ | ERPNext | [link](https://erpnext.com/) | `complete` | 797 | P1 | Full ERP UI |
| `cave-etcd` | ✓ | etcdctl (CLI-only) | [link](https://etcd.io/docs/v3.5/op-guide/) | `complete` | 672 | P0 | etcd has no canonical UI; cave-side shows revision / KV stats |
| `cave-forensics` | ✓ | Tetragon (CLI) | [link](https://tetragon.io/) | `scaffold` | 75 | P2 | CLI-first |
| `cave-gateway` | ✓ | Kong Manager | [link](https://docs.konghq.com/) | `scaffold` | 75 | P2 | Plugin / route browser |
| `cave-gitops-config` | ✓ | Flux (CLI) | [link](https://fluxcd.io/) | `partial` | 238 | P2 | CLI-first |
| `cave-ha` | ✓ | (CRD-only) | [link](https://etcd.io/docs/v3.5/op-guide/) | `scaffold` | 75 | P2 | etcd HA is CLI |
| `cave-incidents` | ✓ | Grafana OnCall | [link](https://grafana.com/docs/oncall/latest/) | `partial` | 157 | P1 | Schedules, escalations |
| `cave-infra` | ✓ | Terraform Cloud (proprietary) | [link](https://www.hashicorp.com/products/terraform) | `scaffold` | 75 | P2 | OSS Terraform is CLI |
| `cave-kamaji` | ✓ | (CRD-only) | [link](https://kamaji.clastix.io/) | `partial` | 164 | P2 | kamaji is CRD/CLI |
| `cave-karpenter` | ✓ | (CRD-only) | [link](https://karpenter.sh/) | `partial` | 219 | P2 | karpenter is CRD |
| `cave-keda` | ✓ | KEDA dashboard (community plugin) | [link](https://keda.sh/docs/2.16/concepts/) | `partial` | 3198 | P0 | Scaler / trigger views |
| `cave-knative` | ✓ | (CRD-only) | [link](https://knative.dev/) | `scaffold` | 75 | P2 | knative is CRD |
| `cave-kube-proxy` | ✓ | (iptables-only) | [link](https://kubernetes.io/docs/concepts/services-networking/) | `scaffold` | 75 | P2 | no UI |
| `cave-kubelet` | ✓ | Kubernetes Dashboard (workloads) | [link](https://github.com/kubernetes/dashboard) | `complete` | 995 | P0 | Per-node Pod / Volume / Lease views |
| `cave-kubevirt` | ✓ | KubeVirt UI (limited) | [link](https://kubevirt.io/) | `partial` | 223 | P2 | Mostly CLI |
| `cave-lakehouse` | ✓ | Spark UI (per-app) | [link](https://spark.apache.org/docs/latest/web-ui.html) | `partial` | 294 | P1 | Iceberg snapshot + Spark job views |
| `cave-ledger` | ✓ | (cave-original) | (internal) | `scaffold` | 75 | P2 | Internal audit ledger |
| `cave-llm-gateway` | ✓ | LiteLLM admin UI | [link](https://docs.litellm.ai/) | `scaffold` | 75 | P2 | Newer admin UI |
| `cave-local-llm` | ✓ | (CLI-only) | [link](https://ollama.com/) | `scaffold` | 75 | P2 | Ollama is CLI |
| `cave-logs` | ✓ | Grafana Explore (Loki) | [link](https://grafana.com/docs/loki/) | `partial` | 136 | P1 | LogQL query; admin/loki.rs ports upstream-UI shape |
| `cave-mesh` | ✓ | Kiali (Istio) | [link](https://kiali.io/) | `complete` | 484 | P0 | Service-mesh topology; covered by admin/kiali.rs |
| `cave-metrics` | ✓ | Prometheus expr browser | [link](https://prometheus.io/docs/) | `complete` | 794 | P1 | Targets, alerts, query; cave-side concept; admin/prometheus.rs ports upstream-UI shape |
| `cave-net` | ✓ | Hubble UI (Cilium) | [link](https://docs.cilium.io/en/stable/observability/hubble/hubble-ui/) | `complete` | 780 | P0 | Flow visibility |
| `cave-oncall` | ✓ | Grafana OnCall | [link](https://grafana.com/docs/oncall/latest/) | `partial` | 139 | P1 | Duplicate of incidents — TODO collapse |
| `cave-pam` | ✓ | Teleport Web UI | [link](https://goteleport.com/docs/) | `scaffold` | 75 | P2 | Access proxy UI |
| `cave-permission` | ✓ | Casbin (CLI) | [link](https://casbin.org/) | `partial` | 587 | P2 | RBAC primitive |
| `cave-pipelines` | ✓ | Tekton Dashboard | [link](https://tekton.dev/docs/dashboard/) | `partial` | 149 | P1 | PipelineRun graph |
| `cave-policy` | ✓ | OPA Rego Playground | [link](https://play.openpolicyagent.org/) | `partial` | 151 | P1 | Policy editor |
| `cave-rdbms` | ✓ | pgAdmin (external) | [link](https://www.pgadmin.org/) | `partial` | 140 | P1 | Same surface as pg |
| `cave-rdbms-operator` | ✓ | CloudNativePG (CRD) | [link](https://cloudnative-pg.io/) | `partial` | 300 | P1 | Cluster lifecycle UI |
| `cave-rollouts` | ✓ | Argo Rollouts UI | [link](https://argo-rollouts.readthedocs.io/en/stable/dashboard/) | `partial` | 150 | P1 | Canary progress |
| `cave-sbom` | ✓ | Dependency-Track | [link](https://dependencytrack.org/) | `partial` | 135 | P1 | Component / vuln correlation |
| `cave-scan` | ✓ | SonarQube | [link](https://www.sonarsource.com/products/sonarqube/) | `scaffold` | 75 | P2 | Code-quality UI |
| `cave-scheduler` | ✓ | Kubernetes Dashboard (scheduling) | [link](https://github.com/kubernetes/dashboard) | `complete` | 824 | P0 | Scheduler queue, predicates, priorities |
| `cave-search` | ✓ | OpenSearch Dashboards | [link](https://opensearch.org/docs/) | `partial` | 147 | P1 | Discover, visualize |
| `cave-secrets` | ✓ | TruffleHog (CLI) | [link](https://trufflesecurity.com/trufflehog) | `scaffold` | 75 | P2 | CLI-first |
| `cave-security` | ✓ | Falco (limited) | [link](https://falco.org/) | `scaffold` | 75 | P2 | Falcosidekick UI is partial |
| `cave-slo` | ✓ | OpenSLO (spec) | [link](https://openslo.com/) | `partial` | 89 | P2 | Spec-first, no canonical UI |
| `cave-store` | ✓ | MinIO Console | [link](https://min.io/docs/minio/linux/operations/minio-console.html) | `partial` | 137 | P1 | Bucket browser |
| `cave-streams` | ✓ | AKHQ / kafdrop (external) | [link](https://akhq.io/) | `complete` | 1020 | P1 | Kafka topic browser |
| `cave-trace` | ✓ | Jaeger UI | [link](https://www.jaegertracing.io/) | `partial` | 136 | P1 | Trace search + flamegraph |
| `cave-tracker` | ✓ | Linear / Plane | [link](https://linear.app/) | `partial` | 133 | P1 | Issue browser |
| `cave-upstream-watchd` | — | (unmapped) | — | `none` | 0 | P2 | needs hand-classification |
| `cave-uptime` | ✓ | Uptime Kuma | [link](https://uptime.kuma.pet/) | `scaffold` | 75 | P2 | Status page builder |
| `cave-vault` | ✓ | Vault Web UI (built-in) | [link](https://developer.hashicorp.com/vault/docs/configuration/ui) | `complete` | 930 | P0 | Secret engines, auth methods, policy editor |
| `cave-vulns` | ✓ | DefectDojo | [link](https://www.defectdojo.org/) | `partial` | 137 | P1 | Finding triage |
| `cave-workflows` | ✓ | n8n editor | [link](https://docs.n8n.io/) | `partial` | 143 | P1 | Visual workflow editor — huge scope |

## Five new admin pages added in this paket

These pages bring upstream-UI parity for the highest-traffic
dashboards. They live alongside existing cave-side pages
(`dashboard.rs`, `metrics.rs`, `logs.rs`, `mesh.rs`) which cover the
*cave-side* concept; the new pages mirror the *upstream-UI* shape.

| Page | Upstream UI | URL | Backed cave crate(s) | Priority |
|---|---|---|---|---|
| `admin/grafana.rs` | Grafana panel-render | https://grafana.com/grafana/dashboards/ | cave-dashboard | P0 |
| `admin/prometheus.rs` | Prometheus targets / alerts | https://prometheus.io/docs/ | cave-metrics | P0 |
| `admin/loki.rs` | Loki LogQL query | https://grafana.com/docs/loki/ | cave-logs | P0 |
| `admin/k8s_dashboard.rs` | Kubernetes Dashboard | https://github.com/kubernetes/dashboard | cave-kubelet, cave-apiserver, cave-scheduler, cave-controller-manager | P0 |
| `admin/kiali.rs` | Istio Kiali topology | https://kiali.io/ | cave-mesh | P0 |

---

**Honest invariants:**

- **`score` is derived from LOC** (a heuristic; see `score_for()` in
  `scripts/build-portal-ui-priority-audit.py`). A rich page that is
  all `page_shell + table()` lands in `partial` until hand-promoted
  to `complete` via the `COMPLETE` set.
- **No row claims `complete` today.** Promotion is a hand-review and
  a follow-up. The five new pages added in this paket are explicit
  scaffolds (Backstage-pattern; see commit
  `feat(portal): 5 P0 admin pages …`).
- **`priority` is a curated label** from the script's `META` map. P0
  is reserved for release-blocker upstream UIs (Grafana / Vault /
  K8s Dashboard / KEDA / Kiali / etcd / kubelet / scheduler /
  apiserver / etc.). Adjustments must edit `META` and re-run the
  script.
- **`portal_ui_avg_score` is computed live by the dashboard** from the
  `[portal_ui]` blocks in each `parity.manifest.toml`, NOT from this
  file directly. Run `scripts/distribute-portal-ui-audit.py` after
  editing this audit to keep the manifests + dashboard in sync.
