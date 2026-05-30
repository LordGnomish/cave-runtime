// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire-controller-manager (Apache-2.0). The
// ClusterSPIFFEID / ClusterFederatedTrustDomain CRD reconciliation logic is
// line-ported from spire-controller-manager (api/v1alpha1 + the controllers/
// reconcilers). The Kubernetes watch/informer wiring (CRD registration,
// controller-runtime manager) is fed externally by cave-cri — cave-identity
// owns the pure CRD-spec → SPIRE-object mapping, consistent with how the
// k8s workload attestor consumes an externally-populated K8sPodInfo table.
//
//! spire-controller-manager: CRD-driven registration.
//!
//! Two CRDs are reconciled:
//!   * `ClusterSPIFFEID` — selects pods by namespace/pod label selectors and
//!     renders a [`RegistrationEntry`] per matching pod from a Go-template
//!     SPIFFE-ID + DNS-name + workload-selector set.
//!   * `ClusterFederatedTrustDomain` — maps to a
//!     [`crate::models::FederationRelationship`].

use crate::error::{IdentityError, Result};
use crate::models::{
    BundleEndpointProfile, FederationRelationship, RegistrationEntry, Selector, SpiffeId,
    TrustDomain,
};
use crate::spiffe_id::{parse_spiffe_id, validate_trust_domain};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// spire-controller-manager default `spiffeIDTemplate` when none is supplied
/// (`config.SPIFFEIDTemplate` default —
/// `spiffe://{trustDomain}/ns/{namespace}/sa/{serviceAccount}`).
pub const DEFAULT_SPIFFE_ID_TEMPLATE: &str =
    "spiffe://{{ .TrustDomain }}/ns/{{ .PodMeta.Namespace }}/sa/{{ .PodSpec.ServiceAccountName }}";

// ─── Kubernetes LabelSelector (metav1.LabelSelector) ─────────────────────────

/// `metav1.LabelSelectorOperator`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelSelectorOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

/// `metav1.LabelSelectorRequirement`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: LabelSelectorOperator,
    pub values: Vec<String>,
}

/// `metav1.LabelSelector` — `matchLabels` AND every `matchExpressions` entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelSelector {
    pub match_labels: BTreeMap<String, String>,
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

/// Evaluate a Kubernetes LabelSelector against a label set
/// (`k8s.io/apimachinery labels.Selector.Matches` semantics).
pub fn label_selector_matches(sel: &LabelSelector, labels: &BTreeMap<String, String>) -> bool {
    // matchLabels: every (k, v) must be present and equal.
    for (k, v) in &sel.match_labels {
        if labels.get(k) != Some(v) {
            return false;
        }
    }
    // matchExpressions: every requirement must hold.
    for req in &sel.match_expressions {
        let present = labels.get(&req.key);
        let ok = match req.operator {
            LabelSelectorOperator::In => present.is_some_and(|v| req.values.contains(v)),
            LabelSelectorOperator::NotIn => present.map_or(true, |v| !req.values.contains(v)),
            LabelSelectorOperator::Exists => present.is_some(),
            LabelSelectorOperator::DoesNotExist => present.is_none(),
        };
        if !ok {
            return false;
        }
    }
    true
}

// ─── Template inputs (the controller-manager template root object) ───────────

/// `PodMeta` — `metav1.ObjectMeta` projection exposed to templates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PodMeta {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
}

/// `PodSpec` projection exposed to templates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PodSpec {
    pub service_account_name: String,
    pub node_name: String,
}

/// Template root — `.TrustDomain`, `.ClusterName`, `.ClusterDomain`,
/// `.PodMeta.*`, `.PodSpec.*` plus the namespace label set used for the
/// namespace selector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconcileContext {
    pub trust_domain: String,
    pub cluster_name: String,
    pub cluster_domain: String,
    pub pod_meta: PodMeta,
    pub pod_spec: PodSpec,
    pub namespace_labels: BTreeMap<String, String>,
}

