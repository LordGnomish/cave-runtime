# Portal admin-UI parity audit — 2026-05-12

Burak's challenge: the dashboard says `Grade A / 100` for structural
coverage, but how many of the 83 cave-portal admin pages are *actual*
re-implementations of the upstream UI versus a `page_shell + table()`
scaffold? This audit gives a first-pass answer.

**Method.** Each `crates/cave-portal/src/admin/<x>.rs` is classified
by source size + DOM-element density:

- **substantial** — `>8 KB` source with multiple forms / tables / inputs.
  Real reimplementation candidate.
- **partial**     — `3.5–8 KB` source with some domain-specific content.
- **scaffold**    — `<3.5 KB` source. Mostly `page_shell` + a placeholder
  panel; not a port of the upstream UI.

Heuristics are NOT a substitute for hand-review. Each cell records the
evidence (`<size>B src, <forms>F/<tables>T/<inputs>I/<fns>fn`).

## Headline

| Bucket | Count | Notes |
|---|--:|---|
| Total admin pages | 77 | excluding `compliance.rs` (this dashboard) |
| Substantial (real port candidate) | 8 | size > 8 KB |
| Partial | 24 | size 3.5–8 KB |
| Scaffold | 45 | size < 3.5 KB |
| Upstream has no UI (legitimate scaffold) | 23 | CLI-/CRD-only upstreams |
| Cave-original (no upstream to mirror) | 3 | internal-only |

## Substantial admin pages (real reimplementation candidates)

| Page | Size | Evidence | Upstream UI | Note |
|---|--:|---|---|---|
| `state` | 83335 | 83335B src, 0F/0T/0I/5fn | (unmapped) | needs hand-classification |
| `contributions` | 33297 | 33297B src, 0F/4T/0I/51fn | (cave-original) | internal dashboard |
| `rdbms_operator` | 10846 | 10846B src, 0F/0T/0I/14fn | (no upstream UI) | cnpg is CRD-only |
| `streams` | 10073 | 10073B src, 0F/0T/0I/15fn | (no upstream UI) | Kafka uses CLI; AKHQ/kafdrop are external |
| `lakehouse` | 10014 | 10014B src, 0F/1T/0I/13fn | apache/iceberg — no UI; data engineers use Spark | no upstream UI |
| `keda` | 9026 | 9026B src, 0F/1T/0I/14fn | (no upstream UI) | KEDA is CRD-only |
| `pg` | 8312 | 8312B src, 1F/1T/2I/14fn | (unmapped) | needs hand-classification |
| `iam` | 8071 | 8071B src, 0F/2T/0I/12fn | (cave-original) | internal IAM UI |

## Partial admin pages

