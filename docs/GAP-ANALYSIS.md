# CAVE Runtime — Gap Analysis (Sovereign Reimplementation)

**Generated:** 2026-04-13
**Perspective:** Each OSS tool from the platform docs is being **reimplemented in Rust+eBPF** as a cave-runtime crate. The upstream OSS project is tracked for API compatibility, feature parity, and protocol evolution — but we ship our own sovereign code, not the upstream binary.

---

## 1. The 73 Components → Crate Mapping

Every component in the One-Prompt Component Map must have a corresponding Rust crate that reimplements its functionality. Upstream projects are tracked in `cave-upstream` for API/protocol compatibility.

### Provider-Abstracted Components (14)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 1 | Talos Linux (K8s) | cave-cluster | 4,873 | PARTIAL — K8s client, no Talos machine config API |
| 2 | AKS (K8s) | cave-cluster | (shared) | PARTIAL — same crate, Azure-specific paths missing |
| 3 | CloudNativePG | cave-pg | 2,176 | PARTIAL — PG client/pool, no operator CRD reconciler |
| 4 | Azure PG Flexible | cave-pg | (shared) | PARTIAL — same crate, Azure ARM/REST missing |
| 5 | MinIO | cave-store | 10,719 | GOOD — S3-compatible object storage reimplemented |
| 6 | ADLS Gen2 | cave-store | (shared) | PARTIAL — Azure Blob paths need work |
| 7 | Strimzi (Kafka) | cave-streams | 10,545 | GOOD — Kafka protocol reimplemented |
| 8 | Confluent Cloud | cave-streams | (shared) | PARTIAL — managed Kafka client |
| 9 | Valkey (Redis) | cave-cache | 11,602 | GOOD — Redis protocol reimplemented |
| 10 | Azure Redis | cave-cache | (shared) | PARTIAL — Azure managed Redis paths |
| 11 | OpenSearch | **MISSING** | 0 | NOT STARTED — need cave-search crate (full-text BM25, inverted index) |
| 12 | Qdrant | **MISSING** | 0 | NOT STARTED — folded into cave-search (HNSW, vector ANN) |
| 13 | Faiss | **MISSING** | 0 | NOT STARTED — folded into cave-search (vector ANN library, IVF/PQ) |
| 14 | Milvus | **MISSING** | 0 | NOT STARTED — folded into cave-search (distributed vector DB) |

> **Note (sovereign profile):** cave-search reimplements sovereign OSS upstreams only (OpenSearch + Qdrant + Faiss + Milvus). Azure AI Search is NOT reimplemented — it is a Microsoft proprietary SaaS that the Azure profile *uses* via Crossplane XR composition (see ADR-049, ADR-114). The Cave Runtime sovereign profile must never depend on Azure AI Search.

### Self-Hosted Components — Identity & Secrets (7)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 15 | Keycloak | cave-auth | 4,997 | GOOD — OIDC/SCIM reimplemented |
| 16 | Okta (commercial) | cave-auth | (shared) | GOOD — OIDC provider, SCIM router |
| 17 | Entra ID | cave-auth | (shared) | PARTIAL — Azure RBAC integration |
| 18 | OpenBao (Vault) | cave-vault | 10,790 | GOOD — secrets engine reimplemented |
| 19 | Key Vault + ESO | cave-vault | (shared) | PARTIAL — ESO sync pattern |
| 20 | Teleport CE | cave-pam | 353 | STUB — session recording, JIT access |
| 21 | CyberArk | cave-pam | (shared) | STUB — PAM proxy, Azure path |

### Self-Hosted Components — Networking & Gateway (5)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 22 | Kong | cave-gateway | 10,183 | GOOD — API gateway, rate limiting, auth |
| 23 | Istio ambient | cave-mesh | 8,545 | GOOD — mTLS, ztunnel, waypoint proxy |
| 24 | Cilium | cave-ebpf-common + cave-dns | 7,923 | PARTIAL — eBPF foundations + DNS, need network policy engine |
| 25 | Cilium Hubble | **MISSING** | 0 | NOT STARTED — need observability in cave-ebpf or cave-forensics |
| 26 | CoreDNS | cave-dns | 7,796 | GOOD — DNS server reimplemented |

