// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0) — spire-controller-manager.
// ClusterSPIFFEID + ClusterFederatedTrustDomain reconcile line-ported from
// github.com/spiffe/spire-controller-manager pkg/spireentry + pkg/reconciler.
//
//! spire-controller-manager parity — the Kubernetes operator that turns
//! `ClusterSPIFFEID` and `ClusterFederatedTrustDomain` custom resources into
//! SPIRE registration entries and federation relationships.
//!
//! The CRD-watch / informer / apply loop is owned by cave-cri / cave-k8s; this
//! module ports the **pure reconcile core**: label-selector matching
//! (`metav1.LabelSelector` semantics), the Go-`text/template` SPIFFE-ID +
//! DNS-name rendering subset used by the controller, and the
//! CR → [`RegistrationEntry`] / [`FederationRelationship`] projection with the
//! same validation the upstream `validateFederatedTrustDomain` performs.

use crate::error::{IdentityError, Result};
use crate::models::{
    BundleEndpointProfile, FederationRelationship, RegistrationEntry, Selector, SpiffeId,
    TrustDomain,
};
use std::collections::BTreeMap;

/// `metav1.LabelSelectorOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSelectorOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

/// `metav1.LabelSelectorRequirement` — a single set-based match expression.
#[derive(Debug, Clone)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: LabelSelectorOp,
    pub values: Vec<String>,
}

impl LabelSelectorRequirement {
    fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        let present = labels.get(&self.key);
        match self.operator {
            LabelSelectorOp::In => present.map(|v| self.values.contains(v)).unwrap_or(false),
            // NotIn is satisfied when the key is absent OR its value is not listed.
            LabelSelectorOp::NotIn => present.map(|v| !self.values.contains(v)).unwrap_or(true),
            LabelSelectorOp::Exists => present.is_some(),
            LabelSelectorOp::DoesNotExist => present.is_none(),
        }
    }
}

/// `metav1.LabelSelector` — `matchLabels` AND `matchExpressions`.
///
/// An empty selector (no labels, no expressions) matches every object, mirroring
/// `labels.Everything()` in client-go.
#[derive(Debug, Clone, Default)]
pub struct LabelSelector {
    pub match_labels: BTreeMap<String, String>,
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

impl LabelSelector {
    /// True when every `matchLabels` pair AND every `matchExpressions`
    /// requirement is satisfied by `labels`.
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        for (k, v) in &self.match_labels {
            if labels.get(k) != Some(v) {
                return false;
            }
        }
        self.match_expressions.iter().all(|r| r.matches(labels))
    }
}

/// Minimal pod metadata the controller reads to render templates + bind
/// selectors — the subset of `corev1.Pod` `ObjectMeta` + `PodSpec` used by
/// `spire-controller-manager`.
#[derive(Debug, Clone, Default)]
pub struct PodMeta {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub service_account: String,
    pub node_name: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
}

/// Render context exposed to the Go-`text/template` SPIFFE-ID template — the
/// `.TrustDomain`, `.PodMeta.*`, `.PodSpec.*` roots the controller documents.
pub struct TemplateContext<'a> {
    pub trust_domain: &'a TrustDomain,
    pub pod: &'a PodMeta,
}

impl<'a> TemplateContext<'a> {
    pub fn new(trust_domain: &'a TrustDomain, pod: &'a PodMeta) -> Self {
        Self { trust_domain, pod }
    }
}

/// Render a `spire-controller-manager` ID/DNS template against `ctx`.
///
/// Supports the documented subset of Go `text/template` the controller uses:
/// `{{ .Field.Path }}` dotted-field access and `{{ index .Map "key" }}` map
/// lookups (labels / annotations). An unknown field or a missing map key is an
/// error — matching the controller's behaviour with `missingkey=error`.
pub fn render_template(tmpl: &str, ctx: &TemplateContext) -> Result<String> {
    let mut out = String::with_capacity(tmpl.len());
    let mut rest = tmpl;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| IdentityError::Internal("unterminated template action".into()))?;
        let expr = after[..close].trim();
        out.push_str(&eval_action(expr, ctx)?);
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn eval_action(expr: &str, ctx: &TemplateContext) -> Result<String> {
    if let Some(args) = expr.strip_prefix("index ") {
        // `index .PodMeta.Labels "key"` / `index .PodMeta.Annotations "key"`
        let args = args.trim();
        let q1 = args
            .find('"')
            .ok_or_else(|| IdentityError::Internal(format!("index: missing key in {expr:?}")))?;
        let path = args[..q1].trim();
        let key_part = &args[q1 + 1..];
        let q2 = key_part
            .find('"')
            .ok_or_else(|| IdentityError::Internal(format!("index: unterminated key in {expr:?}")))?;
        let key = &key_part[..q2];
        let map = match path {
            ".PodMeta.Labels" => &ctx.pod.labels,
            ".PodMeta.Annotations" => &ctx.pod.annotations,
            other => {
                return Err(IdentityError::Internal(format!(
                    "index: unknown map {other:?}"
                )))
            }
        };
        return map
            .get(key)
            .cloned()
            .ok_or_else(|| IdentityError::Internal(format!("index: key {key:?} not found")));
    }
    resolve_field(expr, ctx)
}

