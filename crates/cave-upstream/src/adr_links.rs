// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ADR ↔ upstream-project mapping.
//!
//! For every tracked upstream project we record the ADR(s) that justify the
//! choice (e.g. ADR-027 for Kong → cave-gateway, ADR-004 for Cilium →
//! cave-net). The portal Upstream Tracker page renders the ADR column from
//! this map and lets the user click through to the ADR markdown.
//!
//! Mapping is keyed by `TrackedProject::github_repo` so a single ADR can cover
//! several upstream entries (e.g. ADR-021 covers both Strimzi and Kafka).
//!
//! Entries with no ADR yet (long-tail integrations awaiting a documented
//! decision) return an empty slice — `adrs_for(repo).is_empty()` lets the UI
//! render an "—" instead of pretending coverage exists.

/// Static map: GitHub repo (`owner/repo`) → list of ADR ids that justify it.
///
/// IDs are matched against ADR filenames in `docs/adr/`. The portal resolves
/// each id to the first ADR file whose name starts with `<id>_` (or `<id>-`).
pub const ADR_LINKS: &[(&str, &[&str])] = &[
    // ── Kubernetes core ────────────────────────────────────────────────────
    ("containerd/containerd", &["ADR-016"]),
    (
        "etcd-io/etcd",
        &["ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001"],
    ),
    (
        "kubernetes/kubernetes",
        &["ADR-RUNTIME-STACK-001", "ADR-003"],
    ),
    ("kubernetes/cloud-provider", &["ADR-RUNTIME-STACK-001"]),
    // ── Networking / mesh / gateway ────────────────────────────────────────
    ("cilium/cilium", &["ADR-004", "ADR-014"]),
    ("cilium/hubble", &["ADR-004"]),
    ("cilium/tetragon", &["ADR-016"]),
    ("istio/istio", &["ADR-004"]),
    (
        "Kong/kong",
        &["ADR-027", "ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001"],
    ),
    ("coredns/coredns", &["ADR-024"]),
    // ── Auth / identity / secrets ──────────────────────────────────────────
    (
        "keycloak/keycloak",
        &["ADR-006", "ADR-PORTAL-AUTH-001", "ADR-PORTAL-PERSONAS-001"],
    ),
    ("openbao/openbao", &["ADR-020"]),
    ("external-secrets/external-secrets", &["ADR-020"]),
    ("open-policy-agent/opa", &["ADR-014"]),
    ("open-policy-agent/gatekeeper", &["ADR-014"]),
    ("permitio/opal", &["ADR-014"]),
    // ── Data layer ────────────────────────────────────────────────────────
    (
        "cloudnative-pg/cloudnative-pg",
        &["ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001"],
    ),
    ("minio/minio", &["ADR-DATA-LAKE-PATTERN-001"]),
    (
        "strimzi/strimzi-kafka-operator",
        &["ADR-021", "ADR-RUNTIME-STREAMING-CONSOLIDATION-001"],
    ),
    (
        "apache/kafka",
        &["ADR-021", "ADR-RUNTIME-STREAMING-CONSOLIDATION-001"],
    ),
    ("valkey-io/valkey", &["ADR-008"]),
    ("opensearch-project/OpenSearch", &[]),
    ("qdrant/qdrant", &["ADR-009"]),
    // ── Multi-tenancy / scheduling ────────────────────────────────────────
    ("loft-sh/vcluster", &["ADR-012", "ADR-MULTI-TENANT-001"]),
    ("kedacore/keda", &["ADR-033"]),
    ("knative/serving", &[]),
    // ── GitOps / pipelines / IaC ──────────────────────────────────────────
    ("argoproj/argo-cd", &["ADR-026"]),
    ("argoproj/argo-rollouts", &["ADR-026"]),
    ("argoproj/argo-workflows", &["ADR-026"]),
    ("crossplane/crossplane", &["ADR-026"]),
    // ── Container build / registry ────────────────────────────────────────
    ("goharbor/harbor", &["ADR-028"]),
    ("pulp/pulpcore", &["ADR-028"]),
    ("Apicurio/apicurio-registry", &["ADR-028"]),
    // ── Observability ────────────────────────────────────────────────────
    ("prometheus/prometheus", &["ADR-029"]),
    ("grafana/grafana", &["ADR-029"]),
    ("grafana/loki", &["ADR-029"]),
    ("grafana/tempo", &["ADR-029"]),
    ("grafana/pyroscope", &["ADR-029"]),
    ("grafana/oncall", &["ADR-029"]),
    ("grafana/k6", &["ADR-029"]),
    ("thanos-io/thanos", &["ADR-029"]),
    ("open-telemetry/opentelemetry-collector", &["ADR-029"]),
    // ── Security / scanning ──────────────────────────────────────────────
    ("aquasecurity/trivy", &["ADR-018"]),
    ("SonarSource/sonarqube", &["ADR-019"]),
    ("zaproxy/zaproxy", &["ADR-023"]),
    ("DefectDojo/django-DefectDojo", &["ADR-019"]),
    ("DependencyTrack/dependency-track", &["ADR-018"]),
    ("sigstore/cosign", &["ADR-018"]),
    ("sigstore/policy-controller", &["ADR-018"]),
    // ── DR / backup / chaos ──────────────────────────────────────────────
    ("vmware-tanzu/velero", &[]),
    ("chaos-mesh/chaos-mesh", &[]),
    (
        "cert-manager/cert-manager",
        &["ADR-015", "ADR-RUNTIME-CERT-LIFECYCLE-001"],
    ),
    // ── Data engineering / analytics ─────────────────────────────────────
    ("apache/incubator-devlake", &[]),
    ("opencost/opencost", &[]),
    ("louislam/uptime-kuma", &[]),
    // ── AI / LLM stack ───────────────────────────────────────────────────
    ("BerriAI/litellm", &["ADR-013"]),
    ("ollama/ollama", &["ADR-009"]),
    ("danny-avila/LibreChat", &["ADR-013"]),
    ("langfuse/langfuse", &["ADR-013"]),
    ("mlflow/mlflow", &[]),
    ("microsoft/presidio", &[]),
    // ── Developer portal ─────────────────────────────────────────────────
    ("backstage/backstage", &["ADR-011", "ADR-025"]),
    ("Unleash/unleash", &[]),
    ("go-gitea/gitea", &[]),
    // ── CDC ─────────────────────────────────────────────────────────────
    // (Debezium not in TRACKED_PROJECTS yet; ADR-022 covers it for future)
];

