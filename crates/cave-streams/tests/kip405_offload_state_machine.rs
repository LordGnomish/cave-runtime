// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 storage/src/main/java/org/apache/kafka/storage/internals/log/RemoteLogManager.java
//
// KIP-405 — Segment offload state machine
//   active → readonly → uploaded → deletable

use cave_streams::tiered_storage::offload_state_machine::{
    OffloadEvent, OffloadState, SegmentOffloadFsm, SegmentOffloadTracker,
};

#[test]
fn fsm_initial_state_is_active() {
    let fsm = SegmentOffloadFsm::new(42);
    assert_eq!(fsm.state(), OffloadState::Active);
    assert_eq!(fsm.segment_id(), 42);
}

#[test]
fn active_rolls_to_readonly_on_seal() {
    let mut fsm = SegmentOffloadFsm::new(1);
    fsm.transition(OffloadEvent::Seal).unwrap();
    assert_eq!(fsm.state(), OffloadState::Readonly);
}

#[test]
fn readonly_advances_to_uploaded_on_upload_done() {
    let mut fsm = SegmentOffloadFsm::new(1);
    fsm.transition(OffloadEvent::Seal).unwrap();
    fsm.transition(OffloadEvent::UploadDone).unwrap();
    assert_eq!(fsm.state(), OffloadState::Uploaded);
}

#[test]
fn uploaded_becomes_deletable_after_retention_window() {
    let mut fsm = SegmentOffloadFsm::new(1);
    fsm.transition(OffloadEvent::Seal).unwrap();
    fsm.transition(OffloadEvent::UploadDone).unwrap();
    fsm.transition(OffloadEvent::LocalRetentionExpired).unwrap();
    assert_eq!(fsm.state(), OffloadState::Deletable);
}

#[test]
fn invalid_transition_returns_error() {
    let mut fsm = SegmentOffloadFsm::new(1);
    // Cannot go straight from Active to Uploaded.
    let r = fsm.transition(OffloadEvent::UploadDone);
    assert!(r.is_err());
}

#[test]
fn upload_failed_event_keeps_state_readonly() {
    let mut fsm = SegmentOffloadFsm::new(1);
    fsm.transition(OffloadEvent::Seal).unwrap();
    fsm.transition(OffloadEvent::UploadFailed("network".into())).unwrap();
    assert_eq!(fsm.state(), OffloadState::Readonly);
    assert!(fsm.last_error().is_some());
}

#[test]
fn delete_finalises_segment_into_terminal() {
    let mut fsm = SegmentOffloadFsm::new(1);
    fsm.transition(OffloadEvent::Seal).unwrap();
    fsm.transition(OffloadEvent::UploadDone).unwrap();
    fsm.transition(OffloadEvent::LocalRetentionExpired).unwrap();
    fsm.transition(OffloadEvent::LocalDeleted).unwrap();
    assert!(fsm.is_terminal());
}

#[test]
fn tracker_per_segment_isolation() {
    let mut tracker = SegmentOffloadTracker::new();
    tracker.start(10);
    tracker.start(20);
    assert_eq!(tracker.state(10), Some(OffloadState::Active));
    assert_eq!(tracker.state(20), Some(OffloadState::Active));
    tracker.seal(10).unwrap();
    assert_eq!(tracker.state(10), Some(OffloadState::Readonly));
    assert_eq!(tracker.state(20), Some(OffloadState::Active));
}

#[test]
fn tracker_count_by_state() {
    let mut tracker = SegmentOffloadTracker::new();
    tracker.start(1);
    tracker.start(2);
    tracker.start(3);
    tracker.seal(2).unwrap();
    tracker.seal(3).unwrap();
    tracker.upload_done(3).unwrap();
    assert_eq!(tracker.count(OffloadState::Active), 1);
    assert_eq!(tracker.count(OffloadState::Readonly), 1);
    assert_eq!(tracker.count(OffloadState::Uploaded), 1);
    assert_eq!(tracker.count(OffloadState::Deletable), 0);
}

#[test]
fn tracker_evict_terminal_drops_finished_segments() {
    let mut tracker = SegmentOffloadTracker::new();
    tracker.start(7);
    tracker.seal(7).unwrap();
    tracker.upload_done(7).unwrap();
    tracker.local_retention_expired(7).unwrap();
    tracker.local_deleted(7).unwrap();
    assert_eq!(tracker.count(OffloadState::Deletable), 0); // moved to terminal
    let dropped = tracker.evict_terminal();
    assert_eq!(dropped, 1);
    assert!(tracker.state(7).is_none());
}

#[test]
fn ready_for_upload_returns_only_readonly_segments() {
    let mut tracker = SegmentOffloadTracker::new();
    tracker.start(1);
    tracker.start(2);
    tracker.start(3);
    tracker.seal(2).unwrap();
    tracker.seal(3).unwrap();
    tracker.upload_done(3).unwrap();
    let ready: Vec<u64> = tracker.ready_for_upload().collect();
    assert_eq!(ready, vec![2]);
}