/// Resolve a single `{{ .Path }}` expression against the context.
fn resolve_path(path: &str, ctx: &ReconcileContext) -> Result<String> {
    let p = path.trim();
    match p {
        ".TrustDomain" => Ok(ctx.trust_domain.clone()),
        ".ClusterName" => Ok(ctx.cluster_name.clone()),
        ".ClusterDomain" => Ok(ctx.cluster_domain.clone()),
        ".PodMeta.Namespace" => Ok(ctx.pod_meta.namespace.clone()),
        ".PodMeta.Name" => Ok(ctx.pod_meta.name.clone()),
        ".PodMeta.UID" => Ok(ctx.pod_meta.uid.clone()),
        ".PodSpec.ServiceAccountName" => Ok(ctx.pod_spec.service_account_name.clone()),
        ".PodSpec.NodeName" => Ok(ctx.pod_spec.node_name.clone()),
        other => {
            if let Some(k) = other.strip_prefix(".PodMeta.Labels.") {
                return ctx.pod_meta.labels.get(k).cloned().ok_or_else(|| {
                    IdentityError::Internal(format!("template: missing pod label '{}'", k))
                });
            }
            if let Some(k) = other.strip_prefix(".PodMeta.Annotations.") {
                return ctx.pod_meta.annotations.get(k).cloned().ok_or_else(|| {
                    IdentityError::Internal(format!("template: missing pod annotation '{}'", k))
                });
            }
            Err(IdentityError::Internal(format!(
                "template: unknown path '{}'",
                other
            )))
        }
    }
}

/// Render a Go-`text/template`-style string supporting the `{{ .Path }}`
/// pipeline subset the controller-manager templates rely on.
pub fn render_template(tmpl: &str, ctx: &ReconcileContext) -> Result<String> {
    let mut out = String::with_capacity(tmpl.len());
    let mut rest = tmpl;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| {
            IdentityError::Internal("template: unterminated '{{' action".into())
        })?;
        let expr = after[..end].trim();
        out.push_str(&resolve_path(expr, ctx)?);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

// ─── ClusterSPIFFEID CRD ─────────────────────────────────────────────────────

/// `ClusterSPIFFEID` spec (`api/v1alpha1/clusterspiffeid_types.go`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterSpiffeId {
    pub spiffe_id_template: String,
    pub pod_selector: Option<LabelSelector>,
    pub namespace_selector: Option<LabelSelector>,
    pub dns_name_templates: Vec<String>,
    pub workload_selector_templates: Vec<String>,
    pub ttl_seconds: u32,
    pub jwt_ttl_seconds: u32,
    pub federates_with: Vec<String>,
    pub admin: bool,
    pub downstream: bool,
    pub hint: Option<String>,
    pub auto_populate_dns_names: bool,
    pub fallback: bool,
    pub class_name: Option<String>,
}

impl ClusterSpiffeId {
    /// True when both the namespace selector (vs `namespace_labels`) and the
    /// pod selector (vs `pod_meta.labels`) match. Absent selectors match all.
    pub fn matches(&self, ctx: &ReconcileContext) -> bool {
        if let Some(ns) = &self.namespace_selector {
            if !label_selector_matches(ns, &ctx.namespace_labels) {
                return false;
            }
        }
        if let Some(ps) = &self.pod_selector {
            if !label_selector_matches(ps, &ctx.pod_meta.labels) {
                return false;
            }
        }
        true
    }