| Page | Size | Evidence | Upstream UI | Note |
|---|--:|---|---|---|
| `tenant_dashboard` | 7840 | 7840B src, 0F/2T/0I/10fn | (unmapped) | needs hand-classification |
| `mesh` | 7467 | 7467B src, 0F/2T/0I/12fn | istio/istio — Kiali (separate project) | rich web UI to port |
| `etcd` | 7428 | 7428B src, 0F/3T/0I/9fn | (no upstream UI) | etcd is CLI-only — etcdctl |
| `scheduler` | 7416 | 7416B src, 0F/2T/0I/12fn | (no upstream UI) | k8s scheduler is CLI/CRD-only |
| `net` | 7172 | 7172B src, 0F/2T/0I/11fn | cilium/cilium — Hubble UI (separate) | rich web UI to port |
| `cache` | 6273 | 6273B src, 0F/1T/0I/11fn | (no upstream UI) | redis has redis-cli + RedisInsight (external) |
| `cri` | 5979 | 5979B src, 0F/1T/0I/10fn | (no upstream UI) | containerd is CLI-only |
| `vault` | 5473 | 5473B src, 0F/2T/0I/10fn | openbao/openbao — Vault Web UI | rich web UI to port |
| `kamaji` | 5310 | 5310B src, 0F/1T/0I/9fn | (no upstream UI) | kamaji is CRD-only |
| `kubelet` | 5074 | 5074B src, 0F/1T/0I/10fn | kubernetes/kubernetes — k8s Dashboard add-on | Dashboard add-on UI to port |
| `apiserver` | 4970 | 4970B src, 0F/1T/0I/9fn | (no upstream UI) | k8s control-plane, no operator UI |
| `incidents` | 4736 | 4736B src, 0F/1T/0I/9fn | grafana/oncall — On-Call UI (React) | rich web UI to port |
| `alerts` | 4637 | 4637B src, 0F/2T/0I/10fn | prometheus/alertmanager — UI at /#/alerts | list/silence views |
| `backup` | 4473 | 4473B src, 0F/1T/0I/9fn | vmware-tanzu/velero — limited UI; mostly CLI | CLI-first, partial UI |
| `rdbms` | 4431 | 4431B src, 0F/1T/0I/9fn | (no upstream UI) | Postgres uses psql/pgAdmin (external) |
| `cloud_controller_manager` | 4320 | 4320B src, 0F/1T/0I/9fn | (no upstream UI) | k8s controller, no UI |
| `controller_manager` | 4312 | 4312B src, 0F/1T/0I/8fn | (no upstream UI) | k8s controller, no UI |
| `docdb` | 4175 | 4175B src, 0F/1T/0I/9fn | (no upstream UI) | mongo has Compass (external) |
| `policy` | 4133 | 4133B src, 0F/1T/0I/9fn | open-policy-agent/opa — Rego Playground | minimal upstream UI |
| `chaos` | 4110 | 4110B src, 0F/1T/0I/9fn | chaos-mesh/chaos-mesh — Chaos Dashboard (React) | rich web UI to port |
| `workflows` | 3731 | 3731B src, 0F/1T/0I/9fn | n8n-io/n8n — n8n Editor (Vue) | rich web UI to port |
| `artifacts` | 3704 | 3704B src, 0F/1T/0I/9fn | pulp/pulpcore — Pulp Web UI (Vue) | rich web UI to port |
| `vulns` | 3647 | 3647B src, 0F/1T/0I/10fn | DefectDojo/django-DefectDojo — UI | rich web UI to port |
| `slo` | 3553 | 3553B src, 0F/1T/0I/9fn | OpenSLO/OpenSLO — minimal UI | spec-first, no canonical UI |

## Scaffold admin pages (NOT a real UI port)

Split: legitimate-scaffold (upstream has no UI) vs work-to-do.

### Legitimate scaffolds

| Page | Size | Upstream UI | Note |
|---|--:|---|---|
| `admission` | 3181 | (no upstream UI) | k8s admission controllers are CLI/CRD-only |
| `certs` | 3068 | (no upstream UI) | cert-manager is CRD-only |
| `cluster` | 3110 | (no upstream UI) | cluster-api is CRD/CLI-only |
| `dns` | 2991 | (no upstream UI) | coredns is config-file-only |
| `ha` | 3082 | (no upstream UI) | etcd-HA is CLI/CRD-only |
| `karpenter` | 3158 | (no upstream UI) | karpenter is CRD-only |
| `knative` | 3168 | (no upstream UI) | knative is CRD-only |
| `kube_proxy` | 3186 | (no upstream UI) | kube-proxy is CLI/iptables-only |
| `ledger` | 3074 | (cave-original) | internal audit-ledger UI |
| `local_llm` | 3187 | (no upstream UI) | Ollama is CLI-only |
| `secrets` | 3171 | (no upstream UI) | trufflehog is CLI-only |

### Work-to-do (real upstream UI exists, current page is scaffold)

