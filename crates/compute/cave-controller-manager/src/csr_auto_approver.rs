// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CSR auto-approver — `pkg/controller/certificates/approver/sarapprove.go`.
//!
//! Auto-approves kubelet-bootstrap CSRs that satisfy:
//!
//! 1. SignerName is `kubernetes.io/kube-apiserver-client-kubelet`.
//! 2. The requesting user has been granted the
//!    `system:certificates.k8s.io:certificatesigningrequests:nodeclient`
//!    ClusterRoleBinding (initial bootstrap) or
//!    `:selfnodeclient` (renewal).
//! 3. CSR Subject CN matches the requesting username for renewal flow.
//!
//! Mirrors `recognizers` map.

use crate::csr_signer::{
    CsrCondition, CsrSummary, KeyUsage, SIGNER_KUBE_APISERVER_CLIENT_KUBELET,
    SIGNER_KUBELET_SERVING,
};
use crate::csr_signer_deeper::CsrSubject;
use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const NODE_CLIENT_GROUP: &str = "system:bootstrappers:kubeadm:default-node-token";
pub const SELF_NODE_CLIENT_USERNAME_PREFIX: &str = "system:node:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrRequester {
    pub username: String,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoApproveDecision {
    /// Approve via SubjectAccessReview against `:nodeclient`.
    BootstrapNodeClient,
    /// Approve via SubjectAccessReview against `:selfnodeclient`.
    SelfNodeClient,
    /// Auto-approval doesn't apply — leave for manual approval.
    NotEligible(String),
}

/// Bootstrap-node-client recognizer. Returns Eligible when the requester
/// belongs to the bootstrap group AND the subject is a fresh node client
/// (`system:node:<n>` CN, `system:nodes` org, no SANs).
pub fn is_bootstrap_node_client(
    csr: &CsrSummary,
    subj: &CsrSubject,
    requester: &CsrRequester,
) -> bool {
    if csr.signer_name != SIGNER_KUBE_APISERVER_CLIENT_KUBELET {
        return false;
    }
    if !requester.groups.iter().any(|g| g == NODE_CLIENT_GROUP) {
        return false;
    }
    // Subject must satisfy the client-kubelet form (no SANs).
    crate::csr_signer_deeper::validate_apiserver_client_kubelet_subject(subj).is_ok()
}

/// Self-node-client recognizer. The requesting username equals the subject
/// CN — i.e. `system:node:<X>` is renewing its own client cert.
pub fn is_self_node_client(csr: &CsrSummary, subj: &CsrSubject, requester: &CsrRequester) -> bool {
    if csr.signer_name != SIGNER_KUBE_APISERVER_CLIENT_KUBELET {
        return false;
    }
    if !requester
        .username
        .starts_with(SELF_NODE_CLIENT_USERNAME_PREFIX)
    {
        return false;
    }
    if requester.username != subj.common_name {
        return false;
    }
    crate::csr_signer_deeper::validate_apiserver_client_kubelet_subject(subj).is_ok()
}

/// Run all recognizers and emit the auto-approval decision.
pub fn evaluate(
    csr: &CsrSummary,
    subj: &CsrSubject,
    requester: &CsrRequester,
) -> AutoApproveDecision {
    if csr.conditions.contains(&CsrCondition::Approved) {
        return AutoApproveDecision::NotEligible("already approved".into());
    }
    if csr.conditions.contains(&CsrCondition::Denied) {
        return AutoApproveDecision::NotEligible("denied".into());
    }
    if csr.signer_name == SIGNER_KUBELET_SERVING {
        // Server certs: a separate sarapprove rule applies; our auto-approver
        // intentionally only handles client signers.
        return AutoApproveDecision::NotEligible("server signer requires manual approval".into());
    }
    if is_self_node_client(csr, subj, requester) {
        return AutoApproveDecision::SelfNodeClient;
    }
    if is_bootstrap_node_client(csr, subj, requester) {
        return AutoApproveDecision::BootstrapNodeClient;
    }
    AutoApproveDecision::NotEligible("no recognizer matched".into())
}

