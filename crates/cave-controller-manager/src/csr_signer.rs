// SPDX-License-Identifier: AGPL-3.0-or-later
//! CertificateSigningRequest signer — `pkg/controller/certificates/signer/cfssl_signer.go`.
//!
//! Each CSR has a `signerName` and a list of `usages`. The signer:
//!
//! 1. Verifies the CSR has been **Approved** (and not Denied/Failed).
//! 2. Verifies `signerName` matches one of the recognised signers.
//! 3. Verifies `usages[]` are admissible for the signer.
//! 4. Signs and adds the `Issued` condition with the resulting cert.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CsrCondition {
    Approved,
    Denied,
    Failed,
    Issued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyUsage {
    DigitalSignature,
    KeyEncipherment,
    ClientAuth,
    ServerAuth,
}

/// Recognised signer names. Mirrors the constants in
/// `pkg/apis/certificates/types.go`.
pub const SIGNER_KUBELET_SERVING: &str = "kubernetes.io/kubelet-serving";
pub const SIGNER_KUBE_APISERVER_CLIENT: &str = "kubernetes.io/kube-apiserver-client";
pub const SIGNER_KUBE_APISERVER_CLIENT_KUBELET: &str = "kubernetes.io/kube-apiserver-client-kubelet";
pub const SIGNER_LEGACY_UNKNOWN: &str = "kubernetes.io/legacy-unknown";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrSummary {
    pub name: String,
    pub signer_name: String,
    pub usages: Vec<KeyUsage>,
    pub conditions: Vec<CsrCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignerAction {
    /// CSR is denied or failed — never sign.
    NoOp,
    /// CSR is not approved yet — skip; revisit on next approval.
    NotApproved,
    /// CSR is approved + valid for this signer's usages — sign now.
    Sign,
    /// CSR is approved but the usages or signerName are unsupported.
    Reject(String),
    /// CSR has already been issued — nothing to do.
    AlreadyIssued,
}

/// Map a signer name to the usages it permits. Mirrors the per-signer maps in
/// `pkg/controller/certificates/signer/known_signers.go`.
pub fn allowed_usages(signer_name: &str) -> Option<&'static [KeyUsage]> {
    match signer_name {
        SIGNER_KUBELET_SERVING => Some(&[
            KeyUsage::DigitalSignature,
            KeyUsage::KeyEncipherment,
            KeyUsage::ServerAuth,
        ]),
        SIGNER_KUBE_APISERVER_CLIENT | SIGNER_KUBE_APISERVER_CLIENT_KUBELET => Some(&[
            KeyUsage::DigitalSignature,
            KeyUsage::KeyEncipherment,
            KeyUsage::ClientAuth,
        ]),
        // legacy signs whatever was requested.
        SIGNER_LEGACY_UNKNOWN => Some(&[
            KeyUsage::DigitalSignature,
            KeyUsage::KeyEncipherment,
            KeyUsage::ClientAuth,
            KeyUsage::ServerAuth,
        ]),
        _ => None,
    }
}

fn has_cond(csr: &CsrSummary, c: CsrCondition) -> bool {
    csr.conditions.contains(&c)
}

