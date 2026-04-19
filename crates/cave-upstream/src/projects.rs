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

/// All 61 upstream projects tracked by cave-runtime.
pub const TRACKED_PROJECTS: &[TrackedProject] = &[
    // ============================================================
    // PROVIDER-ABSTRACTED: Kubernetes
    // ============================================================

    // ============================================================
    // PROVIDER-ABSTRACTED: Database
    // ============================================================
    TrackedProject {
        name: "CloudNativePG",
        github_repo: "cloudnative-pg/cloudnative-pg",
        cave_module: "cave-pg",
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
    TrackedProject {
        name: "Qdrant",
        github_repo: "qdrant/qdrant",
        cave_module: "cave-vector-search",
        track_features: "HNSW/IVFFlat indexing, gRPC API, quantization, multi-vector, hybrid search",
        check_frequency: "biweekly",
        category: "search",
        phase: 2,
    },

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
        cave_module: "cave-ebpf-common",
        track_features: "eBPF programs, CiliumNetworkPolicy CRD, egress gateway, kube-proxy replacement, bandwidth mgr",
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
    TrackedProject {
        name: "vcluster",
        github_repo: "loft-sh/vcluster",
        cave_module: "cave-cluster",
        track_features: "Syncer protocol, resource sync config, persistent vs ephemeral modes, pro features going OSS",
        check_frequency: "biweekly",
        category: "operations",
        phase: 1,
    },
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
        name: "Crossplane",
        github_repo: "crossplane/crossplane",
        cave_module: "cave-infra",
        track_features: "v2 XR API, Composition Functions, MRAP, CronOperation, WatchOperation, namespace-first",
        check_frequency: "biweekly",
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
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_tracked_projects_count() {
        assert!(
            TRACKED_PROJECTS.len() >= 55,
            "Expected at least 55 tracked projects, got {}",
            TRACKED_PROJECTS.len()
        );
    }

    #[test]
    fn test_all_projects_have_names() {
        for project in TRACKED_PROJECTS {
            assert!(!project.name.is_empty(), "Project has empty name: {:?}", project);
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
    fn test_core_projects_present() {
        let names: HashSet<&str> = TRACKED_PROJECTS.iter().map(|p| p.name).collect();
        let core = [
            "Prometheus", "Grafana", "Loki", "Tempo", "Kong", "Istio",
            "Cilium", "ArgoCD", "Keycloak", "OpenBao", "OPA Gatekeeper",
            "Crossplane", "Backstage", "Valkey", "MinIO",
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
