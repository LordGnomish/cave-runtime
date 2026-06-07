// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Upstream registry + release fetchers.
//!
//! Every cave-* crate reimplements an upstream OSS project. This module
//! holds the canonical list of those upstreams ([`default_registry`]),
//! the per-source release fetcher abstraction ([`ReleaseFetcher`]), and
//! the live GitHub implementation ([`GithubFetcher`]).
//!
//! The registry is also the seed for the default YAML config, so a fresh
//! checkout polls a useful set with zero configuration. Operators extend
//! or pin entries by editing `cave-runtime-tracker.yaml`.

use serde::{Deserialize, Serialize};

/// One tracked upstream project and the cave-* crate that reimplements it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Upstream {
    /// Human-readable project name, e.g. `Cilium`.
    pub name: String,
    /// GitHub `org/repo`, e.g. `cilium/cilium`.
    pub repo: String,
    /// The cave-* crate that ports this upstream, e.g. `cave-net`.
    pub cave_module: String,
    /// Loose category bucket used to group the markdown report.
    pub category: String,
    /// CAVE rollout phase (1=Core, 2=Data/AI, 3=Advanced, 4=Extensions).
    pub phase: u8,
    /// Short note on the surface we track for parity.
    pub track: String,
    /// The upstream tag/version we have currently ported, if known.
    /// `None` means "not pinned" — the report shows drift as `Unknown`
    /// rather than fabricating a baseline.
    #[serde(default)]
    pub pinned: Option<String>,
}

impl Upstream {
    fn new(
        name: &str,
        repo: &str,
        cave_module: &str,
        category: &str,
        phase: u8,
        track: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            repo: repo.to_string(),
            cave_module: cave_module.to_string(),
            category: category.to_string(),
            phase,
            track: track.to_string(),
            pinned: None,
        }
    }
}

/// Drift verdict for one upstream after a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftStatus {
    /// Pinned version equals the latest upstream tag.
    InSync,
    /// Pinned version is set and differs from the latest upstream tag.
    Behind,
    /// Either no pinned baseline or the upstream tag could not be fetched.
    Unknown,
}

impl DriftStatus {
    pub fn badge(self) -> &'static str {
        match self {
            DriftStatus::InSync => "✅ in-sync",
            DriftStatus::Behind => "⚠️ behind",
            DriftStatus::Unknown => "❔ unknown",
        }
    }
}

/// Classify drift from a pinned baseline against the freshly-fetched
/// latest upstream tag. The comparison is exact-string after trimming a
/// leading `v` from each side, so `v1.2.3` and `1.2.3` count as in-sync.
pub fn drift(pinned: Option<&str>, latest: Option<&str>) -> DriftStatus {
    match (pinned, latest) {
        (Some(p), Some(l)) => {
            if norm_tag(p) == norm_tag(l) {
                DriftStatus::InSync
            } else {
                DriftStatus::Behind
            }
        }
        _ => DriftStatus::Unknown,
    }
}

fn norm_tag(tag: &str) -> &str {
    let t = tag.trim();
    t.strip_prefix('v').unwrap_or(t)
}

/// Async release fetcher. Implementors return the latest release/tag for
/// a `org/repo`, or `None` when it cannot be determined (offline, rate
/// limited, no releases). Returning `None` keeps the daily report
/// degrading gracefully rather than aborting the whole run.
#[async_trait::async_trait]
pub trait ReleaseFetcher: Send + Sync {
    async fn latest_release(&self, repo: &str) -> Option<String>;
}

/// Live GitHub fetcher. Hits `releases/latest`, then falls back to the
/// first entry of `tags` for repos that publish git tags but no GitHub
/// "release" objects (e.g. `kubernetes/kubernetes`).
pub struct GithubFetcher {
    pub api_base: String,
    pub client: reqwest::Client,
    pub token: Option<String>,
}

impl GithubFetcher {
    pub fn new(api_base: impl Into<String>, timeout_secs: u64, token: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("cave-runtime-tracker/0.1 (+https://github.com/cave-runtime)")
            .timeout(std::time::Duration::from_secs(timeout_secs.max(1)))
            .build()
            .expect("reqwest client builds with static config");
        Self {
            api_base: api_base.into(),
            client,
            token,
        }
    }