    /// Render a [`RegistrationEntry`] for `ctx` parented at `parent_id` (the
    /// node/agent SPIFFE ID the pod is scheduled on). Errors if the pod does
    /// not match the selectors or a template fails to render/validate.
    pub fn reconcile(
        &self,
        ctx: &ReconcileContext,
        parent_id: &SpiffeId,
    ) -> Result<RegistrationEntry> {
        if !self.matches(ctx) {
            return Err(IdentityError::Internal(
                "pod does not match ClusterSPIFFEID selectors".into(),
            ));
        }
        let tmpl = if self.spiffe_id_template.is_empty() {
            DEFAULT_SPIFFE_ID_TEMPLATE
        } else {
            self.spiffe_id_template.as_str()
        };
        let spiffe_id_str = render_template(tmpl, ctx)?;
        // Validate the rendered ID is a well-formed SPIFFE ID.
        parse_spiffe_id(&spiffe_id_str)?;

        // Pin the entry to this exact pod via the k8s pod-uid selector, then
        // append any rendered workloadSelectorTemplates (each "kind:value").
        let mut selectors = vec![Selector::new("k8s", format!("pod-uid:{}", ctx.pod_meta.uid))];
        for t in &self.workload_selector_templates {
            let rendered = render_template(t, ctx)?;
            let (kind, value) = rendered.split_once(':').ok_or_else(|| {
                IdentityError::Internal(format!(
                    "workload selector template must render to 'kind:value': {}",
                    rendered
                ))
            })?;
            selectors.push(Selector::new(kind, value));
        }

        let mut dns_names = Vec::new();
        for t in &self.dns_name_templates {
            dns_names.push(render_template(t, ctx)?);
        }
        if self.auto_populate_dns_names && !ctx.pod_meta.name.is_empty() {
            // controller-manager auto-populates the pod's own name as a SAN.
            if !dns_names.contains(&ctx.pod_meta.name) {
                dns_names.push(ctx.pod_meta.name.clone());
            }
        }

        let ttl = if self.ttl_seconds > 0 {
            self.ttl_seconds
        } else {
            RegistrationEntry::default().x509_svid_ttl_seconds
        };
        let jwt_ttl = if self.jwt_ttl_seconds > 0 {
            self.jwt_ttl_seconds
        } else {
            RegistrationEntry::default().jwt_svid_ttl_seconds
        };
        let federates_with = self
            .federates_with
            .iter()
            .map(|s| TrustDomain::new(s.clone()))
            .collect();

        Ok(RegistrationEntry {
            spiffe_id: SpiffeId::new(spiffe_id_str),
            parent_id: parent_id.clone(),
            selectors,
            ttl_seconds: ttl,
            x509_svid_ttl_seconds: ttl,
            jwt_svid_ttl_seconds: jwt_ttl,
            federates_with,
            admin: self.admin,
            downstream: self.downstream,
            dns_names,
            hint: self.hint.clone(),
            ..Default::default()
        })
    }
}

// ─── ClusterFederatedTrustDomain CRD ─────────────────────────────────────────

/// CRD form of the federation bundle-endpoint profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FederationProfileSpec {
    HttpsWeb,
    HttpsSpiffe { endpoint_spiffe_id: String },
}

/// `ClusterFederatedTrustDomain` spec
/// (`api/v1alpha1/clusterfederatedtrustdomain_types.go`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterFederatedTrustDomain {
    pub trust_domain: String,
    pub bundle_endpoint_url: String,
    pub bundle_endpoint_profile: FederationProfileSpec,
    pub class_name: Option<String>,
}

