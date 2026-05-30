// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cert-manager CertificateRequest approval-condition admission validation.
//!
//! Faithful line-port of cert-manager v1.17.2
//! `internal/apis/certmanager/validation/certificaterequest.go`:
//!   - `ValidateCertificateRequestApprovalCondition` (lines 123-171)
//!   - `ValidateUpdateCertificateRequestApprovalCondition` (lines 173-199)
//!   - `getCertificateRequestCondition` (lines 201-208)
//!
//! These rules guarantee approval integrity: at most one `Approved`/`Denied`
//! condition each, each may only be set to `True`, `Approved` and `Denied`
//! cannot coexist, and once set neither may be modified on update.
//!
//! cert-manager runs these inside its webhook binary. The webhook transport
//! (HTTPS server, AdmissionReview decode, K8s wiring) is genuinely cross-crate
//! (cave-admission); the validation algorithm here is pure in-crate logic.

use crate::webhook_validation::ValidationError;

/// Cite: cert-manager `CertificateRequestConditionType` (subset relevant to
/// approval: `Approved` / `Denied`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrConditionType {
    Approved,
    Denied,
    /// Any other condition type (e.g. `Ready`, `InvalidRequest`) — ignored by
    /// approval validation.
    Other,
}

impl CrConditionType {
    fn label(&self) -> &'static str {
        match self {
            CrConditionType::Approved => "Approved",
            CrConditionType::Denied => "Denied",
            CrConditionType::Other => "Other",
        }
    }
}

/// Cite: cert-manager `cmmeta.ConditionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrConditionStatus {
    True,
    False,
    Unknown,
}

/// Cite: cert-manager `CertificateRequestCondition` (the fields compared by
/// `reflect.DeepEqual` during update validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrCondition {
    pub kind: CrConditionType,
    pub status: CrConditionStatus,
    pub reason: String,
    pub message: String,
}

/// Cite: cert-manager `ValidateCertificateRequestApprovalCondition`
/// (certificaterequest.go:123-171). Ensures only a single `Approved` or
/// `Denied` condition may exist and that it is set to `True`.
pub fn validate_cr_approval_condition(
    cr_conds: &[CrCondition],
    fld: &str,
) -> Vec<ValidationError> {
    let mut el = Vec::new();

    let approved: Vec<&CrCondition> = cr_conds
        .iter()
        .filter(|c| c.kind == CrConditionType::Approved)
        .collect();
    let denied: Vec<&CrCondition> = cr_conds
        .iter()
        .filter(|c| c.kind == CrConditionType::Denied)
        .collect();

    for (cond_type, found) in [
        (CrConditionType::Approved, &approved),
        (CrConditionType::Denied, &denied),
    ] {
        if found.is_empty() {
            continue;
        }

        if found.len() > 1 {
            // Cite: certificaterequest.go:153-155.
            el.push(ValidationError {
                field: fld.to_string(),
                message: format!("multiple \"{}\" conditions present", cond_type.label()),
            });
            continue;
        }

        let first = found[0];
        if first.status != CrConditionStatus::True {
            // Cite: certificaterequest.go:158-163.
            el.push(ValidationError {
                field: format!("{fld}.{}", first.kind.label()),
                message: format!(
                    "\"{}\" condition may only be set to True",
                    cond_type.label()
                ),
            });
            continue;
        }
    }

    // Cite: certificaterequest.go:166-168.
    if !denied.is_empty() && !approved.is_empty() {
        el.push(ValidationError {
            field: fld.to_string(),
            message: "both 'Denied' and 'Approved' conditions cannot coexist".to_string(),
        });
    }

    el
}

/// Cite: cert-manager `ValidateUpdateCertificateRequestApprovalCondition`
/// (certificaterequest.go:173-199). Ensures `Approved`/`Denied` conditions are
/// not modified once set, then runs base approval validation on the new
/// conditions.
pub fn validate_update_cr_approval_condition(
    old_cr_conds: &[CrCondition],
    new_cr_conds: &[CrCondition],
    fld: &str,
) -> Vec<ValidationError> {
    let mut el = Vec::new();

    let old_denied = get_cr_condition(old_cr_conds, CrConditionType::Denied);
    let old_approved = get_cr_condition(old_cr_conds, CrConditionType::Approved);

    // Cite: certificaterequest.go:184-189.
    if let Some(old_approved) = old_approved {
        if Some(old_approved) != get_cr_condition(new_cr_conds, CrConditionType::Approved) {
            el.push(ValidationError {
                field: fld.to_string(),
                message: "'Approved' condition may not be modified once set".to_string(),
            });
        }
    }

    // Cite: certificaterequest.go:191-196.
    if let Some(old_denied) = old_denied {
        if Some(old_denied) != get_cr_condition(new_cr_conds, CrConditionType::Denied) {
            el.push(ValidationError {
                field: fld.to_string(),
                message: "'Denied' condition may not be modified once set".to_string(),
            });
        }
    }

    // Cite: certificaterequest.go:198 — append base approval validation.
    el.extend(validate_cr_approval_condition(new_cr_conds, fld));
    el
}

/// Cite: cert-manager `getCertificateRequestCondition`
/// (certificaterequest.go:201-208) — returns the first condition of the given
/// type, if any.
fn get_cr_condition(
    conds: &[CrCondition],
    condition_type: CrConditionType,
) -> Option<&CrCondition> {
    conds.iter().find(|c| c.kind == condition_type)
}
