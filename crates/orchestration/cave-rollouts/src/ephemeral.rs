// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ephemeral pod-metadata reconcile — Argo Rollouts parity.
//!
//! Pure-function port of `rollout/ephemeralmetadata.go` +
//! `utils/replicaset.SyncEphemeralPodMetadata` (argoproj/argo-rollouts v1.9.0):
//! inject canary/stable ephemeral labels & annotations into a pod's ObjectMeta,
//! and strip keys that were previously injected but are no longer desired. The
//! set of injected keys is recorded under the
//! `rollout.argoproj.io/ephemeral-metadata` annotation so a later reconcile can
//! compute removals. The live pod patching belongs to cave-controller-manager.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Annotation under which the currently-injected ephemeral metadata is recorded.
pub const EPHEMERAL_METADATA_ANNOTATION: &str = "rollout.argoproj.io/ephemeral-metadata";

/// The ephemeral labels & annotations a strategy wants applied to a ReplicaSet's
/// pods (Argo's `PodTemplateMetadata`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodMetadata {
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl PodMetadata {
    fn is_empty(&self) -> bool {
        self.labels.is_empty() && self.annotations.is_empty()
    }
}

/// The subset of Kubernetes `ObjectMeta` this reconcile touches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectMeta {
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
}

/// Read back the ephemeral metadata we previously injected, parsed from the
/// `EPHEMERAL_METADATA_ANNOTATION` annotation. `None` if absent/unparseable.
pub fn parse_existing_pod_metadata(meta: &ObjectMeta) -> Option<PodMetadata> {
    let raw = meta.annotations.get(EPHEMERAL_METADATA_ANNOTATION)?;
    serde_json::from_str(raw).ok()
}

/// `SyncEphemeralPodMetadata` — reconcile `meta` toward `target`.
///
/// Previously-injected keys (recorded in the annotation) that are not in
/// `target` are stripped; `target`'s keys are then applied and re-recorded.
/// A `None`/empty target removes all injected metadata. Returns the updated
/// `ObjectMeta` and whether anything changed.
pub fn sync_ephemeral_pod_metadata(
    meta: &ObjectMeta,
    target: Option<&PodMetadata>,
) -> (ObjectMeta, bool) {
    let mut out = meta.clone();

    // 1. strip everything we injected last time.
    if let Some(existing) = parse_existing_pod_metadata(meta) {
        for k in existing.labels.keys() {
            out.labels.remove(k);
        }
        for k in existing.annotations.keys() {
            out.annotations.remove(k);
        }
    }
    out.annotations.remove(EPHEMERAL_METADATA_ANNOTATION);

    // 2. apply the desired target (if any) and re-record it.
    if let Some(t) = target {
        if !t.is_empty() {
            for (k, v) in &t.labels {
                out.labels.insert(k.clone(), v.clone());
            }
            for (k, v) in &t.annotations {
                out.annotations.insert(k.clone(), v.clone());
            }
            if let Ok(j) = serde_json::to_string(t) {
                out.annotations
                    .insert(EPHEMERAL_METADATA_ANNOTATION.to_string(), j);
            }
        }
    }

    let modified = out != *meta;
    (out, modified)
}

/// `reconcileEphemeralMetadata` target selection.
///
/// When the rollout is fully rolled out the new (only) ReplicaSet takes the
/// *stable* metadata and there is no separate stable RS; otherwise the new RS
/// takes `new_meta` and the stable RS takes `stable_meta`. Returns
/// `(new_rs_target, stable_rs_target)`.
pub fn reconcile_ephemeral_targets(
    fully_rolled_out: bool,
    new_meta: Option<PodMetadata>,
    stable_meta: Option<PodMetadata>,
) -> (Option<PodMetadata>, Option<PodMetadata>) {
    if fully_rolled_out {
        (stable_meta, None)
    } else {
        (new_meta, stable_meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pm(labels: &[(&str, &str)]) -> PodMetadata {
        let mut m = PodMetadata::default();
        for (k, v) in labels {
            m.labels.insert(k.to_string(), v.to_string());
        }
        m
    }

    #[test]
    fn apply_target_injects_labels_and_records_annotation() {
        let meta = ObjectMeta::default();
        let target = pm(&[("role", "canary")]);
        let (out, modified) = sync_ephemeral_pod_metadata(&meta, Some(&target));
        assert!(modified);
        assert_eq!(out.labels.get("role").map(String::as_str), Some("canary"));
        assert!(out.annotations.contains_key(EPHEMERAL_METADATA_ANNOTATION));
    }

    #[test]
    fn parse_round_trips_injected_metadata() {
        let meta = ObjectMeta::default();
        let target = pm(&[("role", "canary"), ("track", "blue")]);
        let (out, _) = sync_ephemeral_pod_metadata(&meta, Some(&target));
        let parsed = parse_existing_pod_metadata(&out).unwrap();
        assert_eq!(parsed.labels.get("role").map(String::as_str), Some("canary"));
        assert_eq!(parsed.labels.get("track").map(String::as_str), Some("blue"));
    }

    #[test]
    fn removing_target_strips_previously_injected_keys() {
        let meta = ObjectMeta::default();
        let (injected, _) = sync_ephemeral_pod_metadata(&meta, Some(&pm(&[("role", "canary")])));
        let (out, modified) = sync_ephemeral_pod_metadata(&injected, None);
        assert!(modified);
        assert!(!out.labels.contains_key("role"));
        assert!(!out.annotations.contains_key(EPHEMERAL_METADATA_ANNOTATION));
    }

    #[test]
    fn reapplying_same_metadata_is_noop() {
        let meta = ObjectMeta::default();
        let target = pm(&[("role", "canary")]);
        let (injected, _) = sync_ephemeral_pod_metadata(&meta, Some(&target));
        let (out, modified) = sync_ephemeral_pod_metadata(&injected, Some(&target));
        assert!(!modified);
        assert_eq!(out, injected);
    }

    #[test]
    fn reconcile_targets_fully_rolled_out_uses_stable_for_new_rs() {
        let stable = pm(&[("track", "stable")]);
        let new = pm(&[("track", "canary")]);
        let (new_rs, stable_rs) =
            reconcile_ephemeral_targets(true, Some(new), Some(stable.clone()));
        assert_eq!(new_rs, Some(stable));
        assert_eq!(stable_rs, None);
    }

    #[test]
    fn reconcile_targets_mid_rollout_splits_new_and_stable() {
        let stable = pm(&[("track", "stable")]);
        let new = pm(&[("track", "canary")]);
        let (new_rs, stable_rs) =
            reconcile_ephemeral_targets(false, Some(new.clone()), Some(stable.clone()));
        assert_eq!(new_rs, Some(new));
        assert_eq!(stable_rs, Some(stable));
    }
}
