// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD RED test for the alert grouping / deduplication engine.
//!
//! Line-ports the incident-side grouping algorithm from grafana/oncall v1.10.0:
//!   - engine/apps/alerts/models/alert.py :: Alert.render_group_data / insert_random_uuid
//!   - engine/apps/alerts/models/alert_group.py :: AlertGroupQuerySet.get_or_create_grouping
//!
//! The grouping engine computes a `group_distinction` (md5 of the grouping_id, or a
//! random uuid when no grouping_id is provided so the alert never groups), then
//! gets-or-creates the open incident group for (channel, distinction). A resolve
//! signal can re-attach to the latest resolved group when source-based resolving
//! is allowed.

use cave_incidents::grouping::{GroupingEngine, IncomingAlert};

#[test]
fn test_distinction_is_md5_of_grouping_id() {
    // render_group_data: group_distinction = md5(grouping_id).hexdigest()
    let d = GroupingEngine::group_distinction(Some("cpu-high:host-a"), false);
    // md5("cpu-high:host-a") is a 32-char lowercase hex string and deterministic
    assert_eq!(d.len(), 32);
    assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
    let d2 = GroupingEngine::group_distinction(Some("cpu-high:host-a"), false);
    assert_eq!(d, d2, "same grouping_id must yield identical distinction");
}

#[test]
fn test_no_grouping_id_never_groups() {
    // Insert random uuid when grouping_id is None -> two calls differ
    let a = GroupingEngine::group_distinction(None, false);
    let b = GroupingEngine::group_distinction(None, false);
    assert_ne!(a, b, "None grouping_id must produce unique distinctions");
}

#[test]
fn test_demo_alert_never_groups() {
    // is_demo forces a random uuid even with a grouping_id
    let a = GroupingEngine::group_distinction(Some("same"), true);
    let b = GroupingEngine::group_distinction(Some("same"), true);
    assert_ne!(a, b, "demo alerts must never group");
}

#[test]
fn test_two_alerts_same_distinction_group_together() {
    let mut eng = GroupingEngine::new(true); // allow_source_based_resolving
    let r1 = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("disk-full:/dev/sda".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    let r2 = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("disk-full:/dev/sda".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    assert!(r1.group_created, "first alert opens a new group");
    assert!(!r2.group_created, "second alert joins the existing open group");
    assert_eq!(r1.group_id, r2.group_id, "same distinction -> same group");
    assert_eq!(eng.open_group_count(), 1);
}

#[test]
fn test_different_distinction_make_different_groups() {
    let mut eng = GroupingEngine::new(true);
    let a = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("alert-a".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    let b = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("alert-b".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    assert_ne!(a.group_id, b.group_id);
    assert_eq!(eng.open_group_count(), 2);
}

#[test]
fn test_same_distinction_different_channel_do_not_group() {
    let mut eng = GroupingEngine::new(true);
    let a = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("shared".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    let b = eng.ingest(IncomingAlert {
        channel: "ch2".into(),
        grouping_id: Some("shared".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    assert_ne!(a.group_id, b.group_id, "channel is part of the grouping key");
}

#[test]
fn test_resolve_signal_reattaches_to_latest_resolved_group() {
    // get_or_create_grouping: when channel allows source-based resolving and the
    // incoming alert is a resolve signal, re-attach to the latest resolved group
    // instead of opening a new one.
    let mut eng = GroupingEngine::new(true);
    let open = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("svc-down".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    // Source resolves the group (closes it for grouping).
    eng.resolve_group_by_source(open.group_id);
    // A new resolve-signal alert with the same distinction must re-attach to the
    // resolved group, NOT open a brand new one.
    let resolve = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("svc-down".into()),
        is_resolve_signal: true,
        is_demo: false,
    });
    assert!(!resolve.group_created);
    assert_eq!(resolve.group_id, open.group_id);
}

#[test]
fn test_non_resolve_after_resolution_opens_new_group() {
    // After the group is resolved, a *firing* (non-resolve) alert with the same
    // distinction must open a brand new open group (the resolved one is closed
    // for grouping).
    let mut eng = GroupingEngine::new(true);
    let first = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("flap".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    eng.resolve_group_by_source(first.group_id);
    let second = eng.ingest(IncomingAlert {
        channel: "ch1".into(),
        grouping_id: Some("flap".into()),
        is_resolve_signal: false,
        is_demo: false,
    });
    assert!(second.group_created);
    assert_ne!(second.group_id, first.group_id);
}
