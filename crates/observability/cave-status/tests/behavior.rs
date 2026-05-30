// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for cave-status.
//!
//! These port upstream louislam/uptime-kuma v1.23.0 status-page behaviors that
//! the live crate already implements but does not yet cover: the worst-status
//! rollup precedence (status badge), per-status bucket counts, the
//! maintenance-as-issue banner condition, and resolved-incident serde (the
//! "past incidents" section). Each assertion checks a concrete value derived
//! from the `engine`/`models` implementation.

use cave_status::engine::{compute_overall_status, count_by_status, has_issues};
use cave_status::models::{ComponentStatus, StatusComponent, StatusIncident, StatusPage};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

fn make_component(name: &str, status: ComponentStatus) -> StatusComponent {
    StatusComponent {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: String::new(),
        status,
        group: None,
        updated_at: Utc::now(),
    }
}

fn make_page(components: Vec<StatusComponent>) -> StatusPage {
    let overall = compute_overall_status(&components);
    StatusPage {
        name: "Test Page".to_string(),
        components,
        overall_status: overall,
        last_updated: Utc::now(),
    }
}

/// Interior rank precedence: with Degraded(1), Partial(2) and Maintenance(3)
/// present but no MajorOutage, the rollup must resolve to the maintenance rank
/// (3), the highest present. The existing tests only cover all-operational,
/// single-major, and empty — never this interior ordering.
#[test]
fn test_overall_status_maintenance_outranks_partial_and_degraded() {
    let components = vec![
        make_component("API", ComponentStatus::DegradedPerformance),
        make_component("DB", ComponentStatus::PartialOutage),
        make_component("Cache", ComponentStatus::UnderMaintenance),
    ];
    assert_eq!(
        compute_overall_status(&components),
        ComponentStatus::UnderMaintenance
    );
}

/// Adding a MajorOutage(4) to the mixed set must override maintenance(3),
/// confirming MajorOutage holds the top rank.
#[test]
fn test_overall_status_major_overrides_maintenance() {
    let components = vec![
        make_component("API", ComponentStatus::DegradedPerformance),
        make_component("DB", ComponentStatus::PartialOutage),
        make_component("Cache", ComponentStatus::UnderMaintenance),
        make_component("Edge", ComponentStatus::MajorOutage),
    ];
    assert_eq!(
        compute_overall_status(&components),
        ComponentStatus::MajorOutage
    );
}

/// PartialOutage(2) outranks DegradedPerformance(1) when those are the only
/// two non-operational statuses present.
#[test]
fn test_overall_status_partial_outranks_degraded() {
    let components = vec![
        make_component("API", ComponentStatus::Operational),
        make_component("Cache", ComponentStatus::DegradedPerformance),
        make_component("DB", ComponentStatus::PartialOutage),
    ];
    assert_eq!(
        compute_overall_status(&components),
        ComponentStatus::PartialOutage
    );
}

/// count_by_status keys on the Debug form of each status. The existing test
/// covers only the Operational/Degraded/Major buckets; assert the
/// UnderMaintenance and PartialOutage buckets are tallied under their exact
/// debug-formatted keys.
#[test]
fn test_count_by_status_includes_maintenance_and_partial_buckets() {
    let components = vec![
        make_component("A", ComponentStatus::UnderMaintenance),
        make_component("B", ComponentStatus::UnderMaintenance),
        make_component("C", ComponentStatus::PartialOutage),
        make_component("D", ComponentStatus::Operational),
    ];
    let counts = count_by_status(&components);
    assert_eq!(counts.get("UnderMaintenance"), Some(&2));
    assert_eq!(counts.get("PartialOutage"), Some(&1));
    assert_eq!(counts.get("Operational"), Some(&1));
    // Only the three distinct statuses present produce keys.
    assert_eq!(counts.len(), 3);
}

/// An empty component slice produces an empty count map (no default keys).
#[test]
fn test_count_by_status_empty_is_empty_map() {
    let counts = count_by_status(&[]);
    assert!(counts.is_empty());
}

/// has_issues treats UnderMaintenance as a non-operational issue (banner
/// shown). Existing coverage only exercises DegradedPerformance/PartialOutage.
#[test]
fn test_has_issues_true_for_maintenance_only() {
    let page = make_page(vec![
        make_component("API", ComponentStatus::Operational),
        make_component("Maint", ComponentStatus::UnderMaintenance),
    ]);
    assert!(has_issues(&page));
}

/// has_issues returns false for an empty page (no components, no issues).
#[test]
fn test_has_issues_false_for_empty_page() {
    let page = make_page(vec![]);
    assert!(!has_issues(&page));
}

/// Resolved-incident serde: with `resolved_at = Some(...)` the value must
/// survive a JSON round-trip unchanged, and the field must serialize as an
/// RFC3339 timestamp rather than null. Existing model tests only cover the
/// `None` (active) branch.
#[test]
fn test_status_incident_resolved_roundtrip() {
    let resolved = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let incident = StatusIncident {
        id: Uuid::new_v4(),
        title: "Database connectivity issues".to_string(),
        affected_components: vec![Uuid::new_v4(), Uuid::new_v4()],
        impact: ComponentStatus::PartialOutage,
        created_at: Utc.with_ymd_and_hms(2026, 1, 2, 2, 0, 0).unwrap(),
        resolved_at: Some(resolved),
    };
    let json = serde_json::to_string(&incident).unwrap();
    // Resolved timestamp is present (not null) in the serialized form.
    assert!(json.contains("2026-01-02T03:04:05"));
    assert!(!json.contains("\"resolved_at\":null"));

    let decoded: StatusIncident = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, incident);
    assert_eq!(decoded.resolved_at, Some(resolved));
}

/// ComponentStatus serializes via `#[serde(rename_all = "snake_case")]`. The
/// existing model test only pins `major_outage`; pin the maintenance and
/// partial-outage wire forms and confirm they round-trip.
#[test]
fn test_component_status_snake_case_wire_forms() {
    let cases = [
        (ComponentStatus::UnderMaintenance, "\"under_maintenance\""),
        (ComponentStatus::PartialOutage, "\"partial_outage\""),
        (
            ComponentStatus::DegradedPerformance,
            "\"degraded_performance\"",
        ),
        (ComponentStatus::Operational, "\"operational\""),
    ];
    for (status, wire) in cases {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, wire);
        let decoded: ComponentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, status);
    }
}
