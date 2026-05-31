// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the local-autonomy connection state machine.
//!
//! Faithful port of KubeEdge edge autonomy:
//!   - the EdgeHub keepalive/heartbeat → connection state (Connected /
//!     Disconnected) with a heartbeat timeout;
//!   - the autonomy guarantee that a lost cloud link does NOT evict pods —
//!     the edge keeps running workloads offline (contrast with the default
//!     node-controller eviction);
//!   - reconcile-on-reconnect: a Disconnected→Connected transition flags that
//!     a full resync is required (the edge fell behind while offline);
//!   - offline-duration tracking.
//!
//! Pure state-machine logic — no network keepalive.

use cave_edge_runtime::autonomy::{ConnectionState, EdgeAutonomy};

#[test]
fn starts_connected() {
    let a = EdgeAutonomy::new(0);
    assert_eq!(a.state(), ConnectionState::Connected);
}

#[test]
fn stays_connected_within_heartbeat_timeout() {
    let mut a = EdgeAutonomy::new(0);
    // timeout = 30, last heartbeat at t=0, now t=20 → still connected.
    assert_eq!(a.tick(20, 30), ConnectionState::Connected);
}

#[test]
fn disconnects_after_heartbeat_timeout() {
    let mut a = EdgeAutonomy::new(0);
    assert_eq!(a.tick(31, 30), ConnectionState::Disconnected);
    assert_eq!(a.state(), ConnectionState::Disconnected);
}

#[test]
fn heartbeat_resets_timer_and_prevents_disconnect() {
    let mut a = EdgeAutonomy::new(0);
    a.heartbeat(25);
    // Without the heartbeat t=31 would disconnect; with it the window is fresh.
    assert_eq!(a.tick(31, 30), ConnectionState::Connected);
}

#[test]
fn offline_does_not_evict_pods() {
    let mut a = EdgeAutonomy::new(0);
    a.tick(31, 30); // now Disconnected
    assert_eq!(a.state(), ConnectionState::Disconnected);
    assert!(a.keep_pods_running(), "edge autonomy keeps pods running offline");
    assert!(!a.should_evict_on_disconnect());
}

#[test]
fn reconnect_flags_reconcile_needed_once() {
    let mut a = EdgeAutonomy::new(0);
    a.tick(31, 30); // Disconnected
    a.heartbeat(40); // reconnect
    assert_eq!(a.state(), ConnectionState::Connected);
    // The resync flag is set exactly once and consumed by the reader.
    assert!(a.take_reconcile_needed());
    assert!(!a.take_reconcile_needed());
}

#[test]
fn heartbeat_while_connected_does_not_flag_reconcile() {
    let mut a = EdgeAutonomy::new(0);
    a.heartbeat(10); // still connected, no prior disconnect
    assert!(!a.take_reconcile_needed());
}

#[test]
fn offline_duration_tracks_since_disconnect() {
    let mut a = EdgeAutonomy::new(0);
    assert_eq!(a.offline_duration(20), None); // still connected
    a.tick(31, 30); // Disconnected at t=31
    assert_eq!(a.offline_duration(50), Some(19)); // 50 - 31
    a.heartbeat(60); // reconnect
    assert_eq!(a.offline_duration(70), None);
}
