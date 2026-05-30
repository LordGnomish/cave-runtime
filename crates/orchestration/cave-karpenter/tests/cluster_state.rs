// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of node nomination + deletion marking from
// pkg/controllers/state/cluster.go and the disruption candidate gate in
// pkg/controllers/disruption/candidate.go (kubernetes-sigs/karpenter v1.12.1,
// sha ed490e8).
//
// A node nominated for an incoming pod (NominateNodeForPod) is protected from
// disruption for a nomination window so the scheduler's just-made placement
// decision is not immediately undone. A node marked for deletion
// (MarkForDeletion) is likewise excluded from the disruption candidate set.

use cave_karpenter::cluster_state::{filter_disruptable, ClusterState};
use cave_karpenter::disruption::{Decision, DisruptionReason};
use std::time::{Duration, UNIX_EPOCH};

fn decision(node: &str) -> Decision {
    Decision {
        claim_name: node.into(),
        reason: DisruptionReason::Consolidation,
        message: "underutilized".into(),
    }
}

// ---- nomination window -------------------------------------------------------

#[test]
fn fresh_state_has_no_nominations_or_marks() {
    let s = ClusterState::new();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    assert!(!s.is_nominated("n1", now));
    assert!(!s.is_marked_for_deletion("n1"));
    assert!(s.is_disruption_candidate("n1", now));
}

#[test]
fn nomination_holds_until_expiry() {
    let mut s = ClusterState::new();
    let t0 = UNIX_EPOCH + Duration::from_secs(100);
    s.nominate("n1", t0 + Duration::from_secs(30));

    assert!(s.is_nominated("n1", t0 + Duration::from_secs(10)));
    assert!(s.is_nominated("n1", t0 + Duration::from_secs(29)));
    // At/after expiry the nomination lapses.
    assert!(!s.is_nominated("n1", t0 + Duration::from_secs(30)));
    assert!(!s.is_nominated("n1", t0 + Duration::from_secs(60)));
}

#[test]
fn re_nominating_extends_the_window() {
    let mut s = ClusterState::new();
    let t0 = UNIX_EPOCH + Duration::from_secs(100);
    s.nominate("n1", t0 + Duration::from_secs(10));
    s.nominate("n1", t0 + Duration::from_secs(50));
    assert!(s.is_nominated("n1", t0 + Duration::from_secs(40)));
}

// ---- deletion marking --------------------------------------------------------

#[test]
fn mark_and_unmark_for_deletion() {
    let mut s = ClusterState::new();
    assert!(!s.is_marked_for_deletion("n1"));
    s.mark_for_deletion("n1");
    assert!(s.is_marked_for_deletion("n1"));
    s.unmark_for_deletion("n1");
    assert!(!s.is_marked_for_deletion("n1"));
}

// ---- candidate gate ----------------------------------------------------------

#[test]
fn nominated_node_is_not_a_candidate() {
    let mut s = ClusterState::new();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    s.nominate("n1", now + Duration::from_secs(30));
    assert!(!s.is_disruption_candidate("n1", now));
}

#[test]
fn marked_node_is_not_a_candidate() {
    let mut s = ClusterState::new();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    s.mark_for_deletion("n1");
    assert!(!s.is_disruption_candidate("n1", now));
}

// ---- filter_disruptable integration ------------------------------------------

#[test]
fn filter_drops_nominated_and_marked_keeps_rest() {
    let mut s = ClusterState::new();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    s.nominate("nominated", now + Duration::from_secs(30));
    s.mark_for_deletion("deleting");

    let decisions = vec![
        decision("nominated"),
        decision("deleting"),
        decision("free"),
    ];
    let kept = filter_disruptable(decisions, &s, now);
    let names: Vec<&str> = kept.iter().map(|d| d.claim_name.as_str()).collect();
    assert_eq!(names, vec!["free"]);
}

#[test]
fn filter_keeps_node_whose_nomination_expired() {
    let mut s = ClusterState::new();
    let t0 = UNIX_EPOCH + Duration::from_secs(100);
    s.nominate("n1", t0 + Duration::from_secs(10));
    // Evaluate after the window has lapsed.
    let kept = filter_disruptable(vec![decision("n1")], &s, t0 + Duration::from_secs(20));
    assert_eq!(kept.len(), 1);
}