impl ClusterFederatedTrustDomain {
    /// Validate + map to a [`FederationRelationship`]
    /// (`spireapi.FederationRelationship` conversion in the reconciler).
    pub fn to_federation_relationship(&self) -> Result<FederationRelationship> {
        if self.trust_domain.is_empty() {
            return Err(IdentityError::FederationInvalid("trust domain empty".into()));
        }
        validate_trust_domain(&self.trust_domain)?;
        if !self.bundle_endpoint_url.starts_with("https://") {
            return Err(IdentityError::FederationInvalid(
                "bundle endpoint url must be https://".into(),
            ));
        }
        let profile = match &self.bundle_endpoint_profile {
            FederationProfileSpec::HttpsWeb => BundleEndpointProfile::HttpsWeb,
            FederationProfileSpec::HttpsSpiffe { endpoint_spiffe_id } => {
                parse_spiffe_id(endpoint_spiffe_id)?;
                BundleEndpointProfile::HttpsSpiffe {
                    endpoint_spiffe_id: SpiffeId::new(endpoint_spiffe_id.clone()),
                }
            }
        };
        Ok(FederationRelationship {
            trust_domain: TrustDomain::new(self.trust_domain.clone()),
            bundle_endpoint_url: self.bundle_endpoint_url.clone(),
            bundle_endpoint_profile: profile,
            trust_domain_bundle: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lbl(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn ctx() -> ReconcileContext {
        ReconcileContext {
            trust_domain: "example.org".into(),
            cluster_name: "prod".into(),
            cluster_domain: "cluster.local".into(),
            pod_meta: PodMeta {
                namespace: "ns1".into(),
                name: "frontend-abc".into(),
                uid: "pod-uid-1".into(),
                labels: lbl(&[("app", "frontend"), ("tier", "web")]),
                annotations: lbl(&[("owner", "team-a")]),
            },
            pod_spec: PodSpec {
                service_account_name: "default".into(),
                node_name: "node-1".into(),
            },
            namespace_labels: lbl(&[("env", "prod")]),
        }
    }

    // ── label selector ──────────────────────────────────────────────────────

    #[test]
    fn match_labels_all_must_equal() {
        let sel = LabelSelector {
            match_labels: lbl(&[("app", "frontend")]),
            ..Default::default()
        };
        assert!(label_selector_matches(&sel, &lbl(&[("app", "frontend")])));
        assert!(!label_selector_matches(&sel, &lbl(&[("app", "backend")])));
        assert!(!label_selector_matches(&sel, &lbl(&[("other", "x")])));
    }

    #[test]
    fn match_expressions_in_notin() {
        let sel = LabelSelector {
            match_expressions: vec![LabelSelectorRequirement {
                key: "tier".into(),
                operator: LabelSelectorOperator::In,
                values: vec!["web".into(), "cache".into()],
            }],
            ..Default::default()
        };
        assert!(label_selector_matches(&sel, &lbl(&[("tier", "web")])));
        assert!(!label_selector_matches(&sel, &lbl(&[("tier", "db")])));

        let sel = LabelSelector {
            match_expressions: vec![LabelSelectorRequirement {
                key: "tier".into(),
                operator: LabelSelectorOperator::NotIn,
                values: vec!["db".into()],
            }],
            ..Default::default()
        };
        assert!(label_selector_matches(&sel, &lbl(&[("tier", "web")])));
        assert!(!label_selector_matches(&sel, &lbl(&[("tier", "db")])));
        // NotIn matches when the key is absent.
        assert!(label_selector_matches(&sel, &lbl(&[("x", "y")])));
    }

    #[test]
    fn match_expressions_exists_doesnotexist() {
        let exists = LabelSelector {
            match_expressions: vec![LabelSelectorRequirement {
                key: "app".into(),
                operator: LabelSelectorOperator::Exists,
                values: vec![],
            }],
            ..Default::default()
        };
        assert!(label_selector_matches(&exists, &lbl(&[("app", "x")])));
        assert!(!label_selector_matches(&exists, &lbl(&[("y", "z")])));

        let absent = LabelSelector {
            match_expressions: vec![LabelSelectorRequirement {
                key: "app".into(),
                operator: LabelSelectorOperator::DoesNotExist,
                values: vec![],
            }],
            ..Default::default()
        };
        assert!(absent_matches(&absent));
    }

    fn absent_matches(sel: &LabelSelector) -> bool {
        label_selector_matches(sel, &lbl(&[("y", "z")]))
            && !label_selector_matches(sel, &lbl(&[("app", "x")]))
    }

    #[test]
    fn empty_selector_matches_everything() {
        let sel = LabelSelector::default();
        assert!(label_selector_matches(&sel, &lbl(&[])));
        assert!(label_selector_matches(&sel, &lbl(&[("a", "b")])));
    }

    // ── template rendering ────────────────────────────────────────────────────

    #[test]
    fn render_default_template() {
        let out = render_template(DEFAULT_SPIFFE_ID_TEMPLATE, &ctx()).unwrap();
        assert_eq!(out, "spiffe://example.org/ns/ns1/sa/default");
    }

    #[test]
    fn render_all_scalar_paths() {
        let t = "{{ .TrustDomain }}|{{ .ClusterName }}|{{ .ClusterDomain }}|{{ .PodMeta.Name }}|{{ .PodMeta.UID }}|{{ .PodSpec.NodeName }}";
        assert_eq!(
            render_template(t, &ctx()).unwrap(),
            "example.org|prod|cluster.local|frontend-abc|pod-uid-1|node-1"
        );
    }

    #[test]
    fn render_label_and_annotation_paths() {
        assert_eq!(
            render_template("{{ .PodMeta.Labels.app }}", &ctx()).unwrap(),
            "frontend"
        );
        assert_eq!(
            render_template("{{ .PodMeta.Annotations.owner }}", &ctx()).unwrap(),
            "team-a"
        );
    }

    #[test]
    fn render_rejects_unknown_path() {
        assert!(render_template("{{ .Nope.Field }}", &ctx()).is_err());
    }

    #[test]
    fn render_rejects_unterminated() {
        assert!(render_template("a {{ .TrustDomain ", &ctx()).is_err());
    }

    #[test]
    fn render_literal_passthrough() {
        assert_eq!(render_template("no templates here", &ctx()).unwrap(), "no templates here");
    }

    // ── ClusterSPIFFEID.matches ───────────────────────────────────────────────

    #[test]
    fn matches_pod_and_namespace_selectors() {
        let csid = ClusterSpiffeId {
            pod_selector: Some(LabelSelector {
                match_labels: lbl(&[("app", "frontend")]),
                ..Default::default()
            }),
            namespace_selector: Some(LabelSelector {
                match_labels: lbl(&[("env", "prod")]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(csid.matches(&ctx()));
    }

    #[test]
    fn matches_fails_on_pod_selector() {
        let csid = ClusterSpiffeId {
            pod_selector: Some(LabelSelector {
                match_labels: lbl(&[("app", "backend")]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!csid.matches(&ctx()));
    }

    #[test]
    fn matches_fails_on_namespace_selector() {
        let csid = ClusterSpiffeId {
            namespace_selector: Some(LabelSelector {
                match_labels: lbl(&[("env", "staging")]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!csid.matches(&ctx()));
    }

    // ── ClusterSPIFFEID.reconcile ─────────────────────────────────────────────

    #[test]
    fn reconcile_default_template_pins_pod_uid() {
        let csid = ClusterSpiffeId::default();
        let parent = SpiffeId::new("spiffe://example.org/spire/agent/k8s_psat/prod/node-1");
        let e = csid.reconcile(&ctx(), &parent).unwrap();
        assert_eq!(e.spiffe_id.as_str(), "spiffe://example.org/ns/ns1/sa/default");
        assert_eq!(e.parent_id.as_str(), parent.as_str());
        assert!(e
            .selectors
            .iter()
            .any(|s| s.kind == "k8s" && s.value == "pod-uid:pod-uid-1"));
    }

    #[test]
    fn reconcile_honours_ttl_admin_downstream_federates() {
        let csid = ClusterSpiffeId {
            ttl_seconds: 7200,
            jwt_ttl_seconds: 120,
            admin: true,
            downstream: true,
            federates_with: vec!["peer.org".into()],
            hint: Some("frontend".into()),
            ..Default::default()
        };
        let parent = SpiffeId::new("spiffe://example.org/spire/agent/x/node-1");
        let e = csid.reconcile(&ctx(), &parent).unwrap();
        assert_eq!(e.x509_svid_ttl_seconds, 7200);
        assert_eq!(e.jwt_svid_ttl_seconds, 120);
        assert!(e.admin);
        assert!(e.downstream);
        assert_eq!(e.federates_with.len(), 1);
        assert_eq!(e.federates_with[0].as_str(), "peer.org");
        assert_eq!(e.hint.as_deref(), Some("frontend"));
    }

    #[test]
    fn reconcile_renders_dns_and_workload_selectors() {
        let csid = ClusterSpiffeId {
            dns_name_templates: vec!["{{ .PodMeta.Name }}.{{ .PodMeta.Namespace }}.svc.{{ .ClusterDomain }}".into()],
            workload_selector_templates: vec!["k8s:pod-label:app={{ .PodMeta.Labels.app }}".into()],
            auto_populate_dns_names: true,
            ..Default::default()
        };
        let parent = SpiffeId::new("spiffe://example.org/spire/agent/x/node-1");
        let e = csid.reconcile(&ctx(), &parent).unwrap();
        assert!(e
            .dns_names
            .contains(&"frontend-abc.ns1.svc.cluster.local".to_string()));
        // auto_populate_dns_names adds the bare pod name.
        assert!(e.dns_names.contains(&"frontend-abc".to_string()));
        assert!(e
            .selectors
            .iter()
            .any(|s| s.kind == "k8s" && s.value == "pod-label:app=frontend"));
    }

    #[test]
    fn reconcile_rejects_non_matching_pod() {
        let csid = ClusterSpiffeId {
            pod_selector: Some(LabelSelector {
                match_labels: lbl(&[("app", "backend")]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let parent = SpiffeId::new("spiffe://example.org/spire/agent/x/n");
        assert!(csid.reconcile(&ctx(), &parent).is_err());
    }

    // ── ClusterFederatedTrustDomain ───────────────────────────────────────────

    #[test]
    fn cftd_maps_https_web() {
        let cftd = ClusterFederatedTrustDomain {
            trust_domain: "peer.org".into(),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: FederationProfileSpec::HttpsWeb,
            class_name: None,
        };
        let rel = cftd.to_federation_relationship().unwrap();
        assert_eq!(rel.trust_domain.as_str(), "peer.org");
        assert_eq!(rel.bundle_endpoint_url, "https://peer.org/bundle");
        assert_eq!(rel.bundle_endpoint_profile, BundleEndpointProfile::HttpsWeb);
    }

    #[test]
    fn cftd_maps_https_spiffe() {
        let cftd = ClusterFederatedTrustDomain {
            trust_domain: "peer.org".into(),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: FederationProfileSpec::HttpsSpiffe {
                endpoint_spiffe_id: "spiffe://peer.org/spire/server".into(),
            },
            class_name: Some("spire".into()),
        };
        let rel = cftd.to_federation_relationship().unwrap();
        match rel.bundle_endpoint_profile {
            BundleEndpointProfile::HttpsSpiffe { endpoint_spiffe_id } => {
                assert_eq!(endpoint_spiffe_id.as_str(), "spiffe://peer.org/spire/server");
            }
            _ => panic!("expected HttpsSpiffe"),
        }
    }

    #[test]
    fn cftd_rejects_non_https_endpoint() {
        let cftd = ClusterFederatedTrustDomain {
            trust_domain: "peer.org".into(),
            bundle_endpoint_url: "http://peer.org/bundle".into(),
            bundle_endpoint_profile: FederationProfileSpec::HttpsWeb,
            class_name: None,
        };
        assert!(cftd.to_federation_relationship().is_err());
    }

    #[test]
    fn cftd_rejects_bad_spiffe_endpoint() {
        let cftd = ClusterFederatedTrustDomain {
            trust_domain: "peer.org".into(),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: FederationProfileSpec::HttpsSpiffe {
                endpoint_spiffe_id: "not-a-spiffe-id".into(),
            },
            class_name: None,
        };
        assert!(cftd.to_federation_relationship().is_err());
    }

    #[test]
    fn cftd_rejects_empty_trust_domain() {
        let cftd = ClusterFederatedTrustDomain {
            trust_domain: "".into(),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: FederationProfileSpec::HttpsWeb,
            class_name: None,
        };
        assert!(cftd.to_federation_relationship().is_err());
    }
}
