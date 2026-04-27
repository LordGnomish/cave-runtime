//! ReplicaSet / Deployment revision history — `pkg/controller/deployment/util/deployment_util.go`.
//!
//! Each ReplicaSet owned by a Deployment carries
//! `metadata.annotations["deployment.kubernetes.io/revision"] = "<n>"`.
//! On rollout, the Deployment controller:
//!
//! 1. Finds the active RS (matching pod-template-hash).
//! 2. Stamps the next revision (= max revision + 1).
//! 3. Trims the history of stale RSes to `revisionHistoryLimit`.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const ANNOTATION_REVISION: &str = "deployment.kubernetes.io/revision";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsSnapshot {
    pub name: String,
    pub revision: u64,
    pub pod_template_hash: String,
    pub current_replicas: u32,
}

/// Compute the next revision number — `max(existing) + 1`, or 1 when empty.
pub fn next_revision(existing: &[RsSnapshot]) -> u64 {
    existing.iter().map(|r| r.revision).max().unwrap_or(0) + 1
}

/// Identify which RS is the active one (matching the desired
/// `pod_template_hash`).
pub fn active_rs<'a>(
    rs_list: &'a [RsSnapshot],
    desired_hash: &str,
) -> Option<&'a RsSnapshot> {
    rs_list.iter().find(|r| r.pod_template_hash == desired_hash)
}

/// Identify "old" ReplicaSets — every RS that is *not* the active one and
/// has 0 replicas. These are eligible for pruning under
/// `revisionHistoryLimit`. Returns the names of RSes to delete (oldest
/// revision first), keeping at most `keep` history entries.
pub fn rs_to_prune(
    rs_list: &[RsSnapshot],
    desired_hash: &str,
    keep: u32,
) -> Vec<String> {
    let mut old: Vec<&RsSnapshot> = rs_list
        .iter()
        .filter(|r| r.pod_template_hash != desired_hash && r.current_replicas == 0)
        .collect();
    old.sort_by_key(|r| r.revision);
    let limit = keep as usize;
    if old.len() <= limit {
        return vec![];
    }
    let to_drop = old.len() - limit;
    old.iter()
        .take(to_drop)
        .map(|r| r.name.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RolloutAction {
    /// Active RS already at desired hash and revision present — nothing to do.
    NoOp,
    /// New revision needed — stamp annotation on the named RS.
    StampRevision { rs_name: String, revision: u64 },
    /// Active RS missing — create one (caller supplies hash).
    CreateActiveRs,
}

pub fn evaluate(
    rs_list: &[RsSnapshot],
    desired_hash: &str,
) -> RolloutAction {
    match active_rs(rs_list, desired_hash) {
        None => RolloutAction::CreateActiveRs,
        Some(active) => {
            let next = next_revision(rs_list);
            if active.revision == next - 1 || active.revision == next {
                // Active already has the highest revision — no-op.
                RolloutAction::NoOp
            } else {
                RolloutAction::StampRevision {
                    rs_name: active.name.clone(),
                    revision: next,
                }
            }
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/deployment/util/deployment_util.go",
    "RevisionAnnotation",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn rs(name: &str, rev: u64, hash: &str, replicas: u32) -> RsSnapshot {
        RsSnapshot {
            name: name.into(),
            revision: rev,
            pod_template_hash: hash.into(),
            current_replicas: replicas,
        }
    }

    #[test]
    fn next_revision_starts_at_one_with_empty_list() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "MaxRevision",
            "tenant-rs-rev-empty"
        );
        assert_eq!(next_revision(&[]), 1);
    }

    #[test]
    fn next_revision_picks_max_plus_one() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "MaxRevision",
            "tenant-rs-rev-max"
        );
        let r = vec![rs("a", 3, "h1", 0), rs("b", 1, "h2", 0), rs("c", 7, "h3", 4)];
        assert_eq!(next_revision(&r), 8);
    }

    #[test]
    fn active_rs_matches_by_pod_template_hash() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "GetNewReplicaSet",
            "tenant-rs-rev-active"
        );
        let r = vec![rs("a", 1, "old", 0), rs("b", 2, "new", 4)];
        assert_eq!(active_rs(&r, "new").unwrap().name, "b");
    }

    #[test]
    fn active_rs_returns_none_when_no_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "GetNewReplicaSet",
            "tenant-rs-rev-active-none"
        );
        let r = vec![rs("a", 1, "old", 0)];
        assert!(active_rs(&r, "new").is_none());
    }

    #[test]
    fn rs_to_prune_keeps_history_limit() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "cleanupDeployment",
            "tenant-rs-rev-prune"
        );
        let r = vec![
            rs("active", 5, "new", 4),
            rs("old1", 1, "h1", 0),
            rs("old2", 2, "h2", 0),
            rs("old3", 3, "h3", 0),
            rs("old4", 4, "h4", 0),
        ];
        // 4 olds, keep 2 → prune 2 oldest (old1, old2).
        let to_del = rs_to_prune(&r, "new", 2);
        assert_eq!(to_del.len(), 2);
        assert!(to_del.contains(&"old1".to_string()));
        assert!(to_del.contains(&"old2".to_string()));
    }

    #[test]
    fn rs_to_prune_keeps_active_no_matter_what() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "cleanupDeployment",
            "tenant-rs-rev-prune-active-safe"
        );
        let r = vec![rs("active", 1, "h1", 4)];
        // History limit 0, but active RS still keeps its replicas → not pruned.
        assert!(rs_to_prune(&r, "h1", 0).is_empty());
    }

    #[test]
    fn rs_to_prune_skips_rses_with_replicas() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "cleanupDeployment",
            "tenant-rs-rev-prune-skip-replicas"
        );
        let r = vec![
            rs("active", 5, "new", 4),
            rs("old-still-running", 1, "h1", 2),
        ];
        // Old RS still has replicas → not eligible.
        assert!(rs_to_prune(&r, "new", 0).is_empty());
    }

    #[test]
    fn evaluate_active_at_top_revision_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "syncRolloutStatus",
            "tenant-rs-rev-eval-noop"
        );
        let r = vec![rs("a", 5, "new", 4)];
        assert_eq!(evaluate(&r, "new"), RolloutAction::NoOp);
    }

    #[test]
    fn evaluate_active_below_top_revision_stamps_next() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "syncRolloutStatus",
            "tenant-rs-rev-eval-stamp"
        );
        let r = vec![
            rs("active", 1, "new", 4),
            rs("old", 5, "old", 0),
        ];
        match evaluate(&r, "new") {
            RolloutAction::StampRevision { rs_name, revision } => {
                assert_eq!(rs_name, "active");
                assert_eq!(revision, 6);
            }
            other => panic!("expected StampRevision, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_no_active_rs_emits_create() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "getAllReplicaSetsAndSyncRevision",
            "tenant-rs-rev-eval-create"
        );
        let r = vec![rs("old", 1, "old", 0)];
        assert_eq!(evaluate(&r, "new"), RolloutAction::CreateActiveRs);
    }

    #[test]
    fn rollout_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "RolloutAction",
            "tenant-rs-rev-action-serde"
        );
        for a in [
            RolloutAction::NoOp,
            RolloutAction::CreateActiveRs,
            RolloutAction::StampRevision { rs_name: "x".into(), revision: 3 },
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: RolloutAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn revision_annotation_constant_matches_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "RevisionAnnotation",
            "tenant-rs-rev-const"
        );
        assert_eq!(ANNOTATION_REVISION, "deployment.kubernetes.io/revision");
    }
}
