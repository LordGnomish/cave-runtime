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

/// Parse a *pure* version tag (`v?` + dotted decimals, nothing else) into
/// its numeric components. Rejects component-prefixed tags such as
/// `sdk/v2.9.1`, `python-0.4.0`, `kafka-0.7.2`, `knative-v1.22.0`,
/// `RELEASE.2025-…` and pre-release suffixes like `1.2.0-rc1` — exactly
/// the noise that pollutes a repo's raw tag list.
fn semver_core(tag: &str) -> Option<Vec<u64>> {
    let core = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    if core.is_empty() {
        return None;
    }
    let parts: Vec<u64> = core
        .split('.')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    (!parts.is_empty()).then_some(parts)
}

/// From a repo's raw tag list, pick the highest *clean* semver tag,
/// returning it verbatim (original `v`/casing preserved). `None` when no
/// tag is a pure version — the caller then keeps the first-tag fallback.
///
/// This is what turns a `tags`-only repo's drift from junk (`apache/kafka`
/// → `show`, `twentyhq/twenty` → `sdk/v2.9.1`) into the real latest
/// release (`4.3.0`, `v2.9.0`).
pub fn pick_latest_semver_tag(tags: &[String]) -> Option<String> {
    tags.iter()
        .filter_map(|t| semver_core(t).map(|core| (core, t)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, t)| t.clone())
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
        // 2. tags — repos with git tags but no GitHub release objects
        //    (apache/kafka, apache/datafusion, twentyhq/twenty, …). Pull a
        //    page and pick the highest *clean* semver tag; the raw list is
        //    not version-ordered and is polluted with component tags
        //    (`sdk/v…`, `python-…`, `kafka-0.7.…`).
        let tags_url = format!("{}/repos/{}/tags?per_page=100", self.api_base, repo);
        if let Some(body) = self.get_json(&tags_url).await
            && let Some(arr) = body.as_array()
        {
            let names: Vec<String> = arr
                .iter()
                .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
                .collect();
            // Prefer the best semver tag; fall back to the first listed.
            if let Some(best) = pick_latest_semver_tag(&names) {
                return Some(best);
            }
            if let Some(first) = names.into_iter().next() {
                return Some(first);
            }
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
    let mut registry = vec![
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
    ];
    apply_curated_pins(&mut registry);
    registry
}

/// Ported-version baselines, keyed by `org/repo`, sourced from each
/// crate's `parity.manifest.toml` `[upstream] version` (the upstream
/// tag we line-ported against). These are the real pins that turn the
/// daily report from all-`unknown` into an honest in-sync/behind delta.
///
/// Only repos whose `cave_module` genuinely tracks that exact upstream
/// are listed — repos the registry maps to a *different* upstream than
/// the manifest, or that are pinned to a moving `main`, are deliberately
/// left unpinned (reported `unknown`) rather than pinned to a guess.
///
/// Snapshot: 2026-06-07. Re-sync when a crate's manifest version bumps.
pub const CURATED_PINS: &[(&str, &str)] = &[
    ("kubernetes/kubernetes", "v1.36.0"),
    ("containerd/containerd", "v2.2.3"),
    ("etcd-io/etcd", "v3.6.10"),
    ("clastix/kamaji", "v1.0.0"),
    ("kubevirt/kubevirt", "v1.8.2"),
    ("cilium/cilium", "v1.19.3"),
    ("cilium/tetragon", "v1.7.0"),
    ("coredns/coredns", "v1.14.3"),
    ("istio/istio", "1.30.0"),
    ("Kong/kong", "3.9.1"),
    ("knative/serving", "knative-v1.22.0"),
    ("argoproj/argo-cd", "v3.4.2"),
    ("argoproj/argo-rollouts", "v1.9.0"),
    ("argoproj/argo-workflows", "v4.0.5"),
    ("tektoncd/pipeline", "v0.55.0"),
    ("crossplane/crossplane", "v2.3.1"),
    ("kedacore/keda", "v2.16.1"),
    ("kubernetes-sigs/karpenter", "v1.4.0"),
    ("cert-manager/cert-manager", "v1.17.2"),
    ("aquasecurity/trivy", "v0.70.0"),
    ("DependencyTrack/dependency-track", "v4.11.6"),
    ("open-policy-agent/opa", "v1.16.2"),
    ("open-policy-agent/gatekeeper", "v3.17.1"),
    ("keycloak/keycloak", "v22.0.0"),
    ("spiffe/spire", "v1.15.0"),
    ("openbao/openbao", "v2.5.4"),
    ("falcosecurity/falco", "0.43.1"),
    ("prometheus/prometheus", "v3.3.0"),
    ("grafana/grafana", "v11.5.0"),
    ("grafana/loki", "v3.4.0"),
    ("grafana/pyroscope", "v1.3.0"),
    ("grafana/oncall", "v1.10.0"),
    ("FerretDB/FerretDB", "v2.0.0"),
    ("valkey-io/valkey", "8.0.0"),
    ("minio/minio", "RELEASE.2025-04-22T22-12-26Z"),
    ("apache/kafka", "4.2.0"),
    ("apache/pulsar", "v4.2.0"),
    ("apache/iceberg-rust", "v0.9.1"),
    ("apache/datafusion", "53.1.0"),
    ("BerriAI/litellm", "v1.85.1"),
    ("ollama/ollama", "v0.3.0"),
    ("microsoft/presidio", "v2.2.0"),
    ("langfuse/langfuse", "v3.75.1"),
    ("opencost/opencost", "v1.108.0"),
    ("chaos-mesh/chaos-mesh", "v2.7.0"),
    ("apache/incubator-devlake", "v0.21.1"),
    ("louislam/uptime-kuma", "v1.23.13"),
    ("backstage/backstage", "v1.50.4"),
    ("Unleash/unleash", "v5.0.0"),
    ("twentyhq/twenty", "v2.6.0"),
];

/// Stamp [`CURATED_PINS`] onto every matching registry row. A repo shared
/// by several cave modules (e.g. `kubernetes/kubernetes`) gets the same
/// pin on each of its rows.
fn apply_curated_pins(registry: &mut [Upstream]) {
    for u in registry.iter_mut() {
        if let Some((_, ver)) = CURATED_PINS.iter().find(|(repo, _)| *repo == u.repo) {
            u.pinned = Some((*ver).to_string());
        }
    }
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
    fn curated_pins_land_on_their_rows() {
        let reg = default_registry();
        let pin = |repo: &str| {
            reg.iter()
                .find(|u| u.repo == repo)
                .and_then(|u| u.pinned.clone())
        };
        // A representative pin from each major area, matching the manifests.
        assert_eq!(pin("kubernetes/kubernetes").as_deref(), Some("v1.36.0"));
        assert_eq!(pin("clastix/kamaji").as_deref(), Some("v1.0.0"));
        assert_eq!(pin("cilium/cilium").as_deref(), Some("v1.19.3"));
        assert_eq!(pin("openbao/openbao").as_deref(), Some("v2.5.4"));
        assert_eq!(pin("kedacore/keda").as_deref(), Some("v2.16.1"));
        assert_eq!(pin("kubernetes-sigs/karpenter").as_deref(), Some("v1.4.0"));
        assert_eq!(pin("twentyhq/twenty").as_deref(), Some("v2.6.0"));
        assert_eq!(pin("FerretDB/FerretDB").as_deref(), Some("v2.0.0"));
        assert_eq!(pin("apache/kafka").as_deref(), Some("4.2.0"));
        assert_eq!(pin("apache/pulsar").as_deref(), Some("v4.2.0"));
    }

    #[test]
    fn shared_repo_pins_fan_to_every_row() {
        // All five kubernetes/kubernetes rows carry the same pin.
        let reg = default_registry();
        let k8s: Vec<_> = reg
            .iter()
            .filter(|u| u.repo == "kubernetes/kubernetes")
            .collect();
        assert!(k8s.len() >= 4, "expected several k8s rows");
        assert!(k8s.iter().all(|u| u.pinned.as_deref() == Some("v1.36.0")));
    }

    #[test]
    fn majority_of_registry_is_now_pinned() {
        // Phase 0 cont2 turned the all-unknown report into a real delta:
        // most rows now carry a manifest-sourced baseline.
        let reg = default_registry();
        let pinned = reg.iter().filter(|u| u.pinned.is_some()).count();
        assert!(
            pinned >= 50,
            "expected >=50 pinned rows after cont2, got {pinned}"
        );
    }

    #[test]
    fn every_curated_pin_targets_a_real_registry_repo() {
        // Guard against a pin entry whose repo was renamed/removed from
        // the registry (it would silently never apply).
        let repos: std::collections::BTreeSet<String> =
            default_registry().into_iter().map(|u| u.repo).collect();
        for (repo, _) in CURATED_PINS {
            assert!(
                repos.contains(*repo),
                "curated pin {repo} has no matching registry row"
            );
        }
    }

    #[test]
    fn pick_latest_semver_tag_handles_noisy_lists() {
        // apache/kafka: bare numerics mixed with junk → highest numeric.
        let kafka = vec![
            "show".to_string(),
            "kafka-0.7.2-incubating".to_string(),
            "4.2.0".to_string(),
            "4.3.0".to_string(),
            "4.2.1".to_string(),
        ];
        assert_eq!(pick_latest_semver_tag(&kafka).as_deref(), Some("4.3.0"));

        // twentyhq/twenty: component-prefixed sdk tags must be ignored.
        let twenty = vec![
            "sdk/v2.9.1".to_string(),
            "v2.9.0".to_string(),
            "v2.8.3".to_string(),
        ];
        assert_eq!(pick_latest_semver_tag(&twenty).as_deref(), Some("v2.9.0"));

        // datafusion: numeric tags alongside a python sub-project tag.
        let df = vec!["python-0.4.0".to_string(), "53.1.0".to_string(), "52.5.0".to_string()];
        assert_eq!(pick_latest_semver_tag(&df).as_deref(), Some("53.1.0"));

        // Nothing clean → None (caller keeps its first-tag fallback).
        let messy = vec!["RELEASE.2025-10-15".to_string(), "nightly".to_string()];
        assert_eq!(pick_latest_semver_tag(&messy), None);
    }

    #[test]
    fn semver_core_rejects_prefixed_and_prerelease() {
        assert_eq!(super::semver_core("v1.2.3"), Some(vec![1, 2, 3]));
        assert_eq!(super::semver_core("4.2.0"), Some(vec![4, 2, 0]));
        assert!(super::semver_core("sdk/v2.9.1").is_none());
        assert!(super::semver_core("1.2.0-rc1").is_none());
        assert!(super::semver_core("knative-v1.22.0").is_none());
        assert!(super::semver_core("RELEASE.2025-10-15").is_none());
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
