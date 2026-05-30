// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of the pure label-validation helpers in pkg/apis/v1/labels.go from
// kubernetes-sigs/karpenter v1.12.1 (sha ed490e8): the well-known
// label/capacity-type constants, GetLabelDomain, IsRestrictedLabel (restricted
// domain + ".domain" suffix + RestrictedLabels membership), NodeClassLabelKey,
// and HasKnownValues (well-known requirement value gating).

use cave_karpenter::labels::{
    get_label_domain, has_known_values, is_restricted_label, node_class_label_key,
    CAPACITY_TYPE_LABEL_KEY, CAPACITY_TYPE_ON_DEMAND, CAPACITY_TYPE_SPOT, NODEPOOL_LABEL_KEY,
};

// ---- constants ---------------------------------------------------------------

#[test]
fn karpenter_label_keys() {
    assert_eq!(NODEPOOL_LABEL_KEY, "karpenter.sh/nodepool");
    assert_eq!(CAPACITY_TYPE_LABEL_KEY, "karpenter.sh/capacity-type");
    assert_eq!(CAPACITY_TYPE_SPOT, "spot");
    assert_eq!(CAPACITY_TYPE_ON_DEMAND, "on-demand");
}

// ---- GetLabelDomain ----------------------------------------------------------

#[test]
fn label_domain_is_first_slash_segment() {
    assert_eq!(get_label_domain("karpenter.sh/nodepool"), "karpenter.sh");
    assert_eq!(get_label_domain("topology.kubernetes.io/zone"), "topology.kubernetes.io");
}

#[test]
fn label_domain_empty_when_no_slash() {
    assert_eq!(get_label_domain("hostname"), "");
}

// ---- IsRestrictedLabel -------------------------------------------------------

#[test]
fn well_known_label_is_allowed_even_in_karpenter_domain() {
    // WellKnownLabels short-circuits before the restricted-domain check
    assert!(is_restricted_label(NODEPOOL_LABEL_KEY).is_ok());
    assert!(is_restricted_label(CAPACITY_TYPE_LABEL_KEY).is_ok());
    assert!(is_restricted_label("topology.kubernetes.io/zone").is_ok());
}

#[test]
fn custom_label_in_restricted_domain_is_rejected() {
    assert!(is_restricted_label("karpenter.sh/custom").is_err());
}

#[test]
fn label_in_subdomain_of_restricted_domain_is_rejected() {
    // domain "foo.karpenter.sh" ends with ".karpenter.sh" → restricted
    assert!(is_restricted_label("foo.karpenter.sh/thing").is_err());
}

#[test]
fn hostname_restricted_label_is_rejected() {
    assert!(is_restricted_label("kubernetes.io/hostname").is_err());
}

#[test]
fn ordinary_custom_label_is_allowed() {
    assert!(is_restricted_label("example.com/team").is_ok());
    assert!(is_restricted_label("custom-label").is_ok());
}

// ---- NodeClassLabelKey -------------------------------------------------------

#[test]
fn node_class_label_key_lowercases_kind() {
    assert_eq!(
        node_class_label_key("karpenter.k8s.aws", "EC2NodeClass"),
        "karpenter.k8s.aws/ec2nodeclass"
    );
}

// ---- HasKnownValues ----------------------------------------------------------

#[test]
fn known_values_ok_for_non_well_known_key() {
    // key Karpenter does not gate → any values accepted
    assert!(has_known_values("example.com/anything", &["whatever".into()]).is_ok());
}

#[test]
fn known_values_ok_when_any_value_is_known() {
    assert!(has_known_values(CAPACITY_TYPE_LABEL_KEY, &[CAPACITY_TYPE_SPOT.into()]).is_ok());
    // HasAny semantics: a known value among unknowns still passes
    assert!(has_known_values(
        CAPACITY_TYPE_LABEL_KEY,
        &["bogus".into(), CAPACITY_TYPE_ON_DEMAND.into()]
    )
    .is_ok());
}

#[test]
fn known_values_err_when_all_values_unknown() {
    assert!(has_known_values(CAPACITY_TYPE_LABEL_KEY, &["bogus".into(), "nope".into()]).is_err());
}
