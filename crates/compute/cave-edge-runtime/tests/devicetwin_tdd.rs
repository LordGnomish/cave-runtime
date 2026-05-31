// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the edge device twin.
//!
//! Faithful port of KubeEdge `edge/pkg/devicetwin`:
//!   - the Expected/Actual twin state model with a {cloud, edge} TwinVersion
//!     per attribute (`dtcommon` ExpectedVersion / ActualVersion);
//!   - `DealVersion` conflict resolution — an edge-local actual update bumps
//!     the edge version; a cloud expected update is accepted only when the
//!     edge has not moved ahead of the cloud's view (otherwise it is rejected
//!     so unsynced edge changes are not lost);
//!   - the twin delta (every attribute whose expected != actual), the payload
//!     the twin module publishes toward the device;
//!   - the membership module (bind / unbind devices to the edge node).
//!
//! Pure twin logic — no MQTT, no device protocol.

use cave_edge_runtime::devicetwin::{DeviceTwin, TwinVersion};

// ─── membership ─────────────────────────────────────────────────────────────

#[test]
fn bind_then_member_and_unbind() {
    let mut dt = DeviceTwin::new();
    dt.bind_device("sensor-b");
    dt.bind_device("sensor-a");
    assert!(dt.is_member("sensor-a"));
    assert_eq!(dt.members(), vec!["sensor-a".to_string(), "sensor-b".to_string()]);
    dt.unbind_device("sensor-a");
    assert!(!dt.is_member("sensor-a"));
    assert_eq!(dt.members(), vec!["sensor-b".to_string()]);
}

// ─── DealVersion conflict resolution ────────────────────────────────────────

#[test]
fn actual_update_bumps_edge_version() {
    let mut dt = DeviceTwin::new();
    dt.update_actual("dev1", "temp", "20");
    assert_eq!(dt.version("dev1", "temp"), Some(TwinVersion { cloud: 0, edge: 1 }));
    dt.update_actual("dev1", "temp", "21");
    assert_eq!(dt.version("dev1", "temp"), Some(TwinVersion { cloud: 0, edge: 2 }));
}

#[test]
fn cloud_expected_update_accepted_when_edge_not_ahead() {
    let mut dt = DeviceTwin::new();
    // Cloud pushes desired temp=25, declaring it has seen edge version 0.
    let ok = dt.update_expected("dev1", "temp", "25", TwinVersion { cloud: 1, edge: 0 });
    assert!(ok);
    assert_eq!(dt.expected("dev1", "temp"), Some("25".to_string()));
    assert_eq!(dt.version("dev1", "temp"), Some(TwinVersion { cloud: 1, edge: 0 }));
}

#[test]
fn cloud_expected_update_rejected_when_edge_moved_ahead() {
    let mut dt = DeviceTwin::new();
    // Edge made two local actual updates the cloud has not yet seen.
    dt.update_actual("dev1", "temp", "20");
    dt.update_actual("dev1", "temp", "21"); // edge version now 2
    // Cloud's update only acknowledges edge version 1 → must be rejected so
    // the unsynced edge change is not clobbered.
    let ok = dt.update_expected("dev1", "temp", "25", TwinVersion { cloud: 1, edge: 1 });
    assert!(!ok);
    assert_eq!(dt.expected("dev1", "temp"), None);
    assert_eq!(dt.version("dev1", "temp"), Some(TwinVersion { cloud: 0, edge: 2 }));
}

// ─── twin delta ─────────────────────────────────────────────────────────────

#[test]
fn delta_lists_attributes_where_expected_differs_from_actual() {
    let mut dt = DeviceTwin::new();
    dt.update_actual("dev1", "temp", "20");
    dt.update_expected("dev1", "temp", "25", TwinVersion { cloud: 1, edge: 1 });
    dt.update_actual("dev1", "led", "on");
    dt.update_expected("dev1", "led", "on", TwinVersion { cloud: 1, edge: 1 });
    let delta = dt.delta("dev1");
    // Only `temp` differs (expected 25 != actual 20); `led` matches.
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0].attr, "temp");
    assert_eq!(delta[0].expected, "25");
}

#[test]
fn delta_empty_when_actual_matches_expected() {
    let mut dt = DeviceTwin::new();
    dt.update_actual("dev1", "temp", "25");
    dt.update_expected("dev1", "temp", "25", TwinVersion { cloud: 1, edge: 1 });
    assert!(dt.delta("dev1").is_empty());
}

#[test]
fn delta_omits_attribute_with_no_expected_value() {
    let mut dt = DeviceTwin::new();
    // Edge reports an actual the cloud has never set an expected for.
    dt.update_actual("dev1", "humidity", "40");
    assert!(dt.delta("dev1").is_empty());
}

#[test]
fn delta_clears_once_actual_catches_up() {
    let mut dt = DeviceTwin::new();
    dt.update_actual("dev1", "temp", "20");
    dt.update_expected("dev1", "temp", "25", TwinVersion { cloud: 1, edge: 1 });
    assert_eq!(dt.delta("dev1").len(), 1);
    // Device applied the desired value and reported it back.
    dt.update_actual("dev1", "temp", "25");
    assert!(dt.delta("dev1").is_empty());
}