| Page | Size | Upstream UI | Note |
|---|--:|---|---|
| `ai_obs` | 3108 | langfuse/langfuse — Next.js dashboard | rich web UI to port |
| `auth` | 3119 | keycloak/keycloak — Admin Console (React) | rich web UI to port |
| `cdc` | 3039 | debezium/debezium-ui (deprecated) | minimal historical UI |
| `chat` | 3062 | danny-avila/LibreChat — Chat UI (React) | rich web UI to port |
| `container_scan` | 3273 | aquasecurity/trivy — minimal terminal UI | CLI-first |
| `cost` | 3033 | opencost/opencost-ui — React dashboard | rich web UI to port |
| `crm` | 3031 | twentyhq/twenty — full CRM React app | rich web UI to port |
| `crossplane` | 3155 | crossplane/crossplane — minimal UI; CRD-first | CRD-first |
| `dashboard` | 3152 | grafana/grafana — full Grafana UI | rich web UI to port — biggest scope |
| `dast` | 3039 | zaproxy/zaproxy — Java Swing UI | desktop UI, not directly portable |
| `deploy` | 3096 | argoproj/argo-cd — Web UI (React) | rich web UI to port |
| `devlake` | 3104 | apache/incubator-devlake — Web UI | rich web UI to port |
| `erp` | 3055 | frappe/erpnext — full ERP UI | rich web UI to port |
| `forensics` | 3178 | cilium/tetragon — minimal UI; mostly CLI | CLI-first |
| `gateway` | 3107 | Kong/kong-manager — Kong Manager (Angular) | rich web UI to port |
| `gitops_config` | 3103 | fluxcd/flux2 — minimal UI; CLI-first | CLI-first |
| `infra` | 3093 | hashicorp/terraform-cloud — UI | Terraform OSS itself is CLI-only |
| `kubevirt` | 3098 | kubevirt/kubevirt — limited UI; mostly CLI/CRD | CLI-first |
| `llm_gateway` | 3178 | BerriAI/litellm — admin UI (newer) | partial web UI |
| `logs` | 3081 | grafana/loki — via Grafana Explore | use Grafana panel |
| `metrics` | 3140 | prometheus/prometheus — built-in expr UI | minimal but real UI |
| `oncall` | 3104 | grafana/oncall — On-Call UI | duplicate of incidents — TODO collapse |
| `pam` | 3125 | gravitational/teleport — Web UI (React) | rich web UI to port |
| `pipelines` | 3136 | tektoncd/dashboard — Tekton Dashboard (React) | rich web UI to port |
| `rollouts` | 3135 | argoproj/argo-rollouts — Rollouts UI (React) | rich web UI to port |
| `sbom` | 3079 | DependencyTrack/dependency-track — UI | rich web UI to port |
| `scan` | 3074 | SonarSource/sonarqube — SonarQube UI | rich web UI to port |
| `search` | 3107 | opensearch-project/OpenSearch-Dashboards | rich web UI to port |
| `security` | 3107 | falcosecurity/falco — Falco UI (Falcosidekick-ui) | partial web UI |
| `store` | 3095 | minio/minio — MinIO Console (React) | rich web UI to port |
| `trace` | 3088 | jaegertracing/jaeger-ui — Jaeger UI (React) | rich web UI to port |
| `tracker` | 3116 | linear-app/linear — Linear UI | rich web UI to port |
| `upstream` | 3163 | (unmapped) | needs hand-classification |
| `uptime` | 3090 | louislam/uptime-kuma — Uptime Kuma UI | rich web UI to port |

---

**How this surfaces on `/admin/compliance`.** The new `parity_ratio`
metric tracks upstream-code parity (via the audit doc), NOT upstream-UI
parity. A future iteration could add a `portal_ui_parity` field that
ingests this audit so the dashboard distinguishes backend-parity (Grade
F today) from portal-UI-parity (this doc) explicitly. For now both
axes are tracked separately — see the dashboard's `Upstream Parity`
card for backend parity, and this doc for UI parity.
