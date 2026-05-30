// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Chaos reconcile state machine — Chaos Mesh `controllers/common` port.
//!
//! Pure-logic core of the controller: given the desired phase (Run/Stop) and the
//! per-target injection records, compute the status conditions and the overall
//! experiment phase, and decide (from elapsed time) when an experiment should
//! flip to recovery. The Kubernetes informer/watch loop, leader election and
//! RBAC that *drive* this logic live in cave-controller-manager and remain an
//! honest scope-cut; this module is the portable decision core.

use serde::{Deserialize, Serialize};

/// The phase the controller wants the experiment to be in.
/// Maps to Chaos Mesh `spec.*.desiredPhase` (Run/Stop).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesiredPhase {
    Run,
    Stop,
}

/// Injection state of a single target.
/// Maps to Chaos Mesh `status.experiment.records[].phase`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordPhase {
    NotInjected,
    Injected,
}

/// A per-target record tracking injection progress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetRecord {
    pub id: String,
    pub phase: RecordPhase,
}

/// Computed status conditions.
/// Maps to Chaos Mesh `status.conditions` (Selected/AllInjected/AllRecovered/Paused).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conditions {
    pub selected: bool,
    pub all_injected: bool,
    pub all_recovered: bool,
    pub paused: bool,
}

/// The overall experiment phase derived from desired phase + records.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChaosPhase {
    /// No targets selected yet.
    Initial,
    /// Run desired, some targets still NotInjected.
    Injecting,
    /// Run desired, all targets Injected.
    Running,
    /// Stop desired, some targets still Injected.
    Recovering,
    /// Stop desired, all targets recovered. Terminal.
    Finished,
    /// Experiment paused regardless of records.
    Paused,
}

/// Recovery enforcer: decide the desired phase from elapsed time.
///
/// * `manually_stopped` → always `Stop`.
/// * `duration_secs == 0` → no time bound, `Run` until manually stopped.
/// * otherwise → `Stop` once `elapsed_secs >= duration_secs`, else `Run`.
pub fn desired_phase(elapsed_secs: i64, duration_secs: u32, manually_stopped: bool) -> DesiredPhase {
    if manually_stopped {
        return DesiredPhase::Stop;
    }
    if duration_secs == 0 {
        return DesiredPhase::Run;
    }
    if elapsed_secs >= duration_secs as i64 {
        DesiredPhase::Stop
    } else {
        DesiredPhase::Run
    }
}

/// Compute status conditions from the target records.
pub fn compute_conditions(
    records: &[TargetRecord],
    _desired: DesiredPhase,
    paused: bool,
) -> Conditions {
    let selected = !records.is_empty();
    let all_injected = selected && records.iter().all(|r| r.phase == RecordPhase::Injected);
    let all_recovered = records.iter().all(|r| r.phase == RecordPhase::NotInjected);
    Conditions {
        selected,
        all_injected,
        all_recovered,
        paused,
    }
}

/// Compute the overall phase from the desired phase + records.
pub fn compute_phase(desired: DesiredPhase, records: &[TargetRecord], paused: bool) -> ChaosPhase {
    if paused {
        return ChaosPhase::Paused;
    }
    if records.is_empty() {
        return ChaosPhase::Initial;
    }
    let conds = compute_conditions(records, desired, paused);
    match desired {
        DesiredPhase::Run => {
            if conds.all_injected {
                ChaosPhase::Running
            } else {
                ChaosPhase::Injecting
            }
        }
        DesiredPhase::Stop => {
            if conds.all_recovered {
                ChaosPhase::Finished
            } else {
                ChaosPhase::Recovering
            }
        }
    }
}

/// Whether the phase is terminal (no further reconciliation needed).
pub fn is_terminal(phase: &ChaosPhase) -> bool {
    matches!(phase, ChaosPhase::Finished)
}
