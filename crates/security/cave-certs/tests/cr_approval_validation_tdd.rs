// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RED→GREEN TDD (2026-05-30): cert-manager CertificateRequest approval-condition
//! admission validation.
//!
//! Line-port of cert-manager v1.17.2
//! `internal/apis/certmanager/validation/certificaterequest.go`:
//!   - `ValidateCertificateRequestApprovalCondition`
//!   - `ValidateUpdateCertificateRequestApprovalCondition`
//!
//! These are the PURE admission rules guaranteeing approval-condition
//! integrity: at most one Approved/Denied condition, status must be True,
//! Approved+Denied cannot coexist, and once set they are immutable. The
//! webhook transport that decodes AdmissionReview is cross-crate
//! (cave-admission); these rules are in-crate runtime logic.

use cave_certs::cr_approval_validation::{
    validate_cr_approval_condition, validate_update_cr_approval_condition, CrCondition,
    CrConditionStatus, CrConditionType,
};

fn approved_true() -> CrCondition {
    CrCondition {
        kind: CrConditionType::Approved,
        status: CrConditionStatus::True,
        reason: "ApprovedByPolicy".into(),
        message: "ok".into(),
    }
}

fn denied_true() -> CrCondition {
    CrCondition {
        kind: CrConditionType::Denied,
        status: CrConditionStatus::True,
        reason: "DeniedByPolicy".into(),
        message: "no".into(),
    }
}

#[test]
fn no_approval_conditions_is_valid() {
    let errs = validate_cr_approval_condition(&[], "status.conditions");
    assert!(errs.is_empty());
}

#[test]
fn single_approved_true_is_valid() {
    let errs = validate_cr_approval_condition(&[approved_true()], "status.conditions");
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn multiple_approved_conditions_forbidden() {
    // Cite: certificaterequest.go:153-155.
    let errs =
        validate_cr_approval_condition(&[approved_true(), approved_true()], "status.conditions");
    assert!(errs
        .iter()
        .any(|e| e.message.contains("multiple") && e.message.contains("Approved")));
}

#[test]
fn approved_condition_false_status_rejected() {
    // Cite: certificaterequest.go:158-163 — may only be set to True.
    let mut c = approved_true();
    c.status = CrConditionStatus::False;
    let errs = validate_cr_approval_condition(&[c], "status.conditions");
    assert!(errs.iter().any(|e| e.message.contains("only be set to True")));
}

#[test]
fn denied_condition_unknown_status_rejected() {
    let mut c = denied_true();
    c.status = CrConditionStatus::Unknown;
    let errs = validate_cr_approval_condition(&[c], "status.conditions");
    assert!(errs.iter().any(|e| e.message.contains("only be set to True")));
}

#[test]
fn approved_and_denied_cannot_coexist() {
    // Cite: certificaterequest.go:166-168.
    let errs = validate_cr_approval_condition(
        &[approved_true(), denied_true()],
        "status.conditions",
    );
    assert!(errs
        .iter()
        .any(|e| e.message.contains("cannot coexist")));
}

#[test]
fn update_approved_condition_cannot_be_modified_once_set() {
    // Cite: certificaterequest.go:184-189.
    let old = vec![approved_true()];
    let mut modified = approved_true();
    modified.reason = "ChangedReason".into();
    let new = vec![modified];
    let errs = validate_update_cr_approval_condition(&old, &new, "status.conditions");
    assert!(errs
        .iter()
        .any(|e| e.message.contains("'Approved' condition may not be modified")));
}

#[test]
fn update_denied_condition_cannot_be_modified_once_set() {
    // Cite: certificaterequest.go:191-196.
    let old = vec![denied_true()];
    let mut modified = denied_true();
    modified.message = "different".into();
    let new = vec![modified];
    let errs = validate_update_cr_approval_condition(&old, &new, "status.conditions");
    assert!(errs
        .iter()
        .any(|e| e.message.contains("'Denied' condition may not be modified")));
}

#[test]
fn update_unchanged_approved_condition_is_valid() {
    let old = vec![approved_true()];
    let new = vec![approved_true()];
    let errs = validate_update_cr_approval_condition(&old, &new, "status.conditions");
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn update_setting_approval_from_empty_is_valid() {
    // Going from no conditions to one Approved=True is allowed.
    let old: Vec<CrCondition> = vec![];
    let new = vec![approved_true()];
    let errs = validate_update_cr_approval_condition(&old, &new, "status.conditions");
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn update_also_runs_base_validation() {
    // Cite: certificaterequest.go:198 — appends base approval validation.
    let old: Vec<CrCondition> = vec![];
    let new = vec![approved_true(), denied_true()];
    let errs = validate_update_cr_approval_condition(&old, &new, "status.conditions");
    assert!(errs.iter().any(|e| e.message.contains("cannot coexist")));
}
