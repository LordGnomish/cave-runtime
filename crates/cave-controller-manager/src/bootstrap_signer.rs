// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bootstrap-token signer — `pkg/controller/bootstrap/bootstrapsigner.go`.
//!
//! Maintains a JWT-style signature on the `cluster-info` ConfigMap in
//! `kube-public` so unauthenticated clients can verify its contents
//! against any active bootstrap token they hold.
//!
//! Key behaviors:
//!
//! * For each enabled bootstrap token (`Secret` of type
//!   `bootstrap.kubernetes.io/token`, with usage `signing` enabled and
//!   not expired), emit one signature in
//!   `cluster-info.data["jws-kubeconfig-<token-id>"]`.
//! * Strip signatures whose token has expired or been deleted.
//! * Rebuild signatures whenever `cluster-info.data["kubeconfig"]` changes.

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapToken {
    pub token_id: String,
    pub secret: String,
    pub usage_signing: bool,
    pub expired: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub kubeconfig: String,
    /// `data["jws-kubeconfig-<id>"]` entries keyed by token id.
    pub signatures: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignerAction {
    /// Add or refresh a signature for the named token id.
    AddSignature(String),
    /// Remove a signature for the named token id.
    RemoveSignature(String),
}

/// Compute the diff between `current` cluster-info signatures and the
/// signatures the controller would have written for `tokens`.
pub fn reconcile(current: &ClusterInfo, tokens: &[BootstrapToken]) -> Vec<SignerAction> {
    let mut want: BTreeMap<String, ()> = BTreeMap::new();
    for t in tokens {
        if t.usage_signing && !t.expired {
            want.insert(t.token_id.clone(), ());
        }
    }
    let mut out = Vec::new();
    for token_id in want.keys() {
        if !current.signatures.contains_key(token_id) {
            out.push(SignerAction::AddSignature(token_id.clone()));
        }
    }
    for token_id in current.signatures.keys() {
        if !want.contains_key(token_id) {
            out.push(SignerAction::RemoveSignature(token_id.clone()));
        }
    }
    out.sort_by(|a, b| match (a, b) {
        (SignerAction::AddSignature(x), SignerAction::AddSignature(y))
        | (SignerAction::RemoveSignature(x), SignerAction::RemoveSignature(y)) => x.cmp(y),
        (SignerAction::AddSignature(_), SignerAction::RemoveSignature(_)) => std::cmp::Ordering::Less,
        (SignerAction::RemoveSignature(_), SignerAction::AddSignature(_)) => std::cmp::Ordering::Greater,
    });
    out
}

/// True when a kubeconfig change requires re-signing every active token.
pub fn kubeconfig_changed(prev: &ClusterInfo, next: &ClusterInfo) -> bool {
    prev.kubeconfig != next.kubeconfig
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/bootstrap/bootstrapsigner.go",
    "Signer",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn token(id: &str, signing: bool, expired: bool) -> BootstrapToken {
        BootstrapToken {
            token_id: id.into(),
            secret: format!("secret-{id}"),
            usage_signing: signing,
            expired,
        }
    }
    fn ci(sigs: &[(&str, &str)]) -> ClusterInfo {
        ClusterInfo {
            kubeconfig: "...".into(),
            signatures: sigs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn fresh_token_with_no_signatures_emits_add() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-add"
        );
        let actions = reconcile(&ci(&[]), &[token("abcdef", true, false)]);
        assert_eq!(actions, vec![SignerAction::AddSignature("abcdef".into())]);
    }

    #[test]
    fn expired_token_with_signature_emits_remove() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-remove-expired"
        );
        let actions = reconcile(
            &ci(&[("abcdef", "sig")]),
            &[token("abcdef", true, true)],
        );
        assert_eq!(actions, vec![SignerAction::RemoveSignature("abcdef".into())]);
    }

    #[test]
    fn token_without_signing_usage_is_ignored() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-no-usage"
        );
        let actions = reconcile(&ci(&[]), &[token("abcdef", false, false)]);
        assert!(actions.is_empty());
    }

    #[test]
    fn signature_for_unknown_token_is_removed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-stale"
        );
        let actions = reconcile(&ci(&[("ghost", "sig")]), &[]);
        assert_eq!(actions, vec![SignerAction::RemoveSignature("ghost".into())]);
    }

    #[test]
    fn no_diff_yields_empty_actions() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-noop"
        );
        let actions = reconcile(
            &ci(&[("abcdef", "sig")]),
            &[token("abcdef", true, false)],
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn add_then_remove_produces_combined_diff() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "signConfigMap",
            "tenant-bs-add-and-remove"
        );
        let actions = reconcile(
            &ci(&[("oldid", "sig")]),
            &[token("newid", true, false)],
        );
        assert!(actions.contains(&SignerAction::AddSignature("newid".into())));
        assert!(actions.contains(&SignerAction::RemoveSignature("oldid".into())));
    }

    #[test]
    fn kubeconfig_changed_detects_text_diff() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "syncCluster",
            "tenant-bs-kc-changed"
        );
        let mut a = ci(&[]);
        a.kubeconfig = "X".into();
        let mut b = ci(&[]);
        b.kubeconfig = "Y".into();
        assert!(kubeconfig_changed(&a, &b));
        assert!(!kubeconfig_changed(&a, &a.clone()));
    }

    #[test]
    fn signer_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/bootstrap/bootstrapsigner.go",
            "SignerAction",
            "tenant-bs-action-serde"
        );
        for a in [
            SignerAction::AddSignature("a".into()),
            SignerAction::RemoveSignature("b".into()),
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: SignerAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