/// Returns true when the CSR's usages match `kube-apiserver-client-kubelet`
/// (DigitalSignature + KeyEncipherment + ClientAuth).
pub fn client_kubelet_usages_ok(usages: &[KeyUsage]) -> bool {
    let must = [
        KeyUsage::DigitalSignature,
        KeyUsage::KeyEncipherment,
        KeyUsage::ClientAuth,
    ];
    must.iter().all(|u| usages.contains(u)) && !usages.contains(&KeyUsage::ServerAuth)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/approver/sarapprove.go",
    "sarApprover",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn csr(signer: &str, conds: Vec<CsrCondition>) -> CsrSummary {
        CsrSummary {
            name: "csr-1".into(),
            signer_name: signer.into(),
            usages: vec![
                KeyUsage::DigitalSignature,
                KeyUsage::KeyEncipherment,
                KeyUsage::ClientAuth,
            ],
            conditions: conds,
        }
    }
    fn subj_client(node: &str) -> CsrSubject {
        CsrSubject {
            common_name: format!("system:node:{node}"),
            organizations: vec!["system:nodes".into()],
            dns_names: vec![],
            ip_addresses: vec![],
        }
    }
    fn user(name: &str, groups: Vec<&str>) -> CsrRequester {
        CsrRequester {
            username: name.into(),
            groups: groups.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn bootstrap_flow_recognized() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-aa-bootstrap"
        );
        let c = csr(SIGNER_KUBE_APISERVER_CLIENT_KUBELET, vec![]);
        let s = subj_client("worker-1");
        let r = user("system:bootstrap:abc123", vec![NODE_CLIENT_GROUP]);
        assert_eq!(
            evaluate(&c, &s, &r),
            AutoApproveDecision::BootstrapNodeClient
        );
    }

    #[test]
    fn self_node_client_renewal_recognized() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isSelfNodeClientCert",
            "tenant-csr-aa-self"
        );
        let c = csr(SIGNER_KUBE_APISERVER_CLIENT_KUBELET, vec![]);
        let s = subj_client("worker-1");
        let r = user("system:node:worker-1", vec!["system:nodes".into()]);
        assert_eq!(evaluate(&c, &s, &r), AutoApproveDecision::SelfNodeClient);
    }

    #[test]
    fn self_node_client_username_mismatch_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isSelfNodeClientCert",
            "tenant-csr-aa-self-mismatch"
        );
        let c = csr(SIGNER_KUBE_APISERVER_CLIENT_KUBELET, vec![]);
        let s = subj_client("worker-1");
        // User is requesting cert for someone else's node.
        let r = user("system:node:worker-2", vec!["system:nodes".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn already_approved_csr_not_eligible() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-aa-already-approved"
        );
        let c = csr(
            SIGNER_KUBE_APISERVER_CLIENT_KUBELET,
            vec![CsrCondition::Approved],
        );
        let s = subj_client("w");
        let r = user("system:node:w", vec!["system:nodes".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn denied_csr_not_eligible() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-aa-denied"
        );
        let c = csr(
            SIGNER_KUBE_APISERVER_CLIENT_KUBELET,
            vec![CsrCondition::Denied],
        );
        let s = subj_client("w");
        let r = user("system:node:w", vec!["system:nodes".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn server_signer_not_handled_by_auto_approver() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "recognizers",
            "tenant-csr-aa-server"
        );
        let c = csr(SIGNER_KUBELET_SERVING, vec![]);
        let s = subj_client("w");
        let r = user("system:node:w", vec!["system:nodes".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn bootstrap_without_group_membership_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-aa-no-group"
        );
        let c = csr(SIGNER_KUBE_APISERVER_CLIENT_KUBELET, vec![]);
        let s = subj_client("worker-1");
        let r = user("system:bootstrap:abc", vec!["other-group".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn unknown_signer_not_eligible() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "recognizers",
            "tenant-csr-aa-unknown-signer"
        );
        let c = csr("example.com/custom", vec![]);
        let s = subj_client("w");
        let r = user("system:node:w", vec!["system:nodes".into()]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn client_kubelet_usages_require_client_auth_only() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isUsageAllowed",
            "tenant-csr-aa-usages"
        );
        assert!(client_kubelet_usages_ok(&[
            KeyUsage::DigitalSignature,
            KeyUsage::KeyEncipherment,
            KeyUsage::ClientAuth,
        ]));
        // ServerAuth disqualifies.
        assert!(!client_kubelet_usages_ok(&[
            KeyUsage::DigitalSignature,
            KeyUsage::KeyEncipherment,
            KeyUsage::ClientAuth,
            KeyUsage::ServerAuth,
        ]));
    }

    #[test]
    fn bootstrap_subject_with_extras_rejected_by_subject_validation() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-aa-bootstrap-bad-subject"
        );
        let c = csr(SIGNER_KUBE_APISERVER_CLIENT_KUBELET, vec![]);
        let mut s = subj_client("worker-1");
        s.dns_names.push("evil.example.com".into());
        let r = user("system:bootstrap:abc", vec![NODE_CLIENT_GROUP]);
        match evaluate(&c, &s, &r) {
            AutoApproveDecision::NotEligible(_) => {}
            other => panic!("expected NotEligible, got {other:?}"),
        }
    }

    #[test]
    fn auto_approve_decision_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "AutoApproveDecision",
            "tenant-csr-aa-decision-serde"
        );
        for d in [
            AutoApproveDecision::BootstrapNodeClient,
            AutoApproveDecision::SelfNodeClient,
            AutoApproveDecision::NotEligible("x".into()),
        ] {
            let s = serde_json::to_string(&d).unwrap();
            let back: AutoApproveDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(d, back);
        }
    }
}