### Self-Hosted Components — Observability (7)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 27 | Prometheus | cave-metrics | 8,767 | GOOD — metrics collection/storage/query |
| 28 | Grafana | cave-dashboard | 7,785 | GOOD — dashboard engine reimplemented |
| 29 | Loki | cave-logs | 6,411 | GOOD — log aggregation reimplemented |
| 30 | Tempo | cave-trace | 8,450 | GOOD — distributed tracing reimplemented |
| 31 | Thanos | **MISSING** | 0 | NOT STARTED — need cross-cluster federation crate |
| 32 | Grafana OnCall | cave-incidents | 452 | STUB — escalation, schedules, routing |
| 33 | OpenTelemetry (SDK) | cave-trace | (shared) | PARTIAL — OTLP export, need full collector |

### Self-Hosted Components — GitOps & CI/CD (7)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 34 | ArgoCD | cave-deploy + cave-gitops-config | 6,924 | PARTIAL — app CR gen, need sync engine |
| 35 | Harbor | cave-registry | 4,105 | PARTIAL — OCI registry, need content trust |
| 36 | Argo Rollouts | cave-rollouts | 1,894 | MEDIUM — canary/blue-green, need analysis |
| 37 | ARC (GitHub Actions Runner) | **MISSING** | 0 | NOT STARTED — need runner controller crate |
| 38 | Renovate | **MISSING** | 0 | NOT STARTED — dependency update engine |
| 39 | Pulp | cave-registry | (shared) | PARTIAL — content proxy |
| 40 | Argo Workflows | cave-workflows | 501 | STUB — DAG execution, need workflow engine |

### Self-Hosted Components — Security (9)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 41 | OPA Gatekeeper | cave-policy | 11,621 | GOOD — policy engine, Rego evaluation |
| 42 | OPAL | **MISSING** | 0 | NOT STARTED — real-time policy distribution |
| 43 | Sigstore (cosign) | cave-sign | 183 | STUB — keyless signing, SLSA provenance |
| 44 | Sigstore Policy Controller | cave-admission | 981 | PARTIAL — admission webhook, need sig verify |
| 45 | DefectDojo | cave-vulns | 384 | STUB — vulnerability management |
| 46 | DependencyTrack | cave-sbom | 434 | STUB — SBOM analysis, NVD/OSV correlation |
| 47 | Trivy | cave-scan | 392 | STUB — container image scanning |
| 48 | ZAP | cave-dast | 304 | STUB — dynamic app security testing |
| 49 | Tetragon | cave-forensics | 210 | STUB — eBPF runtime security, need kernel hooks |

### Self-Hosted Components — AI/LLM (5)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 50 | LiteLLM | cave-llm-gateway | 3,684 | PARTIAL — proxy routing, need classification |
| 51 | Ollama | cave-llm-gateway | (shared) | PARTIAL — local inference serving |
| 52 | Presidio | cave-pii | 204 | STUB — PII detection/redaction |
| 53 | LibreChat | cave-chat | 206 | STUB — conversational UI |
| 54 | Langfuse | cave-ai-obs | 201 | STUB — LLM observability, traces |

### Self-Hosted Components — Data Platform (4)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 55 | Spark Operator | **MISSING** | 0 | NOT STARTED — data processing |
| 56 | JupyterHub | **MISSING** | 0 | NOT STARTED — notebook serving |
| 57 | MLflow | **MISSING** | 0 | NOT STARTED — experiment tracking |
| 58 | Databricks (commercial) | **MISSING** | 0 | NOT STARTED — managed data platform |

### Self-Hosted Components — Developer Experience (4)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 59 | Backstage | cave-portal | 1,072 | MINIMAL — HTML page, need catalog/scaffolder/plugins |
| 60 | Unleash | cave-flags | 2,945 | MEDIUM — feature flags, strategies, SDK protocol |
| 61 | Gitea HA | **MISSING** | 0 | NOT STARTED — tenant Git hosting |
| 62 | GitHub Enterprise | **MISSING** | 0 | NOT STARTED — tenant Git (Azure path) |

### Self-Hosted Components — Platform Operations (10)

| # | OSS Upstream | Cave Crate | Lines | Reimplementation Status |
|---|-------------|-----------|-------|------------------------|
| 63 | Chaos Mesh | cave-chaos | 420 | STUB — fault injection, chaos experiments |
| 64 | Velero | cave-backup | 2,662 | MEDIUM — backup/restore, schedule |
| 65 | OpenCost | cave-cost | 1,965 | MEDIUM — cost allocation, attribution |
| 66 | DevLake | cave-devlake | 205 | STUB — DORA metrics, data collection |
| 67 | Uptime Kuma | cave-uptime | 299 | STUB — synthetic monitoring |
| 68 | n8n | cave-workflows | (shared) | STUB — workflow automation |
| 69 | vcluster | **MISSING** | 0 | NOT STARTED — virtual cluster lifecycle |
| 70 | k6 | **MISSING** | 0 | NOT STARTED — load testing |
| 71 | Pyroscope | cave-profiler | 180 | STUB — continuous profiling |
| 72 | Sovereign Ledger | cave-ledger | 821 | NEW — Merkle chain, audit trail (no OSS upstream, native) |
| 73 | Crossplane v2 | **MISSING** | 0 | NOT STARTED — XRD composition engine |