fn resolve_field(path: &str, ctx: &TemplateContext) -> Result<String> {
    let v = match path {
        ".TrustDomain" => ctx.trust_domain.as_str().to_string(),
        ".PodMeta.Namespace" => ctx.pod.namespace.clone(),
        ".PodMeta.Name" => ctx.pod.name.clone(),
        ".PodMeta.UID" => ctx.pod.uid.clone(),
        ".PodMeta.ServiceAccount" => ctx.pod.service_account.clone(),
        ".PodMeta.NodeName" => ctx.pod.node_name.clone(),
        ".PodSpec.ServiceAccountName" => ctx.pod.service_account.clone(),
        ".PodSpec.NodeName" => ctx.pod.node_name.clone(),
        other => {
            return Err(IdentityError::Internal(format!(
                "unknown template field {other:?}"
            )))
        }
    };
    Ok(v)
}

/// `ClusterSPIFFEID` custom-resource spec (the reconcile-relevant subset).
#[derive(Debug, Clone)]
pub struct ClusterSpiffeId {
    pub name: String,
    pub spiffe_id_template: String,
    pub parent_id: SpiffeId,
    pub pod_selector: LabelSelector,
    pub namespace_selector: LabelSelector,
    pub ttl_seconds: u32,
    pub jwt_ttl_seconds: u32,
    pub dns_name_templates: Vec<String>,
    pub federates_with: Vec<TrustDomain>,
    pub admin: bool,
    pub downstream: bool,
    pub hint: Option<String>,
}

/// Binding selectors the controller attaches so the entry resolves only for
/// the specific pod — mirrors the `k8s` workload-attestor selector set.
fn pod_binding_selectors(pod: &PodMeta) -> Vec<Selector> {
    let mut v = vec![
        Selector::new("k8s", format!("ns:{}", pod.namespace)),
        Selector::new("k8s", format!("pod-uid:{}", pod.uid)),
        Selector::new("k8s", format!("pod-name:{}", pod.name)),
        Selector::new("k8s", format!("sa:{}", pod.service_account)),
    ];
    if !pod.node_name.is_empty() {
        v.push(Selector::new("k8s", format!("node-name:{}", pod.node_name)));
    }
    v
}

/// Reconcile a `ClusterSPIFFEID` against the cluster's pods, producing one
/// [`RegistrationEntry`] per pod that matches both the pod- and
/// namespace-label selectors.
///
/// `pods` is the candidate set as `(pod, namespace_labels)` pairs — the
/// controller pre-joins each pod with its namespace's labels so the
/// `namespaceSelector` can be evaluated.
pub fn reconcile_cluster_spiffe_id(
    trust_domain: &TrustDomain,
    cr: &ClusterSpiffeId,
    pods: &[(PodMeta, BTreeMap<String, String>)],
) -> Result<Vec<RegistrationEntry>> {
    let mut entries = Vec::new();
    for (pod, ns_labels) in pods {
        if !cr.pod_selector.matches(&pod.labels) {
            continue;
        }
        if !cr.namespace_selector.matches(ns_labels) {
            continue;
        }
        let ctx = TemplateContext::new(trust_domain, pod);
        let spiffe_id = SpiffeId::new(render_template(&cr.spiffe_id_template, &ctx)?);
        let mut dns_names = Vec::with_capacity(cr.dns_name_templates.len());
        for t in &cr.dns_name_templates {
            dns_names.push(render_template(t, &ctx)?);
        }
        entries.push(RegistrationEntry {
            spiffe_id,
            parent_id: cr.parent_id.clone(),
            selectors: pod_binding_selectors(pod),
            ttl_seconds: cr.ttl_seconds,
            x509_svid_ttl_seconds: cr.ttl_seconds,
            jwt_svid_ttl_seconds: cr.jwt_ttl_seconds,
            federates_with: cr.federates_with.clone(),
            admin: cr.admin,
            downstream: cr.downstream,
            dns_names,
            hint: cr.hint.clone(),
            ..Default::default()
        });
    }
    Ok(entries)
}

