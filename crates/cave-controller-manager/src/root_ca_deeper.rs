// SPDX-License-Identifier: AGPL-3.0-or-later
//! Root CA publisher deeper — `pkg/controller/certificates/rootcacertpublisher/publisher.go`.
//!
//! Adds:
//!
//! * **Mutation detection** — if a third party rewrites
//!   `data["ca.crt"]` to a non-matching value, the controller observes the
//!   change via watch and rewrites it. Mirrors `syncHandler` re-entering
//!   on every ConfigMap event.
//! * **Multi-key rotation** — during CA rotation, the bundle may contain
//!   multiple PEM-encoded certificates concatenated; equality is by bytes.
//! * **OwnerReferences preservation** — the publisher MUST NOT clobber
//!   existing owner refs added by other controllers.
//! * **Finalizer hand-off** — namespace deletion lifecycle.

use crate::root_ca_publisher::{NamespacePhase, ObservedConfigMap, PublishAction};
use crate::types::Cite;
use serde::{Deserialize, Serialize};

/// Detect tampering: returns true when the observed `ca.crt` is non-empty
/// but doesn't match the cluster bundle byte-for-byte.
pub fn ca_crt_was_mutated(observed: Option<&ObservedConfigMap>, expected: &str) -> bool {
    match observed.and_then(|cm| cm.ca_crt.as_deref()) {
        Some(v) => v != expected,
        None => false,
    }
}

/// Compare two CA bundle strings. Bundles can carry multiple PEM blocks; we
/// canonicalize by trimming trailing whitespace and stripping CR before
/// comparison. Mirrors `bytes.Equal(observed, expected)` in upstream after
/// the kubelet/kube-apiserver normalize PEM input.
pub fn bundles_equal(a: &str, b: &str) -> bool {
    canonicalize(a) == canonicalize(b)
}

fn canonicalize(s: &str) -> String {
    s.replace('\r', "").trim_end().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigMapMeta {
    pub name: String,
    pub namespace: String,
    pub owner_uids: Vec<String>,
    /// Finalizers from `metadata.finalizers[]` — not modified by the publisher.
    pub finalizers: Vec<String>,
}

/// Compute the patch the publisher would emit. It MUST preserve
/// owner_uids and finalizers; only `data["ca.crt"]` is touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmPatch {
    pub set_ca_crt: String,
    pub preserved_owners: Vec<String>,
    pub preserved_finalizers: Vec<String>,
}

pub fn compute_patch(meta: &ConfigMapMeta, expected_ca: &str) -> CmPatch {
    CmPatch {
        set_ca_crt: expected_ca.to_string(),
        preserved_owners: meta.owner_uids.clone(),
        preserved_finalizers: meta.finalizers.clone(),
    }
}

/// Decide what to do when the namespace itself transitions to Terminating.
/// Mirrors the GC-driven cleanup: the publisher does NOT remove the
/// ConfigMap; namespace deletion handles it.
pub fn evaluate_terminating(_cm: Option<&ObservedConfigMap>) -> PublishAction {
    PublishAction::NoOp
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/rootcacertpublisher/publisher.go",
    "syncHandler",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cm(ns: &str, ca: Option<&str>) -> ObservedConfigMap {
        ObservedConfigMap {
            namespace: ns.into(),
            ca_crt: ca.map(|s| s.to_string()),
        }
    }

    #[test]
    fn mutated_ca_returns_true() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-mutated"
        );
        let observed = cm("default", Some("OLD"));
        assert!(ca_crt_was_mutated(Some(&observed), "NEW"));
    }

    #[test]
    fn matching_ca_not_mutated() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-match"
        );
        let observed = cm("default", Some("CA"));
        assert!(!ca_crt_was_mutated(Some(&observed), "CA"));
    }

    #[test]
    fn missing_ca_field_not_treated_as_mutation() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-missing"
        );
        let observed = cm("default", None);
        // Missing → publisher uses Update path (caller decides), not "mutated".
        assert!(!ca_crt_was_mutated(Some(&observed), "CA"));
    }

    #[test]
    fn bundles_equal_ignores_trailing_whitespace() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-bundles-trim"
        );
        assert!(bundles_equal("CA-A\n", "CA-A"));
        assert!(bundles_equal("CA-A\r\n", "CA-A\n"));
    }

    #[test]
    fn bundles_equal_byte_for_byte_when_trimmed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-bundles-strict"
        );
        assert!(!bundles_equal("CA-A", "CA-B"));
    }

    #[test]
    fn multi_pem_bundle_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-multi-pem"
        );
        let bundle = "-----BEGIN CERTIFICATE-----\nA\n-----END CERTIFICATE-----\n\
                      -----BEGIN CERTIFICATE-----\nB\n-----END CERTIFICATE-----";
        assert!(bundles_equal(bundle, &format!("{bundle}\n")));
    }

    #[test]
    fn compute_patch_preserves_owner_refs() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-preserve-owners"
        );
        let meta = ConfigMapMeta {
            name: "kube-root-ca.crt".into(),
            namespace: "default".into(),
            owner_uids: vec!["uid-1".into(), "uid-2".into()],
            finalizers: vec![],
        };
        let p = compute_patch(&meta, "CA");
        assert_eq!(p.preserved_owners, vec!["uid-1", "uid-2"]);
        assert_eq!(p.set_ca_crt, "CA");
    }

    #[test]
    fn compute_patch_preserves_finalizers() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-preserve-finalizers"
        );
        let meta = ConfigMapMeta {
            name: "kube-root-ca.crt".into(),
            namespace: "default".into(),
            owner_uids: vec![],
            finalizers: vec!["x.example.com/cleanup".into()],
        };
        let p = compute_patch(&meta, "CA");
        assert_eq!(p.preserved_finalizers, vec!["x.example.com/cleanup"]);
    }

    #[test]
    fn evaluate_terminating_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-terminating"
        );
        let _phase = NamespacePhase::Terminating;
        // Even with an existing ConfigMap, terminating namespace = NoOp.
        let observed = cm("ending", Some("anything"));
        assert_eq!(evaluate_terminating(Some(&observed)), PublishAction::NoOp);
        assert_eq!(evaluate_terminating(None), PublishAction::NoOp);
    }

    #[test]
    fn cm_patch_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "CmPatch",
            "tenant-rca2-patch-serde"
        );
        let p = CmPatch {
            set_ca_crt: "CA".into(),
            preserved_owners: vec!["u".into()],
            preserved_finalizers: vec!["f".into()],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: CmPatch = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn ca_crt_mutation_with_no_observed_returns_false() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/rootcacertpublisher/publisher.go",
            "syncHandler",
            "tenant-rca2-no-observed"
        );
        // No CM at all → no mutation; caller's path is Create, not Update.
        assert!(!ca_crt_was_mutated(None, "CA"));
    }
}