### Additional Internal Crates (no OSS upstream, CAVE-native)

| Cave Crate | Lines | Purpose |
|-----------|-------|---------|
| cave-core | 1,190+ | Shared types, profile system, tenant model, errors |
| cave-cli | 1,416 | cave-ctl CLI binary |
| cave-runtime | 274 | Main binary entry point |
| cave-upstream | 405 | OSS upstream project tracker |
| cave-ha | 5,173 | HA primitives (Raft, leader election, failover) |
| cave-infra | 4,648 | Infrastructure provisioning (OpenTofu wrapper) |
| cave-security | 6,350 | Security framework (zero-trust, network policy) |
| cave-compliance | 2,289 | Compliance export (SOC2/ISO27001/NIS2/GDPR) |
| cave-cost-alloc | 1,737 | Per-tenant cost allocation |
| cave-tracker | 4,898 | Issue/work tracking |
| cave-runbook | 3,237 | Operational runbook engine |
| cave-scaffold | 352 | Project scaffolding templates |
| cave-docs-site | 2,185 | Documentation site engine |
| cave-artifacts | 4,991 | Build artifact management |
| cave-pipelines | 4,204 | CI/CD pipeline engine (27-stage) |
| cave-db | 696 | Database abstraction layer |
| cave-ebpf-common | 127 | Shared eBPF types |
| cave-alerts | 508 | Alerting engine |
| cave-slo | 409 | SLO tracking/budget |
| cave-changelog | 300 | Changelog generation |
| cave-status | 274 | Status page engine |
| cave-docs | 259 | API documentation / schema registry |
| cave-lint | 291 | Dockerfile/config linting |
| cave-certs | 256 | Certificate lifecycle management |
| cave-secrets | 374 | Secret scanning/detection |
| cave-sign | 183 | Image signing/verification |

---

## 2. Summary Scorecard

| Category | Total Components | Crate Exists | GOOD (>4K) | MEDIUM (1K-4K) | STUB (<1K) | MISSING |
|----------|-----------------|-------------|-----------|---------------|-----------|---------|
| Provider-Abstracted | 14 | 10 | 4 (store,streams,cache,auth) | 1 (pg) | 0 | 4 (search, vector) |
| Identity & Secrets | 7 | 5 | 2 (auth,vault) | 0 | 1 (pam) | 0 |
| Networking | 6 | 5 | 3 (gateway,mesh,dns) | 0 | 0 | 1 (hubble) |
| Observability | 7 | 6 | 4 (metrics,dashboard,logs,trace) | 0 | 1 (incidents) | 1 (thanos) |
| GitOps & CI/CD | 7 | 5 | 0 | 2 (deploy+gitops, registry) | 1 (rollouts,workflows) | 2 (arc, renovate) |
| Security | 9 | 8 | 1 (policy) | 0 | 6 | 1 (opal) |
| AI/LLM | 5 | 5 | 0 | 1 (llm-gateway) | 4 | 0 |
| Data Platform | 4 | 0 | 0 | 0 | 0 | 4 |
| Developer Experience | 4 | 3 | 0 | 1 (flags) | 1 (portal) | 1 (gitea) |
| Platform Operations | 11 | 9 | 0 | 2 (backup, cost) | 6 | 3 (vcluster,k6,crossplane) |
| **TOTAL** | **73** | **56** | **14** | **7** | **20** | **17** |

### Key Numbers
- **14 GOOD** — substantial Rust reimplementation (>4K lines each), ~120K lines total
- **7 MEDIUM** — meaningful code (1K–4K), need expansion
- **20 STUBS** — skeleton crates with API routes but minimal logic (<1K lines)
- **17 MISSING** — no crate exists yet, need to create
- **67 crates** exist today, **~17 new crates** needed to cover all 73 + internal crates

---

## 3. What "GOOD" Actually Means

These 14 crates are real Rust reimplementations with significant logic, not just API wrappers:

