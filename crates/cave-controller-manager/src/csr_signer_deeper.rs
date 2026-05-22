// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CSR signer deeper — `pkg/controller/certificates/signer/signer.go`.
//!
//! Extends [`crate::csr_signer`] with:
//!
//! * `expirationSeconds` enforcement — signer-specific cap on the cert's
//!   `NotAfter - NotBefore`.
//! * Multi-condition resolution — when a CSR carries both `Approved` and
//!   `Denied`, `Denied` wins (latest wins regardless of order).
//! * Subject validation for `kubernetes.io/kubelet-serving` —
//!   CommonName must be `system:node:<nodeName>` and Organization must be
//!   `system:nodes`. Mirrors `regularSelfNodeServerSignerName`.

use crate::csr_signer::{CsrCondition, CsrSummary, KeyUsage, SIGNER_KUBELET_SERVING};
use crate::types::Cite;
use serde::{Deserialize, Serialize};

/// Default cert duration when `expirationSeconds` not requested.
pub const DEFAULT_DURATION_SEC: u32 = 365 * 24 * 60 * 60;

/// Per-signer maximum duration cap. Mirrors the maps in
/// `pkg/controller/certificates/signer/signer.go::DurationFromExpirationSeconds`.
pub fn max_duration_sec(signer_name: &str) -> u32 {
    match signer_name {
        SIGNER_KUBELET_SERVING => 365 * 24 * 60 * 60,
        // apiserver-client signers cap at 1 year; legacy uncapped → 10y.
        "kubernetes.io/legacy-unknown" => 10 * 365 * 24 * 60 * 60,
        _ => 365 * 24 * 60 * 60,
    }
}

/// Clamp the requested expiration_sec to the signer's cap.
pub fn clamp_expiration(signer_name: &str, requested_sec: Option<u32>) -> u32 {
    let req = requested_sec.unwrap_or(DEFAULT_DURATION_SEC);
    req.min(max_duration_sec(signer_name))
}

/// True when the CSR's denied condition wins over approved.
/// Mirrors `getCertApprovalCondition` (the latest condition wins; here we
/// treat denied as latching once observed).
pub fn denied_wins(csr: &CsrSummary) -> bool {
    csr.conditions.contains(&CsrCondition::Denied)
}

/// Subject information extracted from the PEM CSR. Mirrors the parsing
/// done by `cfssl_signer.go::parseCSR`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CsrSubject {
    pub common_name: String,
    pub organizations: Vec<String>,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
}

/// Validate the subject for `kubelet-serving`. CN must be
/// `system:node:<nodeName>` and Org must include `system:nodes`.
pub fn validate_kubelet_serving_subject(subj: &CsrSubject) -> Result<String, &'static str> {
    if !subj.common_name.starts_with("system:node:") {
        return Err("CommonName must be system:node:<nodeName>");
    }
    let node = subj.common_name.trim_start_matches("system:node:");
    if node.is_empty() {
        return Err("nodeName is empty");
    }
    if !subj.organizations.iter().any(|o| o == "system:nodes") {
        return Err("Organization must include system:nodes");
    }
    if subj.dns_names.is_empty() && subj.ip_addresses.is_empty() {
        return Err("kubelet-serving subject must have SANs");
    }
    Ok(node.to_string())
}

/// Validate the subject for `apiserver-client-kubelet`. CN must be
/// `system:node:<nodeName>` and Org must include `system:nodes`; SANs
/// are forbidden.
pub fn validate_apiserver_client_kubelet_subject(
    subj: &CsrSubject,
) -> Result<String, &'static str> {
    if !subj.common_name.starts_with("system:node:") {
        return Err("CommonName must be system:node:<nodeName>");
    }
    let node = subj.common_name.trim_start_matches("system:node:");
    if node.is_empty() {
        return Err("nodeName is empty");
    }
    if !subj.organizations.iter().any(|o| o == "system:nodes") {
        return Err("Organization must include system:nodes");
    }
    if !subj.dns_names.is_empty() || !subj.ip_addresses.is_empty() {
        return Err("apiserver-client-kubelet must not have SANs");
    }
    Ok(node.to_string())
}