/// Decide what the signer should do with `csr`. Mirrors `signer.handle`.
pub fn evaluate(csr: &CsrSummary) -> SignerAction {
    if has_cond(csr, CsrCondition::Issued) {
        return SignerAction::AlreadyIssued;
    }
    if has_cond(csr, CsrCondition::Denied) || has_cond(csr, CsrCondition::Failed) {
        return SignerAction::NoOp;
    }
    if !has_cond(csr, CsrCondition::Approved) {
        return SignerAction::NotApproved;
    }
    let Some(allowed) = allowed_usages(&csr.signer_name) else {
        return SignerAction::Reject("unknown signerName".into());
    };
    for u in &csr.usages {
        if !allowed.contains(u) {
            return SignerAction::Reject("usage not allowed for signer".into());
        }
    }
    SignerAction::Sign
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/signer/cfssl_signer.go",
    "Signer",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn csr(signer: &str, usages: Vec<KeyUsage>, conds: Vec<CsrCondition>) -> CsrSummary {
        CsrSummary {
            name: "csr-1".into(),
            signer_name: signer.into(),
            usages,
            conditions: conds,
        }
    }

    #[test]
    fn approved_kubelet_serving_signs() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-sign-kubelet-serving"
        );
        let c = csr(
            SIGNER_KUBELET_SERVING,
            vec![KeyUsage::DigitalSignature, KeyUsage::ServerAuth],
            vec![CsrCondition::Approved],
        );
        assert_eq!(evaluate(&c), SignerAction::Sign);
    }

    #[test]
    fn approved_apiserver_client_signs() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-sign-apiserver-client"
        );
        let c = csr(
            SIGNER_KUBE_APISERVER_CLIENT,
            vec![KeyUsage::DigitalSignature, KeyUsage::ClientAuth],
            vec![CsrCondition::Approved],
        );
        assert_eq!(evaluate(&c), SignerAction::Sign);
    }

    #[test]
    fn unapproved_csr_is_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-not-approved"
        );
        let c = csr(SIGNER_KUBELET_SERVING, vec![KeyUsage::ServerAuth], vec![]);
        assert_eq!(evaluate(&c), SignerAction::NotApproved);
    }

    #[test]
    fn denied_csr_is_no_op() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-denied"
        );
        let c = csr(
            SIGNER_KUBELET_SERVING,
            vec![KeyUsage::ServerAuth],
            vec![CsrCondition::Approved, CsrCondition::Denied],
        );
        assert_eq!(evaluate(&c), SignerAction::NoOp);
    }

    #[test]
    fn failed_csr_is_no_op() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-failed"
        );
        let c = csr(
            SIGNER_KUBELET_SERVING,
            vec![KeyUsage::ServerAuth],
            vec![CsrCondition::Approved, CsrCondition::Failed],
        );
        assert_eq!(evaluate(&c), SignerAction::NoOp);
    }

    #[test]
    fn already_issued_short_circuits() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-issued"
        );
        let c = csr(
            SIGNER_KUBELET_SERVING,
            vec![KeyUsage::ServerAuth],
            vec![CsrCondition::Approved, CsrCondition::Issued],
        );
        assert_eq!(evaluate(&c), SignerAction::AlreadyIssued);
    }

    #[test]
    fn unknown_signer_rejects() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "handle",
            "tenant-csr-unknown-signer"
        );
        let c = csr(
            "example.com/custom-signer",
            vec![KeyUsage::ClientAuth],
            vec![CsrCondition::Approved],
        );
        match evaluate(&c) {
            SignerAction::Reject(_) => {}
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn server_auth_for_apiserver_client_rejects() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/known_signers.go",
            "isUsageAllowed",
            "tenant-csr-bad-usage"
        );
        let c = csr(
            SIGNER_KUBE_APISERVER_CLIENT,
            vec![KeyUsage::ServerAuth], // disallowed for client signer
            vec![CsrCondition::Approved],
        );
        match evaluate(&c) {
            SignerAction::Reject(_) => {}
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn legacy_unknown_admits_any_usage() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/known_signers.go",
            "legacyUnknown",
            "tenant-csr-legacy-any"
        );
        let c = csr(
            SIGNER_LEGACY_UNKNOWN,
            vec![KeyUsage::ClientAuth, KeyUsage::ServerAuth, KeyUsage::DigitalSignature],
            vec![CsrCondition::Approved],
        );
        assert_eq!(evaluate(&c), SignerAction::Sign);
    }

    #[test]
    fn allowed_usages_returns_none_for_unknown_signer() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/known_signers.go",
            "lookup",
            "tenant-csr-allowed-unknown"
        );
        assert!(allowed_usages("foo").is_none());
        assert!(allowed_usages(SIGNER_KUBELET_SERVING).is_some());
    }

    #[test]
    fn signer_constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/certificates/types.go",
            "BuiltInSignerNames",
            "tenant-csr-signer-const"
        );
        assert_eq!(SIGNER_KUBELET_SERVING, "kubernetes.io/kubelet-serving");
        assert_eq!(SIGNER_KUBE_APISERVER_CLIENT, "kubernetes.io/kube-apiserver-client");
        assert_eq!(
            SIGNER_KUBE_APISERVER_CLIENT_KUBELET,
            "kubernetes.io/kube-apiserver-client-kubelet"
        );
        assert_eq!(SIGNER_LEGACY_UNKNOWN, "kubernetes.io/legacy-unknown");
    }

    #[test]
    fn signer_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "SignerAction",
            "tenant-csr-action-serde"
        );
        for a in [
            SignerAction::NoOp,
            SignerAction::NotApproved,
            SignerAction::Sign,
            SignerAction::Reject("x".into()),
            SignerAction::AlreadyIssued,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: SignerAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
