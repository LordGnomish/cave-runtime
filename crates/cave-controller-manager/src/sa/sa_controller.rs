//! ServiceAccount controller — `pkg/controller/serviceaccount/serviceaccounts_controller.go`.
//!
//! Ensures every active namespace has a `default` ServiceAccount. Mirrors
//! `Controller.syncNamespace`.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

/// Default ServiceAccount name. Mirrors `DefaultServiceAccountName`.
pub const DEFAULT_SA_NAME: &str = "default";

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

/// SA name + namespace tuple — the unique key in upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedSa {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceAccountAction {
    /// Default SA exists or namespace terminating — nothing to do.
    NoOp,
    /// Default SA missing — create it.
    Create,
}

/// Decide whether the `default` SA needs to be (re)created in `ns`.
/// Mirrors `serviceAccountsToEnsure` per namespace.
pub fn evaluate(
    ns: &Namespace,
    observed_sas: &[ObservedSa],
    target_name: &str,
) -> ServiceAccountAction {
    if matches!(ns.phase, NamespacePhase::Terminating) {
        return ServiceAccountAction::NoOp;
    }
    let exists = observed_sas
        .iter()
        .any(|s| s.namespace == ns.name && s.name == target_name);
    if exists {
        ServiceAccountAction::NoOp
    } else {
        ServiceAccountAction::Create
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/serviceaccount/serviceaccounts_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ns(name: &str, phase: NamespacePhase) -> Namespace {
        Namespace { name: name.into(), phase }
    }
    fn sa(ns_name: &str, name: &str) -> ObservedSa {
        ObservedSa { namespace: ns_name.into(), name: name.into() }
    }

    #[test]
    fn missing_default_in_active_namespace_creates() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "syncNamespace",
            "tenant-sa-create"
        );
        let n = ns("default", NamespacePhase::Active);
        assert_eq!(
            evaluate(&n, &[], DEFAULT_SA_NAME),
            ServiceAccountAction::Create
        );
    }

    #[test]
    fn existing_default_in_active_namespace_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "syncNamespace",
            "tenant-sa-noop"
        );
        let n = ns("default", NamespacePhase::Active);
        let s = vec![sa("default", "default")];
        assert_eq!(
            evaluate(&n, &s, DEFAULT_SA_NAME),
            ServiceAccountAction::NoOp
        );
    }

    #[test]
    fn terminating_namespace_is_skipped_even_if_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "syncNamespace",
            "tenant-sa-terminating"
        );
        let n = ns("ending", NamespacePhase::Terminating);
        assert_eq!(
            evaluate(&n, &[], DEFAULT_SA_NAME),
            ServiceAccountAction::NoOp
        );
    }

    #[test]
    fn other_namespace_sa_does_not_satisfy_target() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "syncNamespace",
            "tenant-sa-cross-ns"
        );
        let n = ns("ns-a", NamespacePhase::Active);
        // SA exists in ns-b but not ns-a.
        let s = vec![sa("ns-b", "default")];
        assert_eq!(
            evaluate(&n, &s, DEFAULT_SA_NAME),
            ServiceAccountAction::Create
        );
    }

    #[test]
    fn non_default_sa_alone_does_not_count() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "syncNamespace",
            "tenant-sa-not-default"
        );
        let n = ns("default", NamespacePhase::Active);
        let s = vec![sa("default", "build-bot")];
        assert_eq!(
            evaluate(&n, &s, DEFAULT_SA_NAME),
            ServiceAccountAction::Create
        );
    }

    #[test]
    fn custom_target_name_is_honored() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "ServiceAccountsToEnsure",
            "tenant-sa-custom-target"
        );
        let n = ns("default", NamespacePhase::Active);
        let s = vec![sa("default", "default")];
        // Looking for "build-bot" — not present.
        assert_eq!(evaluate(&n, &s, "build-bot"), ServiceAccountAction::Create);
    }

    #[test]
    fn default_sa_name_constant_matches_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "DefaultServiceAccountName",
            "tenant-sa-const"
        );
        assert_eq!(DEFAULT_SA_NAME, "default");
    }

    #[test]
    fn action_round_trips_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/serviceaccounts_controller.go",
            "ServiceAccountAction",
            "tenant-sa-action-serde"
        );
        for a in [ServiceAccountAction::NoOp, ServiceAccountAction::Create] {
            let s = serde_json::to_string(&a).unwrap();
            let back: ServiceAccountAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
