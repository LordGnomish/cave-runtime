// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0) — spire-controller-manager.
// ClusterSPIFFEID + ClusterFederatedTrustDomain reconcile line-ported from
// github.com/spiffe/spire-controller-manager pkg/spireentry + pkg/reconciler.

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
