// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Chaos Mesh reconcile state machine (controllers/common).
//! desiredPhase (Run/Stop), per-target Records (Injected/NotInjected),
//! status Conditions (Selected/AllInjected/AllRecovered/Paused), the
//! Injecting→Running→Recovering→Finished phase machine, and the
//! duration-based recovery enforcer.

use cave_chaos::reconcile::{
    compute_conditions, compute_phase, desired_phase, is_terminal, ChaosPhase, DesiredPhase,
    RecordPhase, TargetRecord,
};

fn records(phases: &[RecordPhase]) -> Vec<TargetRecord> {
    phases
        .iter()
        .enumerate()
        .map(|(i, p)| TargetRecord {
            id: format!("pod-{i}"),
            phase: p.clone(),
        })
        .collect()
}

// ── desired_phase: duration-based recovery enforcer ─────────────────────────

#[test]
fn test_desired_phase_run_before_duration_elapses() {
    // 30s elapsed of a 60s experiment, not manually stopped -> keep running.
    assert_eq!(desired_phase(30, 60, false), DesiredPhase::Run);
}

#[test]
fn test_desired_phase_stop_after_duration_elapses() {
    assert_eq!(desired_phase(60, 60, false), DesiredPhase::Stop);
    assert_eq!(desired_phase(75, 60, false), DesiredPhase::Stop);
}

#[test]
fn test_desired_phase_manual_stop_overrides() {
    assert_eq!(desired_phase(5, 60, true), DesiredPhase::Stop);
}

#[test]
fn test_desired_phase_zero_duration_runs_indefinitely() {
    // duration_secs == 0 means no time bound; only manual stop ends it.
    assert_eq!(desired_phase(100000, 0, false), DesiredPhase::Run);
    assert_eq!(desired_phase(100000, 0, true), DesiredPhase::Stop);
}

// ── compute_conditions ──────────────────────────────────────────────────────

#[test]
fn test_conditions_empty_records_not_selected() {
    let c = compute_conditions(&[], DesiredPhase::Run, false);
    assert!(!c.selected);
    assert!(!c.all_injected);
    assert!(c.all_recovered); // vacuously recovered
}

#[test]
fn test_conditions_all_injected() {
    let r = records(&[RecordPhase::Injected, RecordPhase::Injected]);
    let c = compute_conditions(&r, DesiredPhase::Run, false);
    assert!(c.selected);
    assert!(c.all_injected);
    assert!(!c.all_recovered);
}

#[test]
fn test_conditions_partial_injection() {
    let r = records(&[RecordPhase::Injected, RecordPhase::NotInjected]);
    let c = compute_conditions(&r, DesiredPhase::Run, false);
    assert!(c.selected);
    assert!(!c.all_injected);
    assert!(!c.all_recovered);
}

#[test]
fn test_conditions_all_recovered() {
    let r = records(&[RecordPhase::NotInjected, RecordPhase::NotInjected]);
    let c = compute_conditions(&r, DesiredPhase::Stop, false);
    assert!(c.selected);
    assert!(!c.all_injected);
    assert!(c.all_recovered);
}

#[test]
fn test_conditions_paused_flag_propagates() {
    let r = records(&[RecordPhase::Injected]);
    let c = compute_conditions(&r, DesiredPhase::Run, true);
    assert!(c.paused);
}

// ── compute_phase: the state machine ────────────────────────────────────────

#[test]
fn test_phase_initial_when_no_records() {
    assert_eq!(
        compute_phase(DesiredPhase::Run, &[], false),
        ChaosPhase::Initial
    );
}

#[test]
fn test_phase_injecting_when_run_but_not_all_injected() {
    let r = records(&[RecordPhase::Injected, RecordPhase::NotInjected]);
    assert_eq!(
        compute_phase(DesiredPhase::Run, &r, false),
        ChaosPhase::Injecting
    );
}

#[test]
fn test_phase_running_when_run_and_all_injected() {
    let r = records(&[RecordPhase::Injected, RecordPhase::Injected]);
    assert_eq!(
        compute_phase(DesiredPhase::Run, &r, false),
        ChaosPhase::Running
    );
}

#[test]
fn test_phase_recovering_when_stop_but_some_still_injected() {
    let r = records(&[RecordPhase::Injected, RecordPhase::NotInjected]);
    assert_eq!(
        compute_phase(DesiredPhase::Stop, &r, false),
        ChaosPhase::Recovering
    );
}

#[test]
fn test_phase_finished_when_stop_and_all_recovered() {
    let r = records(&[RecordPhase::NotInjected, RecordPhase::NotInjected]);
    assert_eq!(
        compute_phase(DesiredPhase::Stop, &r, false),
        ChaosPhase::Finished
    );
}

#[test]
fn test_phase_paused_overrides_when_paused() {
    let r = records(&[RecordPhase::Injected, RecordPhase::Injected]);
    assert_eq!(
        compute_phase(DesiredPhase::Run, &r, true),
        ChaosPhase::Paused
    );
}

#[test]
fn test_is_terminal_only_finished() {
    assert!(is_terminal(&ChaosPhase::Finished));
    assert!(!is_terminal(&ChaosPhase::Running));
    assert!(!is_terminal(&ChaosPhase::Recovering));
    assert!(!is_terminal(&ChaosPhase::Injecting));
    assert!(!is_terminal(&ChaosPhase::Initial));
    assert!(!is_terminal(&ChaosPhase::Paused));
}
