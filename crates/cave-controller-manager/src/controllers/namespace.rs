//! Namespace controller — drives Terminating namespaces through their
//! finalizer chain and removes them once empty.
//!
//! Upstream: pkg/controller/namespace/namespace_controller.go::syncNamespace

use crate::models::NamespaceObs;

/// Outcome of a single namespace reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceAction {
    NoChange,
    /// Pop the named finalizer from the namespace's list and persist.
    PopFinalizer(String),
    /// Finalizers are exhausted — delete the namespace object.
    Delete,
}

/// Reconcile a Namespace. If the namespace is Terminating with finalizers,
/// pop the head; if it's Terminating with none left, request deletion.
pub fn reconcile_namespace(ns: &NamespaceObs) -> NamespaceAction {
    if ns.deletion_timestamp.is_none() {
        return NamespaceAction::NoChange;
    }
    match ns.finalizers.first() {
        Some(f) => NamespaceAction::PopFinalizer(f.clone()),
        None => NamespaceAction::Delete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn ns(finalizers: Vec<&str>, terminating: bool) -> NamespaceObs {
        NamespaceObs {
            name: "ns1".into(),
            phase: if terminating { "Terminating".into() } else { "Active".into() },
            finalizers: finalizers.into_iter().map(String::from).collect(),
            deletion_timestamp: terminating.then(Utc::now),
        }
    }

    /// Port of `TestNamespaceFinalizers` — terminating namespace with finalizers
    /// returns `PopFinalizer`; once empty, returns `Delete`.
    #[test]
    fn test_namespace_finalizers() {
        let with_finalizer = ns(vec!["kubernetes"], true);
        assert_eq!(
            reconcile_namespace(&with_finalizer),
            NamespaceAction::PopFinalizer("kubernetes".into()),
        );
        let empty = ns(vec![], true);
        assert_eq!(reconcile_namespace(&empty), NamespaceAction::Delete);
    }

    #[test]
    fn test_namespace_active_no_change() {
        let active = ns(vec![], false);
        assert_eq!(reconcile_namespace(&active), NamespaceAction::NoChange);
    }
}
