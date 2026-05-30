// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: condition transition semantics + observed-generation gating.
//!
//! Upstream: crossplane/crossplane-runtime v1.18.0
//!   apis/common/v1/condition.go::{Condition.Equal, Condition.WithObservedGeneration,
//!                                 ConditionedStatus.SetConditions, GetCondition}
//!
//! Closes the [[partial]] `condition-propagation-healthy`: the prior
//! `propagate_composed_to_xr` blew away all conditions with `Utc::now()` on
//! every reconcile (no LastTransitionTime preservation, no ObservedGeneration).
//! Upstream `SetConditions` is a no-op when the supplied condition is Equal
//! (ignoring time + observedGeneration) to the existing one — only bumping
//! observedGeneration — and replaces (resetting LastTransitionTime) only on a
//! real transition.

use cave_crossplane::conditions::{
    propagate_composed_to_xr, Condition, ConditionStatus, ConditionType, ConditionedStatus,
};
use chrono::{Duration, Utc};
use serde_json::json;

// ── Condition.Equal ignores LastTransitionTime AND ObservedGeneration ───────

#[test]
fn equal_ignores_transition_time_and_generation() {
    let a = Condition::new(ConditionType::Ready, ConditionStatus::True)
        .with_reason("Available")
        .with_observed_generation(3);
    let mut b = Condition::new(ConditionType::Ready, ConditionStatus::True).with_reason("Available");
    // Different transition time + generation, same type/status/reason/message.
    b.last_transition_time = a.last_transition_time - Duration::hours(5);
    b = b.with_observed_generation(99);
    assert!(a.equal(&b), "Equal must ignore LastTransitionTime + ObservedGeneration");
}

#[test]
fn equal_false_on_status_change() {
    let a = Condition::new(ConditionType::Ready, ConditionStatus::True);
    let b = Condition::new(ConditionType::Ready, ConditionStatus::False);
    assert!(!a.equal(&b));
}

#[test]
fn equal_false_on_reason_change() {
    let a = Condition::new(ConditionType::Ready, ConditionStatus::True).with_reason("Available");
    let b = Condition::new(ConditionType::Ready, ConditionStatus::True).with_reason("Creating");
    assert!(!a.equal(&b));
}

// ── ObservedGeneration serializes (omitempty) ───────────────────────────────

#[test]
fn observed_generation_in_json_when_set() {
    let c = Condition::new(ConditionType::Ready, ConditionStatus::True).with_observed_generation(7);
    let v = c.to_json();
    assert_eq!(v["observedGeneration"], json!(7));
}

#[test]
fn observed_generation_omitted_when_zero() {
    let c = Condition::new(ConditionType::Ready, ConditionStatus::True);
    let v = c.to_json();
    assert!(
        v.get("observedGeneration").is_none() || v["observedGeneration"].is_null(),
        "omitempty: observedGeneration must not appear when zero"
    );
}

// ── ConditionedStatus.SetConditions transition semantics ────────────────────

#[test]
fn set_conditions_appends_new_type() {
    let mut s = ConditionedStatus::default();
    s.set_conditions(&[Condition::new(ConditionType::Ready, ConditionStatus::True)]);
    assert_eq!(s.conditions.len(), 1);
    assert_eq!(s.get_condition(ConditionType::Ready).status, ConditionStatus::True);
}

#[test]
fn set_conditions_preserves_transition_time_when_unchanged() {
    let mut s = ConditionedStatus::default();
    let original = Condition::new(ConditionType::Ready, ConditionStatus::True).with_reason("Available");
    let t0 = original.last_transition_time;
    s.set_conditions(&[original]);

    // Re-apply the same condition with a LATER transition time — must be ignored.
    let again = Condition {
        last_transition_time: t0 + Duration::hours(1),
        ..Condition::new(ConditionType::Ready, ConditionStatus::True).with_reason("Available")
    };
    s.set_conditions(&[again]);

    assert_eq!(s.conditions.len(), 1, "no-op must not duplicate");
    assert_eq!(
        s.get_condition(ConditionType::Ready).last_transition_time,
        t0,
        "LastTransitionTime must be preserved across an unchanged reconcile"
    );
}