| Crate | Lines | Reimplements | Key Capabilities |
|-------|-------|-------------|-----------------|
| cave-cache | 11,602 | Valkey/Redis | RESP protocol, data structures, eviction, clustering |
| cave-policy | 11,621 | OPA Gatekeeper | Rego evaluation, constraint framework, audit |
| cave-vault | 10,790 | OpenBao/Vault | Secret engines, transit, PKI, dynamic credentials |
| cave-store | 10,719 | MinIO | S3 API, object lifecycle, replication |
| cave-streams | 10,545 | Strimzi/Kafka | Kafka protocol, consumer groups, partitioning |
| cave-gateway | 10,183 | Kong | API routing, rate limiting, auth plugins, OpenAPI |
| cave-metrics | 8,767 | Prometheus | PromQL, TSDB, scraping, alerting rules |
| cave-mesh | 8,545 | Istio ambient | mTLS, ztunnel, L7 policy, waypoint proxies |
| cave-trace | 8,450 | Tempo | Distributed tracing, OTLP ingest, trace storage |
| cave-dns | 7,796 | CoreDNS/Cilium | DNS server, zone management, service discovery |
| cave-dashboard | 7,785 | Grafana | Dashboard rendering, panel types, data sources |
| cave-logs | 6,411 | Loki | Log ingestion, LogQL, storage, tenant isolation |
| cave-security | 6,350 | (native) | Zero-trust framework, network policy |
| cave-auth | 4,997 | Keycloak/Okta | OIDC, SCIM 2.0, RBAC, token management |

---

## 4. Missing Crates to Create (17)

| New Crate | Reimplements | Priority |
|-----------|-------------|----------|
| cave-search | OpenSearch + Qdrant + Faiss + Milvus (sovereign OSS, full-text + vector/ANN) | HIGH (Phase 2) |
| cave-thanos | Thanos (cross-cluster metrics) | MEDIUM (Phase 2) |
| cave-opal | OPAL (real-time policy distribution) | HIGH (Phase 2) |
| cave-hubble | Cilium Hubble (network observability) | MEDIUM (Phase 1) |
| cave-arc | ARC (GitHub Actions runners) | LOW (Phase 3) |
| cave-renovate | Renovate (dependency updates) | LOW (Phase 3) |
| cave-vcluster | vcluster (virtual clusters) | HIGH (Phase 1) |
| cave-crossplane | Crossplane v2 (XRD composition) | HIGH (Phase 1) |
| cave-k6 | k6 (load testing) | LOW (Phase 3) |
| cave-gitea | Gitea HA (tenant Git hosting) | MEDIUM (Phase 2) |
| cave-spark | Spark Operator (data processing) | LOW (Phase 4) |
| cave-jupyter | JupyterHub (notebooks) | LOW (Phase 4) |
| cave-mlflow | MLflow (experiment tracking) | LOW (Phase 4) |
| cave-databricks | Databricks wrapper (Azure path) | LOW (Phase 4) |
| cave-knative | Knative (serverless, Phase 4) | DEFERRED |
| cave-keda | KEDA (autoscaling + Reflex triggers) | MEDIUM (Phase 3) |

---

## 5. Stubs Needing Substantial Work (20)

These crates exist but have <1K lines — they need to grow 5-20x to reach functional parity with their upstream:

| Crate | Current | Target | Reimplements | Key Missing Functionality |
|-------|---------|--------|-------------|--------------------------|
| cave-pam | 353 | ~5,000 | Teleport/CyberArk | Session recording, JIT access, break-glass, audit |
| cave-forensics | 210 | ~4,000 | Tetragon | eBPF kernel hooks, syscall tracing, WORM export |
| cave-sign | 183 | ~3,000 | Sigstore cosign | Keyless signing, SLSA provenance, verification |
| cave-dast | 304 | ~3,000 | ZAP | HTTP fuzzing, OpenAPI scanning, auth testing |
| cave-scan | 392 | ~3,000 | Trivy/SonarQube | Image scanning, SAST, CVE correlation |
| cave-sbom | 434 | ~3,000 | DependencyTrack | CycloneDX parse, NVD/OSV lookup, policy |
| cave-vulns | 384 | ~3,000 | DefectDojo | Finding management, dedup, JIRA integration |
| cave-chaos | 420 | ~3,000 | Chaos Mesh | Fault injection, network chaos, SLO-aware |
| cave-workflows | 501 | ~4,000 | n8n/Argo Workflows | DAG execution, credential handling, triggers |
| cave-pii | 204 | ~2,500 | Presidio | NER patterns, anonymization, language support |
| cave-chat | 206 | ~3,000 | LibreChat | Multi-provider chat, conversation store |
| cave-ai-obs | 201 | ~2,500 | Langfuse | LLM trace capture, evaluation, prompt mgmt |
| cave-devlake | 205 | ~2,500 | DevLake | DORA metrics, data connectors |
| cave-uptime | 299 | ~2,000 | Uptime Kuma | HTTP/TCP/DNS monitors, notification |
| cave-incidents | 452 | ~3,000 | Grafana OnCall | Escalation, schedules, routing, PagerDuty API |
| cave-profiler | 180 | ~2,500 | Pyroscope | eBPF profiling, pprof format, flame graphs |
| cave-slo | 409 | ~2,500 | (native) | SLO budget, burn rate alerts, error budget |
| cave-admission | 981 | ~3,000 | Sigstore Policy Controller | Image signature verify, mutation, audit |
| cave-secrets | 374 | ~2,500 | Trufflehog | Pattern detection, entropy analysis, verification |
| cave-lint | 291 | ~2,000 | Hadolint | Dockerfile AST, rule engine, best practices |