/// Returns true if the CSR's usages match the `kubelet-serving` requirement.
pub fn kubelet_serving_usages_ok(usages: &[KeyUsage]) -> bool {
    let must_have = [KeyUsage::DigitalSignature, KeyUsage::ServerAuth];
    must_have.iter().all(|u| usages.contains(u))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/signer/signer.go",
    "DurationFromExpirationSeconds",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn subj(cn: &str, org: Vec<&str>) -> CsrSubject {
        CsrSubject {
            common_name: cn.into(),
            organizations: org.into_iter().map(String::from).collect(),
            dns_names: vec!["node-a.local".into()],
            ip_addresses: vec!["10.0.0.5".into()],
        }
    }

    #[test]
    fn clamp_caps_at_signer_max() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "DurationFromExpirationSeconds",
            "tenant-csr-deep-clamp"
        );
        let big = 100 * 365 * 24 * 60 * 60;
        assert_eq!(
            clamp_expiration(SIGNER_KUBELET_SERVING, Some(big)),
            max_duration_sec(SIGNER_KUBELET_SERVING)
        );
    }

    #[test]
    fn clamp_default_when_not_specified() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "DurationFromExpirationSeconds",
            "tenant-csr-deep-default"
        );
        assert_eq!(
            clamp_expiration(SIGNER_KUBELET_SERVING, None),
            DEFAULT_DURATION_SEC
        );
    }

    #[test]
    fn legacy_unknown_max_is_ten_years() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "DurationFromExpirationSeconds",
            "tenant-csr-deep-legacy-max"
        );
        assert_eq!(
            max_duration_sec("kubernetes.io/legacy-unknown"),
            10 * 365 * 24 * 60 * 60
        );
    }

    #[test]
    fn denied_wins_when_present_alongside_approved() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "getCertApprovalCondition",
            "tenant-csr-deep-denied-wins"
        );
        let csr = CsrSummary {
            name: "x".into(),
            signer_name: SIGNER_KUBELET_SERVING.into(),
            usages: vec![],
            conditions: vec![CsrCondition::Approved, CsrCondition::Denied],
        };
        assert!(denied_wins(&csr));
    }

    #[test]
    fn approved_alone_does_not_count_as_denied() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "getCertApprovalCondition",
            "tenant-csr-deep-approved-alone"
        );
        let csr = CsrSummary {
            name: "x".into(),
            signer_name: SIGNER_KUBELET_SERVING.into(),
            usages: vec![],
            conditions: vec![CsrCondition::Approved],
        };
        assert!(!denied_wins(&csr));
    }

    #[test]
    fn kubelet_serving_subject_extracts_node_name() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeServingCert",
            "tenant-csr-deep-subj-ok"
        );
        let s = subj("system:node:worker-1", vec!["system:nodes"]);
        assert_eq!(validate_kubelet_serving_subject(&s).unwrap(), "worker-1");
    }

    #[test]
    fn kubelet_serving_subject_rejects_bad_cn() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeServingCert",
            "tenant-csr-deep-subj-cn"
        );
        let s = subj("admin", vec!["system:nodes"]);
        assert!(validate_kubelet_serving_subject(&s).is_err());
    }

    #[test]
    fn kubelet_serving_subject_rejects_missing_org() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeServingCert",
            "tenant-csr-deep-subj-org"
        );
        let s = subj("system:node:n1", vec!["other"]);
        assert!(validate_kubelet_serving_subject(&s).is_err());
    }

    #[test]
    fn kubelet_serving_subject_rejects_no_sans() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeServingCert",
            "tenant-csr-deep-subj-no-sans"
        );
        let mut s = subj("system:node:n1", vec!["system:nodes"]);
        s.dns_names.clear();
        s.ip_addresses.clear();
        assert!(validate_kubelet_serving_subject(&s).is_err());
    }

    #[test]
    fn apiserver_client_kubelet_subject_rejects_sans() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-deep-client-subj-sans"
        );
        let s = subj("system:node:n1", vec!["system:nodes"]);
        // SANs are populated by default subj() — must reject.
        assert!(validate_apiserver_client_kubelet_subject(&s).is_err());
    }

    #[test]
    fn apiserver_client_kubelet_subject_accepts_clean() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/approver/sarapprove.go",
            "isNodeClientCert",
            "tenant-csr-deep-client-subj-ok"
        );
        let mut s = subj("system:node:n1", vec!["system:nodes"]);
        s.dns_names.clear();
        s.ip_addresses.clear();
        assert_eq!(validate_apiserver_client_kubelet_subject(&s).unwrap(), "n1");
    }

    #[test]
    fn kubelet_serving_usages_require_serverauth() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/signer.go",
            "isUsageAllowed",
            "tenant-csr-deep-usage-serverauth"
        );
        assert!(kubelet_serving_usages_ok(&[
            KeyUsage::DigitalSignature,
            KeyUsage::ServerAuth
        ]));
        assert!(!kubelet_serving_usages_ok(&[KeyUsage::ClientAuth]));
    }
}
