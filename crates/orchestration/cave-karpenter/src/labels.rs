// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Well-known labels + label validation — port of the pure helpers in
//! `pkg/apis/v1/labels.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha ed490e8).
//!
//! Karpenter reserves the `karpenter.sh` label domain and a small set of
//! restricted labels so user-supplied requirements cannot interfere with its
//! provisioning logic. It also gates a handful of well-known requirement keys
//! (e.g. capacity-type) to a known value set.
//!
//! Ported: the well-known label/annotation/capacity constants,
//! [`get_label_domain`], [`is_restricted_label`], [`node_class_label_key`],
//! and [`has_known_values`].

use std::fmt;

// ── apis.Group / CompatibilityGroup ──────────────────────────────────────────
const GROUP: &str = "karpenter.sh";

// ── Well known capacity-type / architecture values ───────────────────────────
pub const ARCHITECTURE_AMD64: &str = "amd64";
pub const ARCHITECTURE_ARM64: &str = "arm64";
pub const CAPACITY_TYPE_SPOT: &str = "spot";
pub const CAPACITY_TYPE_ON_DEMAND: &str = "on-demand";
pub const CAPACITY_TYPE_RESERVED: &str = "reserved";

// ── Karpenter-specific label keys ────────────────────────────────────────────
pub const NODEPOOL_LABEL_KEY: &str = "karpenter.sh/nodepool";
pub const NODE_INITIALIZED_LABEL_KEY: &str = "karpenter.sh/initialized";
pub const NODE_REGISTERED_LABEL_KEY: &str = "karpenter.sh/registered";
pub const NODE_DO_NOT_SYNC_TAINTS_LABEL_KEY: &str = "karpenter.sh/do-not-sync-taints";
pub const CAPACITY_TYPE_LABEL_KEY: &str = "karpenter.sh/capacity-type";

// ── Karpenter-specific finalizer ─────────────────────────────────────────────
pub const TERMINATION_FINALIZER: &str = "karpenter.sh/termination";

// ── Upstream k8s well-known node labels ──────────────────────────────────────
const LABEL_TOPOLOGY_ZONE: &str = "topology.kubernetes.io/zone";
const LABEL_TOPOLOGY_REGION: &str = "topology.kubernetes.io/region";
const LABEL_INSTANCE_TYPE_STABLE: &str = "node.kubernetes.io/instance-type";
const LABEL_ARCH_STABLE: &str = "kubernetes.io/arch";
const LABEL_OS_STABLE: &str = "kubernetes.io/os";
const LABEL_WINDOWS_BUILD: &str = "node.kubernetes.io/windows-build";
const LABEL_HOSTNAME: &str = "kubernetes.io/hostname";

/// `WellKnownLabels` — labels Karpenter understands and may narrow further.
pub const WELL_KNOWN_LABELS: &[&str] = &[
    NODEPOOL_LABEL_KEY,
    LABEL_TOPOLOGY_ZONE,
    LABEL_TOPOLOGY_REGION,
    LABEL_INSTANCE_TYPE_STABLE,
    LABEL_ARCH_STABLE,
    LABEL_OS_STABLE,
    CAPACITY_TYPE_LABEL_KEY,
    LABEL_WINDOWS_BUILD,
];

/// `RestrictedLabelDomains` reserved by Karpenter.
const RESTRICTED_LABEL_DOMAINS: &[&str] = &[GROUP];

/// `RestrictedLabels` that should not be used directly.
const RESTRICTED_LABELS: &[&str] = &[LABEL_HOSTNAME];

/// Error raised by the label validators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelError {
    /// `IsRestrictedLabel` — label collides with Karpenter's reserved set.
    Restricted(String),
    /// `HasKnownValues` — a well-known key was given only unknown values.
    UnknownValues { key: String, values: Vec<String> },
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelError::Restricted(key) => write!(
                f,
                "using label {key} is not allowed as it might interfere with the internal \
                 provisioning logic; specify a well known label or a custom label that does \
                 not use a restricted domain"
            ),
            LabelError::UnknownValues { key, values } => {
                write!(f, "invalid values: {values:?} for key: {key}")
            }
        }
    }
}

impl std::error::Error for LabelError {}

/// `GetLabelDomain`: the segment before the first `/`, or `""` when the key has
/// no domain.
pub fn get_label_domain(key: &str) -> &str {
    match key.split_once('/') {
        Some((domain, _)) => domain,
        None => "",
    }
}

/// `IsRestrictedLabel`: `Ok` if the label may be used. A well-known label is
/// always allowed; otherwise the key is rejected when its domain equals (or is
/// a subdomain of) a restricted domain, or when it is a member of
/// `RestrictedLabels`.
pub fn is_restricted_label(key: &str) -> Result<(), LabelError> {
    if WELL_KNOWN_LABELS.contains(&key) {
        return Ok(());
    }
    let domain = get_label_domain(key);
    for restricted in RESTRICTED_LABEL_DOMAINS {
        if domain == *restricted || domain.ends_with(&format!(".{restricted}")) {
            return Err(LabelError::Restricted(key.to_string()));
        }
    }
    if RESTRICTED_LABELS.contains(&key) {
        return Err(LabelError::Restricted(key.to_string()));
    }
    Ok(())
}

/// `NodeClassLabelKey`: `"<group>/<lowercased kind>"`.
pub fn node_class_label_key(group: &str, kind: &str) -> String {
    format!("{group}/{}", kind.to_lowercase())
}

/// Well-known requirement keys whose values are gated to a known set
/// (`WellKnownValuesForRequirements`).
fn well_known_values_for(key: &str) -> Option<&'static [&'static str]> {
    if key == CAPACITY_TYPE_LABEL_KEY {
        Some(&[
            CAPACITY_TYPE_ON_DEMAND,
            CAPACITY_TYPE_SPOT,
            CAPACITY_TYPE_RESERVED,
        ])
    } else {
        None
    }
}

/// `HasKnownValues`: `Ok` for keys that are not well-known. For a well-known
/// key, at least one of `values` must appear in
/// `WellKnownValuesForRequirements[key]` (mirroring the `HasAny` semantics:
/// a key with no entry resolves to the nil set, so any values fail).
pub fn has_known_values(key: &str, values: &[String]) -> Result<(), LabelError> {
    if !WELL_KNOWN_LABELS.contains(&key) {
        return Ok(());
    }
    // nil set upstream → empty slice here; HasAny over an empty set is false.
    let known = well_known_values_for(key).unwrap_or(&[]);
    if values.iter().any(|v| known.contains(&v.as_str())) {
        Ok(())
    } else {
        Err(LabelError::UnknownValues {
            key: key.to_string(),
            values: values.to_vec(),
        })
    }
}