    async fn get_json(&self, url: &str) -> Option<serde_json::Value> {
        let mut req = self.client.get(url).header("Accept", "application/vnd.github+json");
        if let Some(tok) = &self.token {
            req = req.header("Authorization", format!("Bearer {tok}"));
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<serde_json::Value>().await.ok()
    }
}

#[async_trait::async_trait]
impl ReleaseFetcher for GithubFetcher {
    async fn latest_release(&self, repo: &str) -> Option<String> {
        // 1. releases/latest — the common, fast path.
        let rel_url = format!("{}/repos/{}/releases/latest", self.api_base, repo);
        if let Some(body) = self.get_json(&rel_url).await
            && let Some(tag) = body.get("tag_name").and_then(|v| v.as_str())
        {
            return Some(tag.to_string());
        }
        // 2. tags — repos with git tags but no GitHub release objects.
        let tags_url = format!("{}/repos/{}/tags?per_page=1", self.api_base, repo);
        if let Some(body) = self.get_json(&tags_url).await
            && let Some(name) = body
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t.get("name"))
                .and_then(|v| v.as_str())
        {
            return Some(name.to_string());
        }
        None
    }
}

/// The canonical cave-runtime upstream set. Curated to cover every
/// shipping cave-* subsystem plus the multi-tenant / serverless / data
/// upstreams called out in the cave-runtime charter. Operators override
/// or extend this through the YAML config.
pub fn default_registry() -> Vec<Upstream> {
    use Upstream as U;
    vec![
        // ── Kubernetes control plane ──────────────────────────────────
        U::new("kube-apiserver", "kubernetes/kubernetes", "cave-apiserver", "Kubernetes Core", 1, "resource CRUD, admission, RBAC, watch/list"),
        U::new("kube-scheduler", "kubernetes/kubernetes", "cave-scheduler", "Kubernetes Core", 1, "filter/score/bind, affinity, taints"),
        U::new("kubelet", "kubernetes/kubernetes", "cave-kubelet", "Kubernetes Core", 1, "pod lifecycle, node status, CSI/CNI"),
        U::new("kube-controller-manager", "kubernetes/kubernetes", "cave-controller-manager", "Kubernetes Core", 1, "Deployment/RS/STS/DS, HPA, GC, SA, CSR, PV"),
        U::new("cloud-controller-manager", "kubernetes/cloud-provider", "cave-cloud-controller-manager", "Kubernetes Core", 1, "Node/Service/Route controllers"),
        U::new("kube-proxy", "kubernetes/kubernetes", "cave-kube-proxy", "Kubernetes Core", 1, "Service VIPs, iptables/ipvs datapath"),
        U::new("containerd", "containerd/containerd", "cave-cri", "Kubernetes Core", 1, "container lifecycle, OCI images, cgroups"),
        U::new("etcd", "etcd-io/etcd", "cave-etcd", "Kubernetes Core", 1, "KV store, MVCC, watch, leases, raft"),
        U::new("Kamaji", "clastix/kamaji", "cave-kamaji", "Kubernetes Core", 2, "multi-tenant hosted control planes"),
        U::new("KubeVirt", "kubevirt/kubevirt", "cave-kubevirt", "virtualization", 4, "VM-as-pod, virt-launcher, live migration"),
        // ── Networking / mesh ─────────────────────────────────────────
        U::new("Cilium", "cilium/cilium", "cave-net", "networking", 1, "eBPF CNI, LB, NetworkPolicy, DSR"),
        U::new("Cilium Hubble", "cilium/hubble", "cave-forensics", "networking", 2, "L3-L7 flow observability"),
        U::new("Tetragon", "cilium/tetragon", "cave-forensics", "security", 3, "eBPF runtime enforcement"),
        U::new("CoreDNS", "coredns/coredns", "cave-dns", "networking", 1, "cluster DNS, plugins, zone files"),
        U::new("Istio", "istio/istio", "cave-mesh", "networking", 1, "sidecar/ambient mesh, mTLS, traffic policy"),
        U::new("Linkerd", "linkerd/linkerd2", "cave-mesh", "networking", 2, "lightweight mTLS service mesh"),
        U::new("Kong", "Kong/kong", "cave-gateway", "networking", 1, "API gateway, plugins, Gateway API"),
        U::new("Knative Serving", "knative/serving", "cave-knative", "serverless", 4, "scale-to-zero, revisions, KPA autoscaling"),
        U::new("Knative Eventing", "knative/eventing", "cave-knative", "serverless", 4, "brokers, triggers, event sources"),
        U::new("Knative func", "knative/func", "cave-knative", "serverless", 4, "function build/deploy templates"),
        // ── GitOps / delivery ─────────────────────────────────────────
        U::new("ArgoCD", "argoproj/argo-cd", "cave-deploy", "gitops", 1, "app sync, sync windows, helm/kustomize"),
        U::new("Argo Rollouts", "argoproj/argo-rollouts", "cave-rollouts", "gitops", 1, "canary/blue-green, analysis"),
        U::new("Argo Workflows", "argoproj/argo-workflows", "cave-workflows", "gitops", 3, "DAG/steps, cron, semaphore/mutex"),
        U::new("Argo Events", "argoproj/argo-events", "cave-events", "gitops", 3, "event sources, sensors, triggers"),
        U::new("Knative + Tekton (Pipelines)", "tektoncd/pipeline", "cave-pipelines", "gitops", 3, "Task/Pipeline/PipelineRun"),
        U::new("Crossplane", "crossplane/crossplane", "cave-crossplane", "infrastructure", 1, "XRD/Composition, claims, providers"),
        U::new("KEDA", "kedacore/keda", "cave-keda", "serverless", 3, "event-driven autoscaling, scalers, ScaledObject"),
        U::new("Karpenter", "kubernetes-sigs/karpenter", "cave-karpenter", "node-autoscaling", 3, "just-in-time nodes, consolidation"),
        U::new("cert-manager", "cert-manager/cert-manager", "cave-certs", "operations", 1, "ACME, issuers, certificate lifecycle"),
        // ── Registry / supply chain ───────────────────────────────────
        U::new("Harbor", "goharbor/harbor", "cave-registry", "registry", 1, "OCI registry, replication, scanning"),
        U::new("Pulp", "pulp/pulpcore", "cave-registry", "registry", 2, "content repositories, publications"),
        U::new("Buildah", "containers/buildah", "cave-scaffold", "build", 2, "OCI image build without daemon"),
        U::new("Sigstore cosign", "sigstore/cosign", "cave-sign", "security", 1, "artifact signing, verification, attestations"),
        U::new("Sigstore policy-controller", "sigstore/policy-controller", "cave-admission", "security", 1, "admission-time signature policy"),
        U::new("Trivy", "aquasecurity/trivy", "cave-scan", "security", 3, "image/fs/IaC vulnerability scanning"),
        U::new("DependencyTrack", "DependencyTrack/dependency-track", "cave-sbom", "security", 3, "SBOM ingest, component analysis"),
        // ── Policy / identity / secrets ───────────────────────────────
        U::new("OPA", "open-policy-agent/opa", "cave-policy", "security", 1, "Rego eval, bundles, decisions"),
        U::new("OPA Gatekeeper", "open-policy-agent/gatekeeper", "cave-policy", "security", 1, "ConstraintTemplate, audit"),
        U::new("Kyverno", "kyverno/kyverno", "cave-policy", "security", 2, "policy-as-yaml, mutate/validate/generate"),
        U::new("Keycloak", "keycloak/keycloak", "cave-auth", "identity", 1, "OIDC/SAML, realms, brokering"),
        U::new("SPIFFE/SPIRE", "spiffe/spire", "cave-identity", "identity", 2, "workload identity, SVIDs, attestation"),
        U::new("OpenBao", "openbao/openbao", "cave-vault", "secrets", 1, "KV/transit/PKI, dynamic secrets"),
        U::new("External Secrets Operator", "external-secrets/external-secrets", "cave-vault", "secrets", 1, "external secret sync"),
        U::new("Sealed Secrets", "bitnami-labs/sealed-secrets", "cave-vault", "secrets", 1, "asymmetric-encrypted secrets in git"),
        U::new("Falco", "falcosecurity/falco", "cave-falco", "security", 2, "syscall rules, runtime threat detection"),
        // ── Observability ─────────────────────────────────────────────
        U::new("Prometheus", "prometheus/prometheus", "cave-metrics", "observability", 1, "TSDB, PromQL, scrape, alerting rules"),
        U::new("prometheus-operator", "prometheus-operator/prometheus-operator", "cave-metrics", "observability", 1, "ServiceMonitor/PrometheusRule CRDs"),
        U::new("Thanos", "thanos-io/thanos", "cave-metrics", "observability", 2, "long-term storage, global query, dedup"),
        U::new("Cortex", "cortexproject/cortex", "cave-metrics", "observability", 2, "multi-tenant horizontally-scalable Prometheus"),
        U::new("Grafana", "grafana/grafana", "cave-dashboard", "observability", 1, "dashboards, panels, datasources"),
        U::new("Loki", "grafana/loki", "cave-logs", "observability", 1, "log aggregation, LogQL, query scheduler"),
        U::new("Tempo", "grafana/tempo", "cave-trace", "observability", 1, "trace storage, TraceQL, spanmetrics"),
        U::new("OpenTelemetry Collector", "open-telemetry/opentelemetry-collector", "cave-tracing", "observability", 1, "receivers/processors/exporters"),
        U::new("Pyroscope", "grafana/pyroscope", "cave-profiler", "operations", 3, "continuous profiling"),
        U::new("Grafana OnCall", "grafana/oncall", "cave-incidents", "observability", 3, "on-call schedules, escalation"),
        U::new("k6", "grafana/k6", "cave-slo", "operations", 3, "load testing, thresholds"),
        // ── Data / storage / streaming ────────────────────────────────
        U::new("CloudNativePG", "cloudnative-pg/cloudnative-pg", "cave-rdbms-operator", "database", 2, "Postgres operator, failover, backup"),
        U::new("FerretDB", "FerretDB/FerretDB", "cave-docdb", "database", 2, "MongoDB-compatible document DB"),
        U::new("Valkey", "valkey-io/valkey", "cave-cache", "cache", 2, "in-memory KV, replication"),
        U::new("MinIO", "minio/minio", "cave-store", "storage", 2, "S3-compatible object storage"),
        U::new("OpenSearch", "opensearch-project/OpenSearch", "cave-search", "search", 2, "inverted index, query DSL"),
        U::new("Apache Kafka", "apache/kafka", "cave-streams", "messaging", 2, "log segments, partitions, consumer groups"),
        U::new("Apache Pulsar", "apache/pulsar", "cave-streams", "messaging", 2, "topics, subscriptions, tiered storage"),
        U::new("Strimzi", "strimzi/strimzi-kafka-operator", "cave-streams", "messaging", 2, "Kafka operator CRDs"),
        U::new("Apache Iceberg", "apache/iceberg-rust", "cave-iceberg", "lakehouse", 2, "table format, snapshots, REST catalog"),
        U::new("Apache DataFusion", "apache/datafusion", "cave-datafusion", "lakehouse", 2, "SQL planner, physical execution"),
        // ── AI / LLM ──────────────────────────────────────────────────
        U::new("LiteLLM", "BerriAI/litellm", "cave-llm-gateway", "ai", 2, "provider routing, budgets, rerank/embeddings"),
        U::new("Ollama", "ollama/ollama", "cave-local-llm", "ai", 2, "local model serving, GGUF, templates"),
        U::new("vLLM", "vllm-project/vllm", "cave-local-llm", "ai", 2, "paged-attention serving, sampler, spec decode"),
        U::new("MLX", "ml-explore/mlx", "cave-mlx", "ai", 2, "Apple-silicon array framework, RNG"),
        U::new("Hermes (agent)", "NousResearch/Hermes-Function-Calling", "cave-hermes", "ai", 3, "function-calling agent loop"),
        U::new("Presidio", "microsoft/presidio", "cave-pii", "ai", 2, "PII detection/anonymization"),
        U::new("Langfuse", "langfuse/langfuse", "cave-ai-obs", "ai", 2, "LLM tracing/eval"),
        // ── Operations / platform ─────────────────────────────────────
        U::new("OpenCost", "opencost/opencost", "cave-cost", "operations", 3, "k8s cost allocation, cloud rates"),
        U::new("Velero", "vmware-tanzu/velero", "cave-backup", "operations", 3, "cluster backup/restore, volume snapshots"),
        U::new("Chaos Mesh", "chaos-mesh/chaos-mesh", "cave-chaos", "operations", 3, "fault injection, schedules"),
        U::new("DevLake", "apache/incubator-devlake", "cave-devlake", "operations", 3, "DORA metrics, data collection"),
        U::new("Uptime Kuma", "louislam/uptime-kuma", "cave-uptime", "operations", 3, "uptime monitors, status pages"),
        U::new("Backstage", "backstage/backstage", "cave-portal", "devex", 1, "service catalog, software templates"),
        U::new("Unleash", "Unleash/unleash", "cave-flags", "devex", 1, "feature flags, strategies"),
        U::new("Twenty", "twentyhq/twenty", "cave-crm", "crm", 4, "CRM objects, pipelines, workflows"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_is_comprehensive() {
        let r = default_registry();
        // The charter calls for ~80 subsystems; floor the count well
        // above the "handful" smoke level so an accidental truncation
        // trips the test.
        assert!(r.len() >= 70, "registry shrank to {}", r.len());
    }

    #[test]
    fn every_repo_is_org_slash_repo() {
        for u in default_registry() {
            assert!(
                u.repo.contains('/') && !u.repo.starts_with('/') && !u.repo.ends_with('/'),
                "bad repo path: {}",
                u.repo
            );
            assert!(!u.name.is_empty());
            assert!(u.cave_module.starts_with("cave-"), "module {}", u.cave_module);
            assert!((1..=4).contains(&u.phase), "phase {}", u.phase);
        }
    }

    #[test]
    fn registry_covers_charter_callouts() {
        let repos: Vec<String> = default_registry().into_iter().map(|u| u.repo).collect();
        for must in [
            "kubernetes/kubernetes",
            "clastix/kamaji",
            "knative/serving",
            "argoproj/argo-events",
            "cilium/cilium",
            "kedacore/keda",
            "FerretDB/FerretDB",
            "cortexproject/cortex",
            "thanos-io/thanos",
            "twentyhq/twenty",
            "apache/pulsar",
        ] {
            assert!(repos.iter().any(|r| r == must), "missing charter repo {must}");
        }
    }

    #[test]
    fn upstream_roundtrips_through_yaml() {
        let u = &default_registry()[0];
        let s = serde_yaml::to_string(u).unwrap();
        let back: Upstream = serde_yaml::from_str(&s).unwrap();
        assert_eq!(u, &back);
    }

    #[test]
    fn drift_normalises_v_prefix() {
        assert_eq!(drift(Some("v1.2.3"), Some("1.2.3")), DriftStatus::InSync);
        assert_eq!(drift(Some("1.2.3"), Some("v1.2.4")), DriftStatus::Behind);
        assert_eq!(drift(None, Some("v1.2.3")), DriftStatus::Unknown);
        assert_eq!(drift(Some("1.2.3"), None), DriftStatus::Unknown);
    }

    #[test]
    fn drift_badges_are_distinct() {
        let b: Vec<&str> = [DriftStatus::InSync, DriftStatus::Behind, DriftStatus::Unknown]
            .iter()
            .map(|d| d.badge())
            .collect();
        assert_eq!(b.len(), 3);
        assert_ne!(b[0], b[1]);
        assert_ne!(b[1], b[2]);
    }
}