/// Look up the ADR ids associated with the given upstream `github_repo`.
///
/// Returns an empty slice when no mapping has been recorded yet — callers
/// must treat this as "no documented decision" rather than an error.
pub fn adrs_for(github_repo: &str) -> &'static [&'static str] {
    for (repo, adrs) in ADR_LINKS {
        if *repo == github_repo {
            return adrs;
        }
    }
    &[]
}

/// Reverse index: collect every upstream `github_repo` that lists `adr_id`.
pub fn upstreams_for(adr_id: &str) -> Vec<&'static str> {
    let mut hits = Vec::new();
    for (repo, adrs) in ADR_LINKS {
        if adrs.iter().any(|a| *a == adr_id) {
            hits.push(*repo);
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cilium_resolves_to_adr_004() {
        let adrs = adrs_for("cilium/cilium");
        assert!(adrs.contains(&"ADR-004"), "got {adrs:?}");
    }

    #[test]
    fn kong_resolves_to_adr_027() {
        let adrs = adrs_for("Kong/kong");
        assert!(adrs.contains(&"ADR-027"));
    }

    #[test]
    fn keycloak_lists_persona_adr() {
        let adrs = adrs_for("keycloak/keycloak");
        assert!(adrs.iter().any(|a| a.contains("PORTAL-PERSONAS")));
    }

    #[test]
    fn unmapped_returns_empty() {
        assert_eq!(adrs_for("does/not-exist"), &[] as &[&str]);
    }

    #[test]
    fn reverse_lookup_works() {
        let cilium_for_004 = upstreams_for("ADR-004");
        assert!(cilium_for_004.contains(&"cilium/cilium"));
        assert!(cilium_for_004.contains(&"istio/istio"));
    }

    #[test]
    fn every_listed_upstream_exists_in_tracked_projects() {
        // Sanity: ensure no repo typo — every key in ADR_LINKS must match
        // a TrackedProject github_repo (otherwise the ADR column points at
        // a project that doesn't exist).
        use crate::TRACKED_PROJECTS;
        let known: std::collections::HashSet<&str> =
            TRACKED_PROJECTS.iter().map(|p| p.github_repo).collect();
        for (repo, _) in ADR_LINKS {
            assert!(
                known.contains(*repo),
                "ADR_LINKS references unknown upstream {repo}"
            );
        }
    }
}
