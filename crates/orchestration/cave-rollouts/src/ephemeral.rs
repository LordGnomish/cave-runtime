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