---

## 6. Upstream Tracker Gap

`cave-upstream/src/projects.rs` currently tracks **26** projects. Need to expand to **73** to cover every component. Missing from tracker:

**Need to add (47 more):**
Talos Linux, AKS, CloudNativePG, Azure PG Flexible, MinIO, ADLS Gen2, Strimzi, Confluent Cloud, Valkey, Azure Redis, OpenSearch, Azure AI Search, Qdrant, Keycloak, Okta (API changes), Entra ID, Kong, Istio ambient, Cilium, CoreDNS, Prometheus, Grafana, Loki, Tempo, Thanos, ArgoCD, Harbor, Argo Rollouts, ARC, Renovate, Pulp (already tracked), OPAL, OpenTelemetry, Spark Operator, JupyterHub, MLflow, Databricks, Backstage (already tracked), Gitea, vcluster, OpenCost (already tracked), k6, Crossplane, Knative, KEDA, Velero (already tracked), and CyberArk.

---

## 7. Architecture Principle Implementation Status

(Updated with sovereign reimplementation lens)

| # | Principle | Status | Notes |
|---|----------|--------|-------|
| 1 | Single Pane of Glass | PARTIAL | cave-portal + cave-dashboard exist, need catalog |
| 2 | Three Control Planes | PARTIAL | cave-cli exists, need MCP Server + Emergency CLI |
| 3 | Profile-Driven | NEW | cave-core/profile.rs just added (621 lines) |
| 4 | Two-Layer Provisioning | NOT IMPL | Need cave-crossplane + cave-infra expansion |
| 5 | Crossplane-first | NOT IMPL | Need cave-crossplane crate |
| 6 | GitOps Everything | PARTIAL | cave-gitops-config + cave-deploy exist |
| 7 | Security by Default | PARTIAL | cave-security, cave-vault, cave-policy strong; signing weak |
| 8 | Multi-Tenant | NEW | cave-core/tenant.rs just added (261 lines) |
| 9 | Self-Hosted AI | PARTIAL | cave-llm-gateway exists, PII/classification weak |
| 10 | Full Observability | GOOD | metrics/logs/trace/dashboard all substantial |
| 11 | Policy-as-Code | GOOD | cave-policy 11.6K lines, need OPAL |
| 12 | SLO-driven FinOps | PARTIAL | cave-cost + cave-cost-alloc exist, need kill switch |
| 13 | Immutable Infra | NOT IMPL | Need Talos API in cave-cluster |
| 14 | Sovereign Audit | NEW | cave-ledger just added (821 lines) |
| 15 | APOL | NOT IMPL | Phase 3 — AI agent framework |
| 16 | Automated Remediation | NOT IMPL | Need cave-keda + workflow engine |
| 17 | Exit Strategy | NOT IMPL | Need portability drill in cave-cli |
| 18 | ADRs | NOT IMPL | 0/130+ ADR docs written |

---

## 8. Total Codebase Status

| Metric | Current | Target |
|--------|---------|--------|
| Total crates | 67 | ~84 (67 + 17 new) |
| Total Rust lines | ~178,000 | ~350,000+ |
| GOOD crates (>4K) | 14 | 50+ |
| Upstream projects tracked | 26 | 73 |
| OSS tools with functional reimplementation | ~14 | 73 |
| ADR documents | 0 | 130+ |
| Runbook sections | 8 | 44 |
