// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Registry of all 73 upstream OSS projects tracked by cave-runtime.
//!
//! Each component in the CAVE platform documentation has a corresponding
//! upstream project that we track for API/protocol compatibility.
//! The cave-runtime reimplements each project's functionality in Rust+eBPF.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProject {
    pub name: &'static str,
    pub github_repo: &'static str,
    pub cave_module: &'static str,
    pub track_features: &'static str,
    pub check_frequency: &'static str,
    /// Component category from One-Prompt
    pub category: &'static str,
    /// Phase in CAVE rollout (1=Core, 2=Data/AI, 3=Advanced, 4=Extensions)
    pub phase: u8,
}

/// All 108 upstream projects tracked by cave-runtime.
pub const TRACKED_PROJECTS: &[TrackedProject] = &[
    // ============================================================
    // KUBERNETES CORE (reimplemented as cave-* crates)
    // ============================================================
    TrackedProject {
        name: "containerd",
        github_repo: "containerd/containerd",
        cave_module: "cave-cri",
        track_features: "container lifecycle, OCI images, namespaces, cgroups",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "etcd",
        github_repo: "etcd-io/etcd",
        cave_module: "cave-etcd",
        track_features: "KV store, MVCC, watch, leases, transactions",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "kube-apiserver",
        github_repo: "kubernetes/kubernetes",
        cave_module: "cave-apiserver",
        track_features: "resource CRUD, admission, RBAC, watch/list",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "kube-scheduler",
        github_repo: "kubernetes/kubernetes",
        cave_module: "cave-scheduler",
        track_features: "pod scheduling, filter/score/bind, affinity, taints",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "kubelet",
        github_repo: "kubernetes/kubernetes",
        cave_module: "cave-kubelet",
        track_features: "pod lifecycle, node status, container management",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "kube-controller-manager",
        github_repo: "kubernetes/kubernetes",
        cave_module: "cave-controller-manager",
        track_features: "Deployment, ReplicaSet, StatefulSet, DaemonSet, Job, CronJob, HPA, PDB, EndpointSlice, GC, NodeLifecycle, SA, CSR, RBAC, PV",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "cloud-controller-manager",
        github_repo: "kubernetes/cloud-provider",
        cave_module: "cave-cloud-controller-manager",
        track_features: "CloudProvider trait, Node/Service/Route controllers, Hetzner + Azure providers",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    // ============================================================
    // PROVIDER-ABSTRACTED: Kubernetes
    // ============================================================

    // ============================================================
    // PROVIDER-ABSTRACTED: Database
    // ============================================================
    TrackedProject {
        name: "CloudNativePG",
        github_repo: "cloudnative-pg/cloudnative-pg",
        cave_module: "cave-rdbms-operator",
        track_features: "Cluster CRD spec, Barman backup integration, connection pooling, PVCResize, tablespace mgmt",
        check_frequency: "biweekly",
        category: "database",
        phase: 2,
    },
    // ============================================================
    // PROVIDER-ABSTRACTED: Object Storage
    // ============================================================
    TrackedProject {
        name: "MinIO",
        github_repo: "minio/minio",
        cave_module: "cave-store",
        track_features: "S3 API compatibility, object locking (WORM), erasure coding, replication, IAM policies",
        check_frequency: "biweekly",
        category: "storage",
        phase: 2,
    },
    // ============================================================
    // PROVIDER-ABSTRACTED: Event Streaming
    // ============================================================
    TrackedProject {
        name: "Strimzi",
        github_repo: "strimzi/strimzi-kafka-operator",
        cave_module: "cave-streams",
        track_features: "Kafka protocol versions, KRaft mode, CRD evolution, connector framework",
        check_frequency: "biweekly",
        category: "messaging",
        phase: 2,
    },
    TrackedProject {
        name: "Apache Kafka",
        github_repo: "apache/kafka",
        cave_module: "cave-streams",
        track_features: "Protocol spec changes, KIP proposals, consumer group protocol, transactions",
        check_frequency: "monthly",
        category: "messaging",
        phase: 2,
    },
    // ============================================================
    // PROVIDER-ABSTRACTED: Cache
    // ============================================================
    TrackedProject {
        name: "Valkey",
        github_repo: "valkey-io/valkey",
        cave_module: "cave-cache",
        track_features: "RESP protocol changes, new data structures, cluster protocol, module API",
        check_frequency: "biweekly",
        category: "cache",
        phase: 2,
    },
    // ============================================================
    // PROVIDER-ABSTRACTED: Search
    // ============================================================
    TrackedProject {
        name: "OpenSearch",
        github_repo: "opensearch-project/OpenSearch",
        cave_module: "cave-search",
        track_features: "Query DSL evolution, index management, plugin API, security model",
        check_frequency: "monthly",
        category: "search",
        phase: 2,
    },
    // Phantom: target name `cave-vector-search` has no workspace member.
    // The OpenSearch/Qdrant/Faiss/Milvus split-out exists in tracker but
    // workspace has only `cave-search/` (handles all four together).
    // Re-enable when crates/cave-vector-search/ exists. See naming-audit-2026-05-03.md §1.

    // ============================================================
    // IDENTITY & SECRETS
    // ============================================================
    TrackedProject {
        name: "Keycloak",
        github_repo: "keycloak/keycloak",
        cave_module: "cave-auth",
        track_features: "OIDC spec compliance, SCIM 2.0, realm export format, admin REST API, organization model",
        check_frequency: "biweekly",
        category: "identity",
        phase: 1,
    },
    TrackedProject {
        name: "OpenBao",
        github_repo: "openbao/openbao",
        cave_module: "cave-vault",
        track_features: "Secret engine API, transit encryption, PKI cert issuance, dynamic credentials, audit format",
        check_frequency: "biweekly",
        category: "secrets",
        phase: 1,
    },
    TrackedProject {
        name: "External Secrets Operator",
        github_repo: "external-secrets/external-secrets",
        cave_module: "cave-vault",
        track_features: "Provider spec, SecretStore CRD, push secret, refresh intervals",
        check_frequency: "monthly",
        category: "secrets",
        phase: 1,
    },
    // ============================================================
    // NETWORKING & GATEWAY
    // ============================================================
    TrackedProject {
        name: "Kong",
        github_repo: "Kong/kong",
        cave_module: "cave-gateway",
        track_features: "Plugin API, rate limiting algorithms, OpenAPI validation, JWT/OAuth2, admin API spec",
        check_frequency: "biweekly",
        category: "networking",
        phase: 1,
    },
    TrackedProject {
        name: "Istio",
        github_repo: "istio/istio",
        cave_module: "cave-mesh",
        track_features: "Ambient mode (ztunnel, waypoint), SPIFFE identity, AuthorizationPolicy, traffic management API",
        check_frequency: "biweekly",
        category: "networking",
        phase: 1,
    },
    TrackedProject {
        name: "Cilium",
        github_repo: "cilium/cilium",
        // 2026-05-13: was `cave-ebpf-common` — a 188-LOC shared eBPF
        // types crate with a skeleton parity manifest (every [[files]]
        // / [[functions]] / [[tests]] / [[surfaces]] block commented
        // out). The kernel parity calculator therefore returned
        // overall=0.0 for it, and the /upstream tracker page rendered
        // "Cilium 0%" — Burak's report. The actual Cilium port lives
        // in cave-net (36k LOC, full mapping inventory, fill_ratio =
        // 0.9179 per its parity.manifest.toml). Remap to the real one.
        cave_module: "cave-net",
        track_features: "eBPF programs, CiliumNetworkPolicy CRD, egress gateway, kube-proxy replacement, bandwidth mgr, Hubble flow visibility, agent state machines",
        check_frequency: "biweekly",
        category: "networking",
        phase: 1,
    },
    TrackedProject {
        name: "Cilium Hubble",
        github_repo: "cilium/hubble",
        cave_module: "cave-forensics",
        track_features: "Flow API, L7 visibility, service map, metrics export, relay protocol",
        check_frequency: "monthly",
        category: "networking",
        phase: 1,
    },
    TrackedProject {
        name: "CoreDNS",
        github_repo: "coredns/coredns",
        cave_module: "cave-dns",
        track_features: "Plugin chain, zone file format, service discovery integration, DNS-over-HTTPS",
        check_frequency: "monthly",
        category: "networking",
        phase: 1,
    },
    // ============================================================
    // OBSERVABILITY
    // ============================================================
    TrackedProject {
        name: "Prometheus",
        github_repo: "prometheus/prometheus",
        cave_module: "cave-metrics",
        track_features: "PromQL spec, remote write/read API, TSDB format, recording rules, alerting rules",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Grafana",
        github_repo: "grafana/grafana",
        cave_module: "cave-dashboard",
        track_features: "Dashboard JSON model, panel plugins, data source API, provisioning format, alerting",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Loki",
        github_repo: "grafana/loki",
        cave_module: "cave-logs",
        track_features: "LogQL spec, push API (protobuf), chunk format, label indexing, retention rules",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Tempo",
        github_repo: "grafana/tempo",
        cave_module: "cave-trace",
        track_features: "OTLP ingest, TraceQL, Parquet backend, trace-to-metrics, span format",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Thanos",
        github_repo: "thanos-io/thanos",
        cave_module: "cave-metrics",
        track_features: "Store API, compaction, downsampling, query federation, sidecar protocol",
        check_frequency: "monthly",
        category: "observability",
        phase: 2,
    },
    TrackedProject {
        name: "OpenTelemetry Collector",
        github_repo: "open-telemetry/opentelemetry-collector",
        cave_module: "cave-trace",
        track_features: "OTLP spec, receiver/processor/exporter API, configuration format",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Grafana OnCall",
        github_repo: "grafana/oncall",
        cave_module: "cave-incidents",
        track_features: "Escalation logic, integration methods, schedule format, API spec",
        check_frequency: "monthly",
        category: "observability",
        phase: 3,
    },
    // ============================================================
    // GITOPS & CI/CD
    // ============================================================
    TrackedProject {
        name: "ArgoCD",
        github_repo: "argoproj/argo-cd",
        cave_module: "cave-deploy",
        track_features: "Application CRD, ApplicationSet, sync waves, server-side apply, OCI support, v3.x changes",
        check_frequency: "biweekly",
        category: "gitops",
        phase: 1,
    },
    TrackedProject {
        name: "Harbor",
        github_repo: "goharbor/harbor",
        cave_module: "cave-registry",
        track_features: "OCI distribution spec, content trust (Notary v2), replication, vulnerability scanning hooks",
        check_frequency: "monthly",
        category: "gitops",
        phase: 1,
    },
    TrackedProject {
        name: "Argo Rollouts",
        github_repo: "argoproj/argo-rollouts",
        cave_module: "cave-rollouts",
        track_features: "Canary/blue-green strategies, analysis templates, traffic management, Istio integration",
        check_frequency: "monthly",
        category: "gitops",
        phase: 1,
    },
    TrackedProject {
        name: "Argo Workflows",
        github_repo: "argoproj/argo-workflows",
        cave_module: "cave-workflows",
        track_features: "DAG spec, template types, artifact passing, retry/backoff, cron workflows",
        check_frequency: "monthly",
        category: "gitops",
        phase: 3,
    },
    TrackedProject {
        name: "Pulp",
        github_repo: "pulp/pulpcore",
        cave_module: "cave-registry",
        track_features: "New repository types, content plugin API, remote sync",
        check_frequency: "monthly",
        category: "gitops",
        phase: 2,
    },
    // ============================================================
    // SECURITY
    // ============================================================
    TrackedProject {
        name: "OPA Gatekeeper",
        github_repo: "open-policy-agent/gatekeeper",
        cave_module: "cave-policy",
        track_features: "Constraint framework changes, audit improvements, mutation support, external data",
        check_frequency: "monthly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "OPA (Open Policy Agent)",
        github_repo: "open-policy-agent/opa",
        cave_module: "cave-policy",
        track_features: "Rego language spec, built-in functions, Wasm compilation, bundle format",
        check_frequency: "biweekly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "OPAL",
        github_repo: "permitio/opal",
        cave_module: "cave-policy",
        track_features: "Policy/data update protocol, external data sources, pub-sub model",
        check_frequency: "monthly",
        category: "security",
        phase: 2,
    },
    TrackedProject {
        name: "Sigstore cosign",
        github_repo: "sigstore/cosign",
        cave_module: "cave-sign",
        track_features: "Signing formats, keyless workflow, SLSA provenance spec, verification API",
        check_frequency: "monthly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "Sigstore Policy Controller",
        github_repo: "sigstore/policy-controller",
        cave_module: "cave-admission",
        track_features: "ClusterImagePolicy CRD, verification modes, attestation types",
        check_frequency: "monthly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "DefectDojo",
        github_repo: "DefectDojo/django-DefectDojo",
        cave_module: "cave-vulns",
        track_features: "Importer formats, JIRA integration patterns, finding dedup logic",
        check_frequency: "monthly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "DependencyTrack",
        github_repo: "DependencyTrack/dependency-track",
        cave_module: "cave-sbom",
        track_features: "NVD/OSV data format, CycloneDX spec, policy engine",
        check_frequency: "biweekly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "Trivy",
        github_repo: "aquasecurity/trivy",
        cave_module: "cave-scan",
        track_features: "Scanner plugins, vulnerability DB format, SBOM output, misconfiguration rules",
        check_frequency: "biweekly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "ZAP",
        github_repo: "zaproxy/zaproxy",
        cave_module: "cave-dast",
        track_features: "Scan rules, API scanning, authentication methods",
        check_frequency: "monthly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "Tetragon",
        github_repo: "cilium/tetragon",
        cave_module: "cave-forensics",
        track_features: "eBPF hook types, TracingPolicy CRD, export formats, syscall tracing",
        check_frequency: "biweekly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "SonarQube",
        github_repo: "SonarSource/sonarqube",
        cave_module: "cave-scan",
        track_features: "New rules per language, quality profile changes, CE vs EE gap",
        check_frequency: "monthly",
        category: "security",
        phase: 3,
    },
    // ============================================================
    // AI / LLM
    // ============================================================
    TrackedProject {
        name: "LiteLLM",
        github_repo: "BerriAI/litellm",
        cave_module: "cave-llm-gateway",
        track_features: "Provider adapters, routing strategies, budget management, model fallbacks",
        check_frequency: "weekly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Ollama",
        github_repo: "ollama/ollama",
        cave_module: "cave-llm-gateway",
        track_features: "Model format, API spec, GPU scheduling, quantization methods",
        check_frequency: "weekly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Presidio",
        github_repo: "microsoft/presidio",
        cave_module: "cave-pii",
        track_features: "New recognizer patterns, language support, anonymization strategies",
        check_frequency: "monthly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "LibreChat",
        github_repo: "danny-avila/LibreChat",
        cave_module: "cave-chat",
        track_features: "Provider integration patterns, conversation schema, plugin system",
        check_frequency: "monthly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Langfuse",
        github_repo: "langfuse/langfuse",
        cave_module: "cave-ai-obs",
        track_features: "Trace schema evolution, evaluation framework, prompt management API",
        check_frequency: "biweekly",
        category: "ai",
        phase: 2,
    },
    // ============================================================
    // DATA PLATFORM
    // ============================================================
    TrackedProject {
        name: "MLflow",
        github_repo: "mlflow/mlflow",
        cave_module: "cave-ai-obs",
        track_features: "Tracking API, model registry, experiment schema, deployment targets",
        check_frequency: "monthly",
        category: "data-platform",
        phase: 4,
    },
    // ============================================================
    // DEVELOPER EXPERIENCE
    // ============================================================
    TrackedProject {
        name: "Backstage",
        github_repo: "backstage/backstage",
        cave_module: "cave-portal",
        track_features: "Catalog spec, scaffolder actions, plugin API, search architecture, Declarative Integration",
        check_frequency: "weekly",
        category: "devex",
        phase: 1,
    },
    TrackedProject {
        name: "Unleash",
        github_repo: "Unleash/unleash",
        cave_module: "cave-flags",
        track_features: "Feature types, strategies, metrics API, client SDK protocol",
        check_frequency: "monthly",
        category: "devex",
        phase: 1,
    },
    TrackedProject {
        name: "Gitea",
        github_repo: "go-gitea/gitea",
        cave_module: "cave-registry",
        track_features: "Git protocol, API v1, webhook format, LFS, container registry, HA clustering",
        check_frequency: "monthly",
        category: "devex",
        phase: 2,
    },
    TrackedProject {
        name: "Apicurio Registry",
        github_repo: "Apicurio/apicurio-registry",
        cave_module: "cave-docs",
        track_features: "Schema format support, compatibility rules, API changes",
        check_frequency: "monthly",
        category: "devex",
        phase: 2,
    },
    // ============================================================
    // PLATFORM OPERATIONS
    // ============================================================
    TrackedProject {
        name: "Chaos Mesh",
        github_repo: "chaos-mesh/chaos-mesh",
        cave_module: "cave-chaos",
        track_features: "New chaos types, schedule model, status reporting, PhysicalMachine support",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "Velero",
        github_repo: "vmware-tanzu/velero",
        cave_module: "cave-backup",
        track_features: "Backup/restore improvements, CSI snapshot integration, schedule model, data mover",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "OpenCost",
        github_repo: "opencost/opencost",
        cave_module: "cave-cost",
        track_features: "Cost model changes, cloud pricing API updates, allocation logic, plugins",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "DevLake",
        github_repo: "apache/incubator-devlake",
        cave_module: "cave-devlake",
        track_features: "DORA metric calculation changes, new data source plugins, domain model",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "Uptime Kuma",
        github_repo: "louislam/uptime-kuma",
        cave_module: "cave-uptime",
        track_features: "New monitor types, notification methods, maintenance windows",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    // loft-sh/vcluster intentionally NOT tracked: Charter decision — `cave-cluster`
    // multi-tenant control-plane uses clastix/kamaji, not vcluster. `cave-kamaji`
    // is tracked separately (LATEST status). See version-audit-2026-05-02.md DROPPED
    // section (lands with feat/cave-upstream-watchd-001 branch).
    TrackedProject {
        name: "k6",
        github_repo: "grafana/k6",
        cave_module: "cave-slo",
        track_features: "JavaScript runtime, check/threshold API, extension system, cloud output",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "Pyroscope",
        github_repo: "grafana/pyroscope",
        cave_module: "cave-profiler",
        track_features: "Profile formats, language support, eBPF profiling methods, pprof spec",
        check_frequency: "monthly",
        category: "operations",
        phase: 3,
    },
    TrackedProject {
        name: "cert-manager",
        github_repo: "cert-manager/cert-manager",
        cave_module: "cave-certs",
        track_features: "ACME protocol changes, new issuers, CRD evolution, trust-manager",
        check_frequency: "monthly",
        category: "operations",
        phase: 1,
    },
    // ============================================================
    // CROSSPLANE & INFRASTRUCTURE
    // ============================================================
    TrackedProject {
        // 2026-05-27: cave-infra's parity.manifest.toml declares hashicorp/terraform
        // as its upstream — the legacy mapping to crossplane/crossplane was a
        // duplicate of the cave-crossplane entry below and tripped
        // test_unique_project_names. Crossplane stays tracked via the
        // cave-crossplane entry below.
        name: "HashiCorp Terraform",
        github_repo: "hashicorp/terraform",
        cave_module: "cave-infra",
        track_features: "HCL parser, provider plugin protocol, state backend, plan/apply graph, modules",
        check_frequency: "monthly",
        category: "infrastructure",
        phase: 1,
    },
    // ============================================================
    // PHASE 4: EXTENSIONS (opt-in)
    // ============================================================
    TrackedProject {
        name: "Knative",
        github_repo: "knative/serving",
        cave_module: "cave-deploy",
        track_features: "Serving API, autoscaling, revision management, traffic splitting",
        check_frequency: "monthly",
        category: "serverless",
        phase: 4,
    },
    TrackedProject {
        name: "KEDA",
        github_repo: "kedacore/keda",
        cave_module: "cave-ha",
        track_features: "ScaledObject/ScaledJob API, scaler types, metrics server, Reflex Engine triggers",
        check_frequency: "monthly",
        category: "serverless",
        phase: 3,
    },
    TrackedProject {
        name: "Karpenter",
        github_repo: "kubernetes-sigs/karpenter",
        cave_module: "cave-karpenter",
        track_features: "NodePool/NodeClaim/NodeClass v1 CRDs, scheduler, consolidation, drift detection, disruption budgets",
        check_frequency: "monthly",
        category: "node-autoscaling",
        phase: 3,
    },
    TrackedProject {
        name: "KubeVirt",
        github_repo: "kubevirt/kubevirt",
        cave_module: "cave-kubevirt",
        track_features: "VirtualMachine/VirtualMachineInstance/DataVolume CRDs, virt-launcher Pod, live migration, instancetype/preference, CDI",
        check_frequency: "monthly",
        category: "virtualization",
        phase: 4,
    },
    // ============================================================
    // LAKEHOUSE (consolidated per ADR-147 — N upstreams → 1 cave-module)
    // ============================================================
    TrackedProject {
        name: "Apache Iceberg",
        github_repo: "apache/iceberg-rust",
        cave_module: "cave-lakehouse",
        track_features: "Table format — Schema, PartitionSpec, Manifest, Snapshot, TableMetadata, time-travel",
        check_frequency: "biweekly",
        category: "lakehouse",
        phase: 2,
    },
    TrackedProject {
        name: "Apache DataFusion",
        github_repo: "apache/datafusion",
        cave_module: "cave-lakehouse",
        track_features: "Query engine — SQL planner, DataFrame, vectorized executor, LogicalPlan/PhysicalPlan",
        check_frequency: "biweekly",
        category: "lakehouse",
        phase: 2,
    },
    // ============================================================
    // BUSINESS APPLICATIONS (CRM, ERP — standalone modules)
    // ============================================================
    // Twenty is the standalone CRM upstream (ADR-145). cave-crm is a
    // function-based crate (ADR-147 naming pattern), independent from
    // cave-erp's CRM submodule which is being deprecated.
    // Latest stable at scaffold time: v2.2.0 (2026-05-04).
    TrackedProject {
        name: "Twenty",
        github_repo: "twentyhq/twenty",
        cave_module: "cave-crm",
        track_features: "Person/Company/Opportunity/Activity data model, GraphQL + REST API, custom objects, workflow automation, AGPL-3.0 license",
        check_frequency: "biweekly",
        category: "crm",
        phase: 4,
    },
    // ============================================================
    // CHARTER v2 merge wave 2026-05-23/24 — TRACKED_PROJECTS backfill
    // (parity.manifest.toml landed on main but projects.rs was not
    //  edited at close-time; entries added so /api/upstream/tracker
    //  surfaces these crates.)
    // ============================================================
    TrackedProject {
        name: "kube-bench",
        github_repo: "aquasecurity/kube-bench",
        cave_module: "cave-bench",
        track_features: "CIS benchmark master/node/etcd/control-plane checks, YAML rule loader, test operators, JSON/SARIF report",
        check_frequency: "biweekly",
        category: "security",
        phase: 2,
    },
    TrackedProject {
        name: "kubescape",
        github_repo: "kubescape/kubescape",
        cave_module: "cave-bench",
        track_features: "NSA + MITRE ATT&CK for K8s control catalogue, manifest facts evaluator, scan runner modes",
        check_frequency: "biweekly",
        category: "security",
        phase: 2,
    },
    TrackedProject {
        name: "SPIRE",
        github_repo: "spiffe/spire",
        cave_module: "cave-identity",
        track_features: "SPIFFE ID/SVID issuance, workload attestation, X.509 + JWT SVID, federation trust bundle, registration API",
        check_frequency: "biweekly",
        category: "identity",
        phase: 2,
    },
    TrackedProject {
        name: "gVisor",
        github_repo: "google/gvisor",
        cave_module: "cave-sandbox",
        track_features: "User-space kernel (runsc), syscall interception, OCI runtime, platform=ptrace/kvm, seccomp profile",
        check_frequency: "biweekly",
        category: "virtualization",
        phase: 2,
    },
    TrackedProject {
        name: "Kata Containers",
        github_repo: "kata-containers/kata-containers",
        cave_module: "cave-sandbox",
        track_features: "VM-isolated containers, kata-agent, hypervisor drivers (qemu/cloud-hypervisor/firecracker), OCI runtime, snapshot",
        check_frequency: "biweekly",
        category: "virtualization",
        phase: 2,
    },
    TrackedProject {
        name: "Firecracker",
        github_repo: "firecracker-microvm/firecracker",
        cave_module: "cave-sandbox",
        track_features: "microVM hypervisor, jailer, KVM API, vsock, balloon, snapshot/restore, REST control plane",
        check_frequency: "biweekly",
        category: "virtualization",
        phase: 2,
    },
    TrackedProject {
        name: "Knative Serving",
        github_repo: "knative/serving",
        cave_module: "cave-knative",
        track_features: "Service/Configuration/Revision/Route CRDs, KPA autoscaler (stable+panic), TrafficTarget, RevisionTemplateSpec",
        check_frequency: "biweekly",
        category: "serverless",
        phase: 2,
    },
    TrackedProject {
        name: "Knative Eventing",
        github_repo: "knative/eventing",
        cave_module: "cave-knative",
        track_features: "Broker/Trigger/Channel/Subscription CRDs, CloudEvents 1.0 spec, in-memory channel, Argo Events sources",
        check_frequency: "biweekly",
        category: "serverless",
        phase: 2,
    },
    TrackedProject {
        name: "Crossplane",
        github_repo: "crossplane/crossplane",
        cave_module: "cave-crossplane",
        track_features: "XRD + Composition v2 pipeline (FunctionRef), XR/Claim lifecycle, Provider/Function/Configuration packages, ProviderConfig + DeploymentRuntime",
        check_frequency: "biweekly",
        category: "infrastructure",
        phase: 2,
    },
    // ============================================================
    // BACKFILL 2026-05-27 (portal-v3 ray) — workspace crates that
    // ship a [upstream] block in parity.manifest.toml but had no
    // entry here. Names are qualified when an existing entry already
    // claims the upstream's canonical name, so test_unique_project_names
    // stays green.
    // ============================================================
    TrackedProject {
        name: "Prometheus Alertmanager",
        github_repo: "prometheus/alertmanager",
        cave_module: "cave-alerts",
        track_features: "alert routing, silences, inhibitions, deduplication, receivers, HA gossip",
        check_frequency: "biweekly",
        category: "observability",
        phase: 1,
    },
    TrackedProject {
        name: "Pulp (artifacts)",
        github_repo: "pulp/pulpcore",
        cave_module: "cave-artifacts",
        track_features: "content unit + distribution + repository version + publication + remote sync",
        check_frequency: "biweekly",
        category: "gitops",
        phase: 2,
    },
    TrackedProject {
        name: "Debezium Server",
        github_repo: "debezium/debezium-server",
        cave_module: "cave-cdc",
        track_features: "CDC source connectors (Postgres/MySQL/Mongo), Kafka/Kinesis/Pulsar sinks, schema history",
        check_frequency: "biweekly",
        category: "data-platform",
        phase: 2,
    },
    TrackedProject {
        name: "Cluster API",
        github_repo: "kubernetes-sigs/cluster-api",
        cave_module: "cave-cluster",
        track_features: "Cluster/Machine/MachineDeployment/MachineSet/KCP, bootstrap+control-plane providers, ClusterClass",
        check_frequency: "biweekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "OPA Gatekeeper (compliance)",
        github_repo: "open-policy-agent/gatekeeper",
        cave_module: "cave-compliance",
        track_features: "ConstraintTemplate, audit report, mutation, expansion, sync set, validating-admission",
        check_frequency: "biweekly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "Trivy (container scan)",
        github_repo: "aquasecurity/trivy",
        cave_module: "cave-container-scan",
        track_features: "image/fs/repo/k8s scan, SBOM gen, OS+lang pkg detection, secret + misconfig",
        check_frequency: "biweekly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "Apache DataFusion (engine)",
        github_repo: "apache/datafusion",
        cave_module: "cave-datafusion",
        track_features: "logical+physical planner, expression rewriter, parquet/csv/json IO, vectorized execution",
        check_frequency: "biweekly",
        category: "lakehouse",
        phase: 2,
    },
    TrackedProject {
        name: "FerretDB",
        github_repo: "FerretDB/FerretDB",
        cave_module: "cave-docdb",
        track_features: "MongoDB wire protocol on Postgres, aggregation pipeline, BSON codec, replica state",
        check_frequency: "biweekly",
        category: "database",
        phase: 2,
    },
    TrackedProject {
        name: "ERPNext",
        github_repo: "frappe/erpnext",
        cave_module: "cave-erp",
        track_features: "sales/purchase/stock/accounts/HR DocTypes, journal entries, multi-currency",
        check_frequency: "monthly",
        category: "crm",
        phase: 4,
    },
    TrackedProject {
        name: "Falco",
        github_repo: "falcosecurity/falco",
        cave_module: "cave-falco",
        track_features: "rule engine, syscall events, k8s audit, plugin framework, gRPC outputs",
        check_frequency: "biweekly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "Gitleaks",
        github_repo: "gitleaks/gitleaks",
        cave_module: "cave-gitleaks",
        track_features: "regex+entropy secret scan, git history walk, allowlist + baseline, SARIF output",
        check_frequency: "biweekly",
        category: "security",
        phase: 2,
    },
    TrackedProject {
        name: "Argo CD (gitops config)",
        github_repo: "argoproj/argo-cd",
        cave_module: "cave-gitops-config",
        track_features: "Application CRD config surface, AppProject RBAC, sync windows, ApplicationSet generator",
        check_frequency: "biweekly",
        category: "gitops",
        phase: 1,
    },
    TrackedProject {
        name: "Hermes Agent",
        github_repo: "NousResearch/hermes-agent",
        cave_module: "cave-hermes",
        track_features: "memory manager, tool router, persona prompt, conversation graph",
        check_frequency: "weekly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Apache Iceberg (rust engine)",
        github_repo: "apache/iceberg-rust",
        cave_module: "cave-iceberg",
        track_features: "table metadata v2/v3, snapshot+manifest, scan builder, REST/Memory catalog, partition transforms",
        check_frequency: "biweekly",
        category: "lakehouse",
        phase: 2,
    },
    TrackedProject {
        name: "Kamaji",
        github_repo: "clastix-labs/kamaji",
        cave_module: "cave-kamaji",
        track_features: "TenantControlPlane CRD, multi-tenant Postgres+etcd backend, kine adapter, datastore lifecycle",
        check_frequency: "biweekly",
        category: "Kubernetes Core",
        phase: 2,
    },
    TrackedProject {
        name: "KEDA (operator)",
        github_repo: "kedacore/keda",
        cave_module: "cave-keda",
        track_features: "ScaledObject/ScaledJob CRDs, 60+ scalers, HPA bridge, ClusterTriggerAuthentication",
        check_frequency: "biweekly",
        category: "operations",
        phase: 2,
    },
    TrackedProject {
        name: "kube-proxy",
        github_repo: "kubernetes/kubernetes",
        cave_module: "cave-kube-proxy",
        track_features: "Service VIP, iptables/IPVS/nftables backends, EndpointSlice consumer, conntrack",
        check_frequency: "weekly",
        category: "Kubernetes Core",
        phase: 1,
    },
    TrackedProject {
        name: "Cave LLM Tracker",
        github_repo: "cave-runtime/cave-llm-tracker",
        cave_module: "cave-llm-tracker",
        track_features: "HF/Ollama/LMSys/vLLM/llama.cpp/MLX-LM trending + license aggregation, daily verdicts",
        check_frequency: "daily",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Ollama (local)",
        github_repo: "ollama/ollama",
        cave_module: "cave-local-llm",
        track_features: "local inference daemon, modelfile, REST API, GGUF runtime",
        check_frequency: "biweekly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "MLX",
        github_repo: "ml-explore/mlx",
        cave_module: "cave-mlx",
        track_features: "N-dim array, broadcasting ops, matmul, conv1d/conv2d + pooling, reverse-mode autograd, nn modules (Linear/Conv2d), SGD/Adam/AdamW optimizers (CPU backend)",
        check_frequency: "biweekly",
        category: "ai",
        phase: 2,
    },
    TrackedProject {
        name: "Grafana OnCall (engine)",
        github_repo: "grafana/oncall",
        cave_module: "cave-oncall",
        track_features: "alert group/integration/escalation chain, schedules, on-call shifts, ChatOps webhook",
        check_frequency: "biweekly",
        category: "observability",
        phase: 3,
    },
    TrackedProject {
        name: "Teleport",
        github_repo: "gravitational/teleport",
        cave_module: "cave-pam",
        track_features: "PAM agent, session recording, RBAC roles, certificate authority, access requests",
        check_frequency: "biweekly",
        category: "identity",
        phase: 2,
    },
    TrackedProject {
        name: "Casbin",
        github_repo: "casbin/casbin",
        cave_module: "cave-permission",
        track_features: "ACL/RBAC/ABAC model parser, policy adapter, enforcer, watcher",
        check_frequency: "biweekly",
        category: "identity",
        phase: 2,
    },
    TrackedProject {
        name: "Tekton Pipelines",
        github_repo: "tektoncd/pipeline",
        cave_module: "cave-pipelines",
        track_features: "Task/Pipeline/PipelineRun/TaskRun CRDs, results, workspaces, sidecars, custom tasks",
        check_frequency: "biweekly",
        category: "gitops",
        phase: 2,
    },
    TrackedProject {
        name: "PostgreSQL",
        github_repo: "postgres/postgres",
        cave_module: "cave-rdbms",
        track_features: "MVCC, WAL replication, parser+planner+executor, vacuum, replication slots",
        check_frequency: "monthly",
        category: "database",
        phase: 1,
    },
    TrackedProject {
        name: "Trivy DB",
        github_repo: "aquasecurity/trivy-db",
        cave_module: "cave-scan-db",
        track_features: "vuln DB OCI layout, bbolt store, advisory ingest (alpine/debian/redhat/etc.), data signing",
        check_frequency: "biweekly",
        category: "security",
        phase: 3,
    },
    TrackedProject {
        name: "TruffleHog",
        github_repo: "trufflesecurity/trufflehog",
        cave_module: "cave-secrets",
        track_features: "700+ detectors, git/s3/docker/github sources, live verification, decoder framework",
        check_frequency: "biweekly",
        category: "security",
        phase: 2,
    },
    TrackedProject {
        name: "Falco (security facade)",
        github_repo: "falcosecurity/falco",
        cave_module: "cave-security",
        track_features: "rule engine facade, syscall+k8s_audit feeds, alert outputs",
        check_frequency: "biweekly",
        category: "security",
        phase: 1,
    },
    TrackedProject {
        name: "Plane",
        github_repo: "makeplane/plane",
        cave_module: "cave-tracker",
        track_features: "workspaces, projects, cycles, modules, issues, comments, integrations",
        check_frequency: "biweekly",
        category: "devex",
        phase: 3,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_tracked_projects_count() {
        assert!(
            TRACKED_PROJECTS.len() >= 60,
            "Expected at least 55 tracked projects, got {}",
            TRACKED_PROJECTS.len()
        );
    }

    #[test]
    fn test_all_projects_have_names() {
        for project in TRACKED_PROJECTS {
            assert!(
                !project.name.is_empty(),
                "Project has empty name: {:?}",
                project
            );
        }
    }

    #[test]
    fn test_all_projects_have_github_repos() {
        for project in TRACKED_PROJECTS {
            assert!(
                !project.github_repo.is_empty(),
                "Project {} has empty github_repo",
                project.name
            );
        }
    }

    #[test]
    fn test_all_github_repos_have_slash() {
        for project in TRACKED_PROJECTS {
            assert!(
                project.github_repo.contains('/'),
                "Project {} github_repo '{}' missing slash (expected org/repo format)",
                project.name,
                project.github_repo
            );
        }
    }

    #[test]
    fn test_all_projects_have_module() {
        for project in TRACKED_PROJECTS {
            assert!(
                !project.cave_module.is_empty(),
                "Project {} has empty cave_module",
                project.name
            );
        }
    }

    #[test]
    fn test_all_projects_have_category() {
        for project in TRACKED_PROJECTS {
            assert!(
                !project.category.is_empty(),
                "Project {} has empty category",
                project.name
            );
        }
    }

    #[test]
    fn test_all_projects_have_valid_phase() {
        for project in TRACKED_PROJECTS {
            assert!(
                (1..=4).contains(&project.phase),
                "Project {} has invalid phase {}",
                project.name,
                project.phase
            );
        }
    }

    #[test]
    fn test_unique_project_names() {
        let mut seen = HashSet::new();
        for project in TRACKED_PROJECTS {
            assert!(
                seen.insert(project.name),
                "Duplicate project name '{}' found",
                project.name
            );
        }
    }

    #[test]
    fn cilium_project_maps_to_cave_net_not_cave_ebpf_common() {
        // Regression for the 2026-05-13 "Cilium 0%" bug: previously
        // mapped to cave-ebpf-common (a 188-LOC shared types crate
        // with a skeleton manifest, kernel parity = 0.0). The actual
        // Cilium port lives in cave-net (36k LOC, fill_ratio = 0.9179).
        let cilium = TRACKED_PROJECTS
            .iter()
            .find(|p| p.name == "Cilium" && p.github_repo == "cilium/cilium")
            .expect("Cilium tracked project must exist");
        assert_eq!(
            cilium.cave_module, "cave-net",
            "Cilium must map to cave-net (where the parity manifest declares cilium/cilium); \
             cave-ebpf-common is a shared types crate, not a port"
        );
    }

    #[test]
    fn test_core_projects_present() {
        let names: HashSet<&str> = TRACKED_PROJECTS.iter().map(|p| p.name).collect();
        let core = [
            "Prometheus",
            "Grafana",
            "Loki",
            "Tempo",
            "Kong",
            "Istio",
            "Cilium",
            "ArgoCD",
            "Keycloak",
            "OpenBao",
            "OPA Gatekeeper",
            "Crossplane",
            "Backstage",
            "Valkey",
            "MinIO",
        ];
        for name in &core {
            assert!(names.contains(name), "Core project '{}' not tracked", name);
        }
    }

    #[test]
    fn test_phase_distribution() {
        let mut phase_counts = [0u32; 5];
        for project in TRACKED_PROJECTS {
            phase_counts[project.phase as usize] += 1;
        }
        assert!(phase_counts[1] >= 20, "Phase 1 should have >= 20 projects");
        assert!(phase_counts[2] >= 15, "Phase 2 should have >= 15 projects");
        assert!(phase_counts[3] >= 15, "Phase 3 should have >= 15 projects");
    }
}
