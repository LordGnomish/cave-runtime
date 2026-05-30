// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! NodePool runtime validation — port of
//! `pkg/apis/v1/nodepool_validation.go` from kubernetes-sigs/karpenter
//! v1.12.1 (sha ed490e8). Apache-2.0 upstream; see NOTICE.
//!
//! `RuntimeValidate` covers the parts of the NodePool CRD that CRD-level
//! schema validation cannot express: it fans out into label validation, taint
//! validation, requirement validation, and the reservation of the
//! `karpenter.sh/nodepool` label key (which Karpenter injects itself and must
//! not be user-set). The per-requirement and per-taint logic lives in
//! [`crate::validation`]; this module is the NodePool-template fan-out.

use std::collections::BTreeMap;

use crate::labels::{is_restricted_label, NODEPOOL_LABEL_KEY};
use crate::scheduling::requirement::NodeSelectorRequirement;
use crate::scheduling::taints::Taint;
use crate::validation::{
    is_qualified_name, is_valid_label_value, validate_requirements, validate_taints,
    ValidationError,
};

/// `(*NodeClaimTemplate).validateLabels`: each label key must be a qualified
/// name, must not be the reserved `karpenter.sh/nodepool` key, must not be a
/// restricted label, and each value must be a valid label value.
pub fn validate_labels(labels: &BTreeMap<String, String>) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    for (key, value) in labels {
        if key == NODEPOOL_LABEL_KEY {
            errs.append(format!("invalid key name {key:?} in labels, restricted"));
        }
        for msg in is_qualified_name(key) {
            errs.append(format!("invalid key name {key:?} in labels, {msg:?}"));
        }
        for msg in is_valid_label_value(value) {
            errs.append(format!("invalid value: {value} for label[{key}], {msg}"));
        }
        if let Err(e) = is_restricted_label(key) {
            errs.append(format!("invalid key name {key:?} in labels, {e}"));
        }
    }
    errs.into_result()
}

/// `(*NodeClaimTemplate).validateRequirementsNodePoolKeyDoesNotExist`: the
/// reserved `karpenter.sh/nodepool` key may not appear among requirements.
pub fn validate_requirements_node_pool_key_does_not_exist(
    requirements: &[NodeSelectorRequirement],
) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    for requirement in requirements {
        if requirement.key == NODEPOOL_LABEL_KEY {
            errs.append(format!(
                "invalid key: {:?} in requirements, restricted",
                requirement.key
            ));
        }
    }
    errs.into_result()
}

/// `(*NodePool).RuntimeValidate`: combines label, taint, requirement, and
/// nodepool-key validation, aggregating every failure (`multierr.Combine`).
/// Taints and startup taints share a duplicate-detection set, exactly as
/// [`validate_taints`] models.
pub fn runtime_validate(
    labels: &BTreeMap<String, String>,
    taints: &[Taint],
    startup_taints: &[Taint],
    requirements: &[NodeSelectorRequirement],
) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    if let Err(e) = validate_labels(labels) {
        errs.absorb(e);
    }
    if let Err(e) = validate_taints(taints, startup_taints) {
        errs.absorb(e);
    }
    if let Err(e) = validate_requirements(requirements) {
        errs.absorb(e);
    }
    if let Err(e) = validate_requirements_node_pool_key_does_not_exist(requirements) {
        errs.absorb(e);
    }
    errs.into_result()
}
