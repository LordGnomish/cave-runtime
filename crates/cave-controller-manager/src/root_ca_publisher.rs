// SPDX-License-Identifier: AGPL-3.0-or-later
//! Root CA publisher — `pkg/controller/certificates/rootcacertpublisher/publisher.go`.
//!
//! Maintains a `kube-root-ca.crt` ConfigMap in every active namespace whose
//! `data["ca.crt"]` matches the cluster root CA bundle. Skipped for
//! terminating namespaces.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const CONFIGMAP_NAME: &str = "kube-root-ca.crt";
pub const DATA_KEY: &str = "ca.crt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespacePhase {
    Active,
    Terminating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub name: String,
    pub phase: NamespacePhase,
}

/// Observed state of a `kube-root-ca.crt` ConfigMap. Absent → publisher
/// should create it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedConfigMap {
    pub namespace: String,
    pub ca_crt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishAction {
    /// Namespace is terminating or already in sync — nothing to do.
    NoOp,
    /// ConfigMap doesn't exist; create it with the cluster CA bundle.
    Create,
    /// ConfigMap exists but `ca.crt` differs; rewrite it.
    Update,
}

/// Decide whether a single namespace's root-CA ConfigMap needs publishing.
pub fn evaluate(
    ns: &Namespace,
    observed: Option<&ObservedConfigMap>,
    cluster_ca: &str,
) -> PublishAction {
    if matches!(ns.phase, NamespacePhase::Terminating) {
        return PublishAction::NoOp;
    }
    match observed {
        None => PublishAction::Create,
        Some(cm) => match &cm.ca_crt {
            None => PublishAction::Update,
            Some(existing) if existing == cluster_ca => PublishAction::NoOp,
            Some(_) => PublishAction::Update,
        },
    }
}

/// Compute the per-namespace plan in one pass. Mirrors `syncHandler` running
/// once per (namespace, ConfigMap) pair.
pub fn plan_for_namespaces(
    namespaces: &[Namespace],
    observed: &[ObservedConfigMap],
    cluster_ca: &str,
) -> Vec<(String, PublishAction)> {
    namespaces
        .iter()
        .map(|ns| {
            let cm = observed.iter().find(|c| c.namespace == ns.name);
            (ns.name.clone(), evaluate(ns, cm, cluster_ca))
        })
        .collect()
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/rootcacertpublisher/publisher.go",
    "Publisher",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ns(name: &str, phase: NamespacePhase) -> Namespace {
        Namespace { name: name.into(), phase }
    }
    fn cm(ns_name: &str, ca: Option<&str>) -> ObservedConfigMap {
        ObservedConfigMap {
            namespace: ns_name.into(),
            ca_crt: ca.map(|s| s.to_string()),
        }
    }

    #[test]
    fn missing_configmap_triggers_create() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca-create"
        );
        let n = ns("default", NamespacePhase::Active);
        assert_eq!(evaluate(&n, None, "CA-A"), PublishAction::Create);
    }

    #[test]
    fn matching_ca_is_no_op() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca-noop"
        );
        let n = ns("default", NamespacePhase::Active);
        let c = cm("default", Some("CA-A"));
        assert_eq!(evaluate(&n, Some(&c), "CA-A"), PublishAction::NoOp);
    }

    #[test]
    fn mismatched_ca_triggers_update() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca-update"
        );
        let n = ns("default", NamespacePhase::Active);
        let c = cm("default", Some("CA-OLD"));
        assert_eq!(evaluate(&n, Some(&c), "CA-NEW"), PublishAction::Update);
    }

    #[test]
    fn missing_ca_crt_key_triggers_update() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca-missing-key"
        );
        let n = ns("default", NamespacePhase::Active);
        let c = cm("default", None);
        assert_eq!(evaluate(&n, Some(&c), "CA-A"), PublishAction::Update);
    }

    #[test]
    fn terminating_namespace_is_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca-terminating"
        );
        let n = ns("ending", NamespacePhase::Terminating);
        // Even when the CM is missing, terminating ns is left alone.
        assert_eq!(evaluate(&n, None, "CA-A"), PublishAction::NoOp);
    }

    #[test]
    fn plan_visits_every_namespace() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "Run",
            "tenant-rca-plan"
        );
        let nss = vec![
            ns("a", NamespacePhase::Active),
            ns("b", NamespacePhase::Active),
            ns("c", NamespacePhase::Terminating),
        ];
        let cms = vec![cm("a", Some("CA-A")), cm("b", Some("OLD"))];
        let plan = plan_for_namespaces(&nss, &cms, "CA-A");
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0], ("a".into(), PublishAction::NoOp));
        assert_eq!(plan[1], ("b".into(), PublishAction::Update));
        assert_eq!(plan[2], ("c".into(), PublishAction::NoOp));
    }

    #[test]
    fn constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "RootCAConfigMapName",
            "tenant-rca-const"
        );
        assert_eq!(CONFIGMAP_NAME, "kube-root-ca.crt");
        assert_eq!(DATA_KEY, "ca.crt");
    }

    #[test]
    fn publish_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "PublishAction",
            "tenant-rca-action-serde"
        );
        for a in [
            PublishAction::NoOp,
            PublishAction::Create,
            PublishAction::Update,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: PublishAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn empty_namespace_list_yields_empty_plan() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "Run",
            "tenant-rca-plan-empty"
        );
        let plan = plan_for_namespaces(&[], &[], "CA");
        assert!(plan.is_empty());
    }

    #[test]
    fn unobserved_namespace_in_plan_creates() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "Run",
            "tenant-rca-plan-no-cm"
        );
        let plan =
            plan_for_namespaces(&[ns("fresh", NamespacePhase::Active)], &[], "CA");
        assert_eq!(plan, vec![("fresh".into(), PublishAction::Create)]);
    }
}