/// `ClusterFederatedTrustDomain` custom-resource spec.
#[derive(Debug, Clone)]
pub struct ClusterFederatedTrustDomain {
    pub name: String,
    pub trust_domain: TrustDomain,
    pub bundle_endpoint_url: String,
    pub bundle_endpoint_profile: BundleEndpointProfile,
}

/// Reconcile a `ClusterFederatedTrustDomain` into a [`FederationRelationship`].
///
/// Applies the same validation `spire-controller-manager`'s
/// `ParseClusterFederatedTrustDomainSpec` does: the peer trust domain must
/// differ from our own, the bundle endpoint URL must be HTTPS, and for the
/// `https_spiffe` profile the endpoint SPIFFE ID must belong to the peer
/// trust domain.
pub fn reconcile_federated_trust_domain(
    own: &TrustDomain,
    cr: &ClusterFederatedTrustDomain,
) -> Result<FederationRelationship> {
    if cr.trust_domain.as_str() == own.as_str() {
        return Err(IdentityError::FederationInvalid(
            "cannot federate with own trust domain".into(),
        ));
    }
    if !cr.bundle_endpoint_url.starts_with("https://") {
        return Err(IdentityError::FederationInvalid(format!(
            "bundle endpoint URL must be https: {}",
            cr.bundle_endpoint_url
        )));
    }
    if let BundleEndpointProfile::HttpsSpiffe { endpoint_spiffe_id } = &cr.bundle_endpoint_profile {
        let expected = cr.trust_domain.id_string();
        let belongs = endpoint_spiffe_id.as_str() == expected
            || endpoint_spiffe_id
                .as_str()
                .starts_with(&format!("{expected}/"));
        if !belongs {
            return Err(IdentityError::FederationInvalid(format!(
                "endpoint SPIFFE ID {} not in trust domain {}",
                endpoint_spiffe_id,
                cr.trust_domain.as_str()
            )));
        }
    }
    Ok(FederationRelationship {
        trust_domain: cr.trust_domain.clone(),
        bundle_endpoint_url: cr.bundle_endpoint_url.clone(),
        bundle_endpoint_profile: cr.bundle_endpoint_profile.clone(),
        trust_domain_bundle: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BundleEndpointProfile, SpiffeId, TrustDomain};
    use std::collections::BTreeMap;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn pod(ns: &str, name: &str, sa: &str, ls: &[(&str, &str)]) -> PodMeta {
        PodMeta {
            namespace: ns.to_string(),
            name: name.to_string(),
            uid: format!("uid-{name}"),
            service_account: sa.to_string(),
            node_name: "node-a".to_string(),
            labels: labels(ls),
            annotations: BTreeMap::new(),
        }
    }

    // ── label-selector matching ────────────────────────────────────────────

    #[test]
    fn empty_label_selector_matches_everything() {
        let sel = LabelSelector::default();
        assert!(sel.matches(&labels(&[("app", "foo")])));
        assert!(sel.matches(&BTreeMap::new()));
    }

    #[test]
    fn match_labels_requires_all_pairs() {
        let mut sel = LabelSelector::default();
        sel.match_labels = labels(&[("app", "foo"), ("tier", "web")]);
        assert!(sel.matches(&labels(&[("app", "foo"), ("tier", "web"), ("x", "y")])));
        assert!(!sel.matches(&labels(&[("app", "foo")])));
        assert!(!sel.matches(&labels(&[("app", "bar"), ("tier", "web")])));
    }

    #[test]
    fn match_expressions_in_notin_exists() {
        let sel = LabelSelector {
            match_labels: BTreeMap::new(),
            match_expressions: vec![
                LabelSelectorRequirement {
                    key: "tier".into(),
                    operator: LabelSelectorOp::In,
                    values: vec!["web".into(), "api".into()],
                },
                LabelSelectorRequirement {
                    key: "deprecated".into(),
                    operator: LabelSelectorOp::DoesNotExist,
                    values: vec![],
                },
                LabelSelectorRequirement {
                    key: "app".into(),
                    operator: LabelSelectorOp::Exists,
                    values: vec![],
                },
            ],
        };
        assert!(sel.matches(&labels(&[("tier", "web"), ("app", "foo")])));
        // tier not in {web,api}
        assert!(!sel.matches(&labels(&[("tier", "db"), ("app", "foo")])));
        // deprecated present → DoesNotExist fails
        assert!(!sel.matches(&labels(&[("tier", "web"), ("app", "foo"), ("deprecated", "x")])));
        // app missing → Exists fails
        assert!(!sel.matches(&labels(&[("tier", "web")])));

        let notin = LabelSelector {
            match_labels: BTreeMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "env".into(),
                operator: LabelSelectorOp::NotIn,
                values: vec!["prod".into()],
            }],
        };
        assert!(notin.matches(&labels(&[("env", "dev")])));
        assert!(notin.matches(&BTreeMap::new())); // absent key satisfies NotIn
        assert!(!notin.matches(&labels(&[("env", "prod")])));
    }

    // ── spiffe-id template rendering ───────────────────────────────────────

    #[test]
    fn render_template_substitutes_known_fields() {
        let td = TrustDomain::new("example.org");
        let p = pod("prod", "web-1", "frontend", &[("app", "foo")]);
        let ctx = TemplateContext::new(&td, &p);
        let out = render_template(
            "spiffe://{{ .TrustDomain }}/ns/{{ .PodMeta.Namespace }}/sa/{{ .PodSpec.ServiceAccountName }}",
            &ctx,
        )
        .unwrap();
        assert_eq!(out, "spiffe://example.org/ns/prod/sa/frontend");
    }

    #[test]
    fn render_template_index_label() {
        let td = TrustDomain::new("example.org");
        let p = pod("prod", "web-1", "frontend", &[("app", "foo")]);
        let ctx = TemplateContext::new(&td, &p);
        let out = render_template(
            "spiffe://{{ .TrustDomain }}/{{ index .PodMeta.Labels \"app\" }}",
            &ctx,
        )
        .unwrap();
        assert_eq!(out, "spiffe://example.org/foo");
    }

    #[test]
    fn render_template_unknown_field_errors() {
        let td = TrustDomain::new("example.org");
        let p = pod("prod", "web-1", "frontend", &[]);
        let ctx = TemplateContext::new(&td, &p);
        assert!(render_template("{{ .PodMeta.Bogus }}", &ctx).is_err());
        // missing label key is an error too (parity with template "index" miss)
        assert!(render_template("{{ index .PodMeta.Labels \"absent\" }}", &ctx).is_err());
    }

    // ── ClusterSPIFFEID reconcile → registration entries ───────────────────

    fn base_cr() -> ClusterSpiffeId {
        ClusterSpiffeId {
            name: "default".into(),
            spiffe_id_template: "spiffe://{{ .TrustDomain }}/ns/{{ .PodMeta.Namespace }}/sa/{{ .PodSpec.ServiceAccountName }}".into(),
            parent_id: SpiffeId::new("spiffe://example.org/spire/server"),
            pod_selector: LabelSelector::default(),
            namespace_selector: LabelSelector::default(),
            ttl_seconds: 3600,
            jwt_ttl_seconds: 300,
            dns_name_templates: vec!["{{ .PodMeta.Name }}.{{ .PodMeta.Namespace }}.svc".into()],
            federates_with: vec![TrustDomain::new("peer.org")],
            admin: false,
            downstream: false,
            hint: Some("k8s".into()),
        }
    }

    #[test]
    fn reconcile_emits_one_entry_per_matching_pod() {
        let td = TrustDomain::new("example.org");
        let cr = base_cr();
        let pods = vec![
            (pod("prod", "web-1", "frontend", &[("app", "foo")]), labels(&[])),
            (pod("prod", "web-2", "frontend", &[("app", "foo")]), labels(&[])),
        ];
        let entries = reconcile_cluster_spiffe_id(&td, &cr, &pods).unwrap();
        assert_eq!(entries.len(), 2);
        let e = &entries[0];
        assert_eq!(e.spiffe_id.as_str(), "spiffe://example.org/ns/prod/sa/frontend");
        assert_eq!(e.parent_id.as_str(), "spiffe://example.org/spire/server");
        assert_eq!(e.ttl_seconds, 3600);
        assert_eq!(e.jwt_svid_ttl_seconds, 300);
        assert_eq!(e.federates_with, vec![TrustDomain::new("peer.org")]);
        assert_eq!(e.hint.as_deref(), Some("k8s"));
        // binding selectors derived from the pod
        assert!(e.selectors.iter().any(|s| s.canonical() == "k8s:ns:prod"));
        assert!(e.selectors.iter().any(|s| s.canonical() == "k8s:pod-uid:uid-web-1"));
        // dns name rendered
        assert_eq!(e.dns_names, vec!["web-1.prod.svc".to_string()]);
    }

    #[test]
    fn reconcile_respects_pod_and_namespace_selectors() {
        let td = TrustDomain::new("example.org");
        let mut cr = base_cr();
        cr.pod_selector.match_labels = labels(&[("app", "foo")]);
        cr.namespace_selector.match_labels = labels(&[("env", "prod")]);
        let pods = vec![
            // matches both
            (pod("prod", "a", "sa1", &[("app", "foo")]), labels(&[("env", "prod")])),
            // wrong pod label
            (pod("prod", "b", "sa1", &[("app", "bar")]), labels(&[("env", "prod")])),
            // wrong namespace label
            (pod("dev", "c", "sa1", &[("app", "foo")]), labels(&[("env", "dev")])),
        ];
        let entries = reconcile_cluster_spiffe_id(&td, &cr, &pods).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].selectors.iter().filter(|s| s.value == "pod-uid:uid-a").count(), 1);
    }

    #[test]
    fn reconcile_admin_and_downstream_flags_propagate() {
        let td = TrustDomain::new("example.org");
        let mut cr = base_cr();
        cr.admin = true;
        cr.downstream = true;
        let pods = vec![(pod("prod", "a", "sa1", &[]), labels(&[]))];
        let entries = reconcile_cluster_spiffe_id(&td, &cr, &pods).unwrap();
        assert!(entries[0].admin);
        assert!(entries[0].downstream);
    }

    // ── ClusterFederatedTrustDomain reconcile ──────────────────────────────

    #[test]
    fn reconcile_federated_trust_domain_ok() {
        let own = TrustDomain::new("example.org");
        let cr = ClusterFederatedTrustDomain {
            name: "peer".into(),
            trust_domain: TrustDomain::new("peer.org"),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: BundleEndpointProfile::HttpsSpiffe {
                endpoint_spiffe_id: SpiffeId::new("spiffe://peer.org/spire/server"),
            },
        };
        let rel = reconcile_federated_trust_domain(&own, &cr).unwrap();
        assert_eq!(rel.trust_domain.as_str(), "peer.org");
        assert_eq!(rel.bundle_endpoint_url, "https://peer.org/bundle");
    }

    #[test]
    fn reconcile_federated_rejects_self_federation() {
        let own = TrustDomain::new("example.org");
        let cr = ClusterFederatedTrustDomain {
            name: "self".into(),
            trust_domain: TrustDomain::new("example.org"),
            bundle_endpoint_url: "https://example.org/bundle".into(),
            bundle_endpoint_profile: BundleEndpointProfile::HttpsWeb,
        };
        assert!(reconcile_federated_trust_domain(&own, &cr).is_err());
    }

    #[test]
    fn reconcile_federated_rejects_non_https_and_spiffe_td_mismatch() {
        let own = TrustDomain::new("example.org");
        let bad_scheme = ClusterFederatedTrustDomain {
            name: "p".into(),
            trust_domain: TrustDomain::new("peer.org"),
            bundle_endpoint_url: "http://peer.org/bundle".into(),
            bundle_endpoint_profile: BundleEndpointProfile::HttpsWeb,
        };
        assert!(reconcile_federated_trust_domain(&own, &bad_scheme).is_err());

        let wrong_td = ClusterFederatedTrustDomain {
            name: "p".into(),
            trust_domain: TrustDomain::new("peer.org"),
            bundle_endpoint_url: "https://peer.org/bundle".into(),
            bundle_endpoint_profile: BundleEndpointProfile::HttpsSpiffe {
                endpoint_spiffe_id: SpiffeId::new("spiffe://other.org/spire/server"),
            },
        };
        assert!(reconcile_federated_trust_domain(&own, &wrong_td).is_err());
    }
}