#[test]
fn set_conditions_bumps_observed_generation_when_unchanged() {
    let mut s = ConditionedStatus::default();
    s.set_conditions(&[
        Condition::new(ConditionType::Ready, ConditionStatus::True).with_observed_generation(1)
    ]);
    // Same status/reason but a newer generation — observedGeneration must advance,
    // LastTransitionTime must NOT.
    let t0 = s.get_condition(ConditionType::Ready).last_transition_time;
    s.set_conditions(&[
        Condition::new(ConditionType::Ready, ConditionStatus::True).with_observed_generation(5)
    ]);
    let c = s.get_condition(ConditionType::Ready);
    assert_eq!(c.observed_generation, 5, "observedGeneration must advance on unchanged condition");
    assert_eq!(c.last_transition_time, t0, "LastTransitionTime must stay put");
}

#[test]
fn set_conditions_does_not_lower_observed_generation() {
    let mut s = ConditionedStatus::default();
    s.set_conditions(&[
        Condition::new(ConditionType::Ready, ConditionStatus::True).with_observed_generation(10)
    ]);
    s.set_conditions(&[
        Condition::new(ConditionType::Ready, ConditionStatus::True).with_observed_generation(2)
    ]);
    assert_eq!(s.get_condition(ConditionType::Ready).observed_generation, 10);
}

#[test]
fn set_conditions_resets_transition_time_on_real_change() {
    let mut s = ConditionedStatus::default();
    let early = Condition {
        last_transition_time: Utc::now() - Duration::hours(2),
        ..Condition::new(ConditionType::Ready, ConditionStatus::False)
    };
    let t0 = early.last_transition_time;
    s.set_conditions(&[early]);

    // Status actually changes False → True: replace, new LastTransitionTime applies.
    let now_cond = Condition::new(ConditionType::Ready, ConditionStatus::True);
    s.set_conditions(&[now_cond]);

    assert_eq!(s.conditions.len(), 1);
    let c = s.get_condition(ConditionType::Ready);
    assert_eq!(c.status, ConditionStatus::True);
    assert!(c.last_transition_time > t0, "transition must refresh LastTransitionTime");
}

#[test]
fn get_condition_unknown_when_absent() {
    let s = ConditionedStatus::default();
    let c = s.get_condition(ConditionType::Synced);
    assert_eq!(c.status, ConditionStatus::Unknown);
    assert_eq!(c.condition_type, ConditionType::Synced);
}

// ── propagate_composed_to_xr now honours transition semantics ───────────────

#[test]
fn propagate_preserves_transition_time_across_stable_reconciles() {
    let composed = vec![json!({
        "status":{"conditions":[
            {"type":"Ready","status":"True"},
            {"type":"Synced","status":"True"},
            {"type":"Healthy","status":"True"}
        ]}
    })];

    // First reconcile establishes conditions.
    let xr1 = propagate_composed_to_xr(&json!({"metadata":{"generation":1}}), &composed);
    let ready1 = xr1["status"]["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "Ready")
        .unwrap()
        .clone();
    let lt1 = ready1["lastTransitionTime"].as_str().unwrap().to_string();

    // Second reconcile, same composed state, higher generation — Ready stays True,
    // so lastTransitionTime must be IDENTICAL, but observedGeneration advances.
    let xr2 = propagate_composed_to_xr(&xr1, &composed);
    // re-stamp generation 2 on the carried-over xr
    let mut xr2_in = xr2.clone();
    xr2_in["metadata"] = json!({"generation": 2});
    let xr3 = propagate_composed_to_xr(&xr2_in, &composed);
    let ready3 = xr3["status"]["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "Ready")
        .unwrap()
        .clone();

    assert_eq!(
        ready3["lastTransitionTime"].as_str().unwrap(),
        lt1,
        "stable Ready=True must keep its original lastTransitionTime"
    );
    assert_eq!(
        ready3["observedGeneration"],
        json!(2),
        "observedGeneration must reflect metadata.generation"
    );
}
