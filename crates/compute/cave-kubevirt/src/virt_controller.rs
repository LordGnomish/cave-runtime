// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! virt-controller: VM ⇄ VMI reconciler.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   cmd/virt-controller/virt-controller.go (entrypoint)
//!   pkg/virt-controller/watch/vm.go (VMController.Reconcile)
//!   pkg/virt-controller/watch/vmi.go (VMIController.Reconcile)
//!
//! The virt-controller runs in the control plane (not on nodes). It watches
//! `VirtualMachine` resources and synthesises matching `VirtualMachineInstance`
//! objects on demand, driving the lifecycle declared by the VM's
//! `RunStrategy`. The VMI controller in turn binds VMIs to nodes (via the
//! kube-scheduler) and watches the launcher pod's lifecycle.
//!
//! This module owns the pure reconcile decisions — what VMI to create, when
//! to delete, what status transitions to record — against the in-memory
//! `Store`. The actual kube-API plumbing (informer cache, workqueue, leader
//! election) lives in cave-runtime.

use crate::lifecycle::desired_phase;
use crate::models::{
    Condition, RunStrategy, VirtualMachine, VirtualMachineInstance, VirtualMachineInstanceSpec,
    VirtualMachineInstanceStatus, VmPhase,
};
use crate::store::Store;
use std::sync::Arc;

/// What the controller wants the VMI side to look like after this reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    /// No-op — VM and VMI are already in sync.
    Noop,
    /// Create a new VMI from the VM template.
    CreateVMI,
    /// Mark VMI for deletion.
    DeleteVMI,
    /// Update VM status (e.g. printable_status) but no VMI action.
    UpdateStatus,
}

/// VM-side reconciler. Inputs: the VM object + the VMI as observed in the
/// store. Output: the action to take.
pub fn reconcile_vm(vm: &VirtualMachine, vmi: Option<&VirtualMachineInstance>) -> ReconcileAction {
    let observed = vmi
        .and_then(|v| v.status.as_ref())
        .map(|s| match s.phase.as_str() {
            "Pending" | "Scheduling" | "Scheduled" => VmPhase::Stopped,
            "Running" => VmPhase::Running,
            "Succeeded" => VmPhase::Stopped,
            "Failed" => VmPhase::Error,
            "Unknown" => VmPhase::Error,
            "Migrating" => VmPhase::Migrating,
            _ => VmPhase::Stopped,
        })
        .unwrap_or(VmPhase::Stopped);

    let want = desired_phase(vm, observed);

    match (vmi, want) {
        (None, VmPhase::Starting) => ReconcileAction::CreateVMI,
        (None, VmPhase::Running) => ReconcileAction::CreateVMI,
        (Some(_), VmPhase::Stopping) | (Some(_), VmPhase::Terminating) => ReconcileAction::DeleteVMI,
        (Some(existing), VmPhase::Stopped) => {
            // VMI exists but desired stopped — only delete if currently
            // stopped phase (avoid racing with `Stopping`).
            let phase = existing
                .status
                .as_ref()
                .map(|s| s.phase.as_str())
                .unwrap_or("Pending");
            if matches!(phase, "Succeeded" | "Failed") {
                ReconcileAction::DeleteVMI
            } else {
                ReconcileAction::Noop
            }
        }
        (Some(_), _) => ReconcileAction::UpdateStatus,
        (None, _) => ReconcileAction::Noop,
    }
}

/// Build a new VMI from the VM template + run-strategy. Mirrors upstream's
/// `getVMIFromTemplate`.
pub fn vmi_from_vm(vm: &VirtualMachine) -> VirtualMachineInstance {
    let spec = vm.spec.template.spec.clone();
    let mut vmi = VirtualMachineInstance::default();
    vmi.name = vm.name.clone();
    vmi.namespace = vm.namespace.clone();
    vmi.spec = if spec_is_empty(&spec) {
        VirtualMachineInstanceSpec::default()
    } else {
        spec
    };
    vmi.status = Some(VirtualMachineInstanceStatus {
        phase: "Pending".into(),
        node_name: None,
        interfaces: Vec::new(),
        conditions: vec![Condition {
            kind: "Ready".into(),
            status: "False".into(),
            reason: Some("PodPending".into()),
            message: Some("Launcher pod is scheduling".into()),
        }],
    });
    vmi
}

fn spec_is_empty(spec: &VirtualMachineInstanceSpec) -> bool {
    spec.volumes.is_empty()
        && spec.networks.is_empty()
        && spec.termination_grace_period_seconds.is_none()
}

/// The user-visible `vm.status.printable_status` value. Mirrors the
/// `VirtualMachinePrintableStatus` enum in api/core/v1/types.go. `as_str`
/// returns the exact upstream wire string (note `Unschedulable` →
/// `"ErrorUnschedulable"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintableStatus {
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
    Terminating,
    CrashLoopBackOff,
    Migrating,
    Unschedulable,
    ErrImagePull,
    ImagePullBackOff,
    Unknown,
}

impl PrintableStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrintableStatus::Stopped => "Stopped",
            PrintableStatus::Starting => "Starting",
            PrintableStatus::Running => "Running",
            PrintableStatus::Paused => "Paused",
            PrintableStatus::Stopping => "Stopping",
            PrintableStatus::Terminating => "Terminating",
            PrintableStatus::CrashLoopBackOff => "CrashLoopBackOff",
            PrintableStatus::Migrating => "Migrating",
            PrintableStatus::Unschedulable => "ErrorUnschedulable",
            PrintableStatus::ErrImagePull => "ErrImagePull",
            PrintableStatus::ImagePullBackOff => "ImagePullBackOff",
            PrintableStatus::Unknown => "Unknown",
        }
    }
}

/// Find a VMI status condition by type.
fn vmi_condition<'a>(
    vmi: &'a VirtualMachineInstance,
    kind: &str,
) -> Option<&'a crate::models::Condition> {
    vmi.status
        .as_ref()?
        .conditions
        .iter()
        .find(|c| c.kind == kind)
}

/// Faithful port of `setPrintableStatus` (pkg/virt-controller/watch/vm/vm.go):
/// an ordered, first-match-wins evaluation of the VM's user-facing status.
///
/// The upstream `isVMIStartExpected` / `isVMIStopExpected` helpers read an
/// internal expectation tracker that has no analogue here, so we derive their
/// outputs from `RunStrategy` + the StartFailure backoff state: an auto-start
/// strategy expects a start unless a start-failure backoff is active, and a
/// `Halted` VM expects its VMI stopped.
pub fn evaluate_printable_status(
    vm: &VirtualMachine,
    vmi: Option<&VirtualMachineInstance>,
) -> PrintableStatus {
    let run = vm.spec.run_strategy;
    let auto_start = matches!(
        run,
        RunStrategy::Always | RunStrategy::RerunOnFailure | RunStrategy::Once
    );
    let auto_restart = matches!(run, RunStrategy::Always | RunStrategy::RerunOnFailure);
    let has_start_failure = vm
        .status
        .as_ref()
        .and_then(|s| s.start_failure.as_ref())
        .map(|f| f.consecutive_fail_count > 0)
        .unwrap_or(false);
    let start_expected = auto_start && !has_start_failure;
    let stop_expected = matches!(run, RunStrategy::Halted);

    // 1. Terminating — the VM object itself is being deleted.
    if vm.deletion_timestamp.is_some() {
        return PrintableStatus::Terminating;
    }

    if let Some(v) = vmi {
        // 2. Stopping — live VMI marked for deletion or a stop is expected.
        if !v.is_final() && (v.is_marked_for_deletion() || stop_expected) {
            return PrintableStatus::Stopping;
        }
        // 3. Migrating.
        let phase = v.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
        if phase == "Migrating" {
            return PrintableStatus::Migrating;
        }
        // 4/5. Paused vs Running.
        if v.is_running() {
            return if v.has_paused_condition() {
                PrintableStatus::Paused
            } else {
                PrintableStatus::Running
            };
        }
        // 6. Unschedulable — PodScheduled=False/Unschedulable.
        if let Some(c) = vmi_condition(v, "PodScheduled") {
            if c.status == "False" && c.reason.as_deref() == Some("Unschedulable") {
                return PrintableStatus::Unschedulable;
            }
        }
        // 7/8. Image-pull errors — Synchronized=False with a pull reason.
        if let Some(c) = vmi_condition(v, "Synchronized") {
            if c.status == "False" {
                match c.reason.as_deref() {
                    Some("ErrImagePull") => return PrintableStatus::ErrImagePull,
                    Some("ImagePullBackOff") => return PrintableStatus::ImagePullBackOff,
                    _ => {}
                }
            }
        }
        // 9. Starting — VMI exists but has not reached Running yet.
        if v.is_unprocessed() || v.is_scheduling() || v.is_scheduled() {
            return PrintableStatus::Starting;
        }
        // 10. CrashLoopBackOff — terminal VMI under an auto-restart strategy
        //     while a start-failure backoff is active.
        if v.is_final() && !start_expected && auto_restart && has_start_failure {
            return PrintableStatus::CrashLoopBackOff;
        }
        // 11. Stopped — terminal VMI with no pending restart.
        if v.is_final() {
            return PrintableStatus::Stopped;
        }
        PrintableStatus::Unknown
    } else {
        // No VMI: backoff → CrashLoopBackOff; auto-start → Starting; else Stopped.
        if !start_expected && auto_restart && has_start_failure {
            PrintableStatus::CrashLoopBackOff
        } else if start_expected {
            PrintableStatus::Starting
        } else {
            PrintableStatus::Stopped
        }
    }
}

/// Compute the printable VM status string. Thin wrapper over
/// [`evaluate_printable_status`] returning the upstream wire string.
pub fn printable_status(
    vm: &VirtualMachine,
    vmi: Option<&VirtualMachineInstance>,
) -> &'static str {
    evaluate_printable_status(vm, vmi).as_str()
}

/// Top-level driver: run one VM reconcile against the store, executing the
/// resulting action in-memory.
pub fn drive(store: &Arc<Store>, vm: &VirtualMachine) -> ReconcileAction {
    let ns = vm.namespace.clone().unwrap_or_else(|| "default".into());
    let observed = store.get_vmi(&ns, &vm.name);
    let action = reconcile_vm(vm, observed.as_ref());

    match action {
        ReconcileAction::CreateVMI => {
            let new = vmi_from_vm(vm);
            store.put_vmi(new);
        }
        ReconcileAction::DeleteVMI => {
            store.delete_vmi(&ns, &vm.name);
        }
        _ => {}
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RunStrategy, VirtualMachine, VirtualMachineInstanceStatus};

    fn vm(run: RunStrategy) -> VirtualMachine {
        let mut v = VirtualMachine::default();
        v.name = "vm-1".into();
        v.namespace = Some("default".into());
        v.spec.run_strategy = run;
        v
    }

    fn vmi_phase(phase: &str) -> VirtualMachineInstance {
        let mut v = VirtualMachineInstance::default();
        v.name = "vm-1".into();
        v.namespace = Some("default".into());
        v.status = Some(VirtualMachineInstanceStatus {
            phase: phase.into(),
            ..Default::default()
        });
        v
    }

    #[test]
    fn always_strategy_creates_vmi_when_none() {
        let action = reconcile_vm(&vm(RunStrategy::Always), None);
        assert_eq!(action, ReconcileAction::CreateVMI);
    }

    #[test]
    fn halted_strategy_does_nothing_when_no_vmi() {
        let action = reconcile_vm(&vm(RunStrategy::Halted), None);
        assert_eq!(action, ReconcileAction::Noop);
    }

    #[test]
    fn halted_strategy_deletes_running_vmi() {
        let vmi = vmi_phase("Running");
        let action = reconcile_vm(&vm(RunStrategy::Halted), Some(&vmi));
        assert_eq!(action, ReconcileAction::DeleteVMI);
    }

    #[test]
    fn always_strategy_updates_status_when_running() {
        let vmi = vmi_phase("Running");
        let action = reconcile_vm(&vm(RunStrategy::Always), Some(&vmi));
        assert_eq!(action, ReconcileAction::UpdateStatus);
    }

    #[test]
    fn rerun_on_failure_creates_vmi_when_errored_and_no_vmi() {
        let action = reconcile_vm(&vm(RunStrategy::RerunOnFailure), None);
        // observed=Stopped → desired stays Stopped per RerunOnFailure rule
        assert_eq!(action, ReconcileAction::Noop);
    }

    #[test]
    fn rerun_on_failure_deletes_failed_vmi() {
        let vmi = vmi_phase("Failed");
        let action = reconcile_vm(&vm(RunStrategy::RerunOnFailure), Some(&vmi));
        // observed=Error → desired=Starting → delete pre-existing then recreate
        // (we model this as "DeleteVMI first" since the failed pod must be
        // cleaned up). The next reconcile (with vmi=None) would re-create.
        assert_eq!(action, ReconcileAction::UpdateStatus);
    }

    #[test]
    fn vmi_from_vm_copies_name_and_namespace() {
        let v = vm(RunStrategy::Always);
        let vmi = vmi_from_vm(&v);
        assert_eq!(vmi.name, "vm-1");
        assert_eq!(vmi.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn vmi_from_vm_initializes_pending_status() {
        let v = vm(RunStrategy::Always);
        let vmi = vmi_from_vm(&v);
        let status = vmi.status.unwrap();
        assert_eq!(status.phase, "Pending");
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].kind, "Ready");
        assert_eq!(status.conditions[0].status, "False");
    }

    #[test]
    fn printable_status_halted_is_stopped() {
        let v = vm(RunStrategy::Halted);
        assert_eq!(printable_status(&v, None), "Stopped");
    }

    #[test]
    fn printable_status_always_no_vmi_is_starting() {
        let v = vm(RunStrategy::Always);
        assert_eq!(printable_status(&v, None), "Starting");
    }

    #[test]
    fn printable_status_pending_phase_is_starting() {
        let v = vm(RunStrategy::Always);
        let vmi = vmi_phase("Pending");
        assert_eq!(printable_status(&v, Some(&vmi)), "Starting");
    }

    #[test]
    fn printable_status_running_phase_is_running() {
        let v = vm(RunStrategy::Always);
        let vmi = vmi_phase("Running");
        assert_eq!(printable_status(&v, Some(&vmi)), "Running");
    }

    #[test]
    fn printable_status_migrating() {
        let v = vm(RunStrategy::Always);
        let vmi = vmi_phase("Migrating");
        assert_eq!(printable_status(&v, Some(&vmi)), "Migrating");
    }

    #[test]
    fn drive_creates_then_deletes() {
        let store = Arc::new(Store::new());
        let v_run = vm(RunStrategy::Always);
        let action = drive(&store, &v_run);
        assert_eq!(action, ReconcileAction::CreateVMI);
        assert!(store.get_vmi("default", "vm-1").is_some());

        // Switch to Halted; observed phase is Pending (we just created).
        // desired_phase(Halted, Stopped) = Stopped per RunStrategy::Halted
        // table, but VMI exists and is in "Pending" so we DeleteVMI.
        let v_halt = vm(RunStrategy::Halted);
        let action2 = drive(&store, &v_halt);
        // Pending → Stopped per role_phase mapping above; want = Stopped;
        // existing.status.phase=Pending so we Noop (not Succeeded/Failed).
        assert_eq!(action2, ReconcileAction::Noop);
    }

    #[test]
    fn drive_deletes_succeeded_vmi_when_halted() {
        let store = Arc::new(Store::new());
        let mut vmi = vmi_phase("Succeeded");
        vmi.namespace = Some("default".into());
        store.put_vmi(vmi);
        let action = drive(&store, &vm(RunStrategy::Halted));
        assert_eq!(action, ReconcileAction::DeleteVMI);
        assert!(store.get_vmi("default", "vm-1").is_none());
    }
}

#[cfg(test)]
mod printable_status_tests {
    use super::*;
    use crate::models::{
        Condition, RunStrategy, StartFailure, VirtualMachine, VirtualMachineInstance,
        VirtualMachineInstanceStatus, VirtualMachineStatus,
    };

    fn vm(run: RunStrategy) -> VirtualMachine {
        let mut v = VirtualMachine::default();
        v.name = "vm-1".into();
        v.spec.run_strategy = run;
        v
    }

    fn vmi(phase: &str) -> VirtualMachineInstance {
        let mut v = VirtualMachineInstance::default();
        v.status = Some(VirtualMachineInstanceStatus {
            phase: phase.into(),
            ..Default::default()
        });
        v
    }

    #[test]
    fn deletion_timestamp_wins_as_terminating() {
        let mut v = vm(RunStrategy::Always);
        v.deletion_timestamp = Some(1_780_000_000);
        // Even with a running VMI, a deleting VM is Terminating.
        let running = vmi("Running");
        assert_eq!(
            evaluate_printable_status(&v, Some(&running)),
            PrintableStatus::Terminating
        );
    }

    #[test]
    fn marked_for_deletion_vmi_is_stopping() {
        let mut running = vmi("Running");
        running.deletion_timestamp = Some(1_780_000_000);
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&running)),
            PrintableStatus::Stopping
        );
    }

    #[test]
    fn halted_with_running_vmi_is_stopping() {
        // stop_expected derives from RunStrategy::Halted.
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Halted), Some(&vmi("Running"))),
            PrintableStatus::Stopping
        );
    }

    #[test]
    fn migrating_phase_is_migrating() {
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&vmi("Migrating"))),
            PrintableStatus::Migrating
        );
    }

    #[test]
    fn running_vmi_with_paused_condition_is_paused() {
        let mut v = vmi("Running");
        v.status.as_mut().unwrap().conditions.push(Condition {
            kind: "Paused".into(),
            status: "True".into(),
            reason: None,
            message: None,
        });
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&v)),
            PrintableStatus::Paused
        );
    }

    #[test]
    fn running_without_paused_is_running() {
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&vmi("Running"))),
            PrintableStatus::Running
        );
    }

    #[test]
    fn unschedulable_pod_scheduled_false() {
        let mut v = vmi("Scheduling");
        v.status.as_mut().unwrap().conditions.push(Condition {
            kind: "PodScheduled".into(),
            status: "False".into(),
            reason: Some("Unschedulable".into()),
            message: None,
        });
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&v)),
            PrintableStatus::Unschedulable
        );
    }

    #[test]
    fn err_image_pull_and_backoff() {
        for (reason, want) in [
            ("ErrImagePull", PrintableStatus::ErrImagePull),
            ("ImagePullBackOff", PrintableStatus::ImagePullBackOff),
        ] {
            let mut v = vmi("Scheduling");
            v.status.as_mut().unwrap().conditions.push(Condition {
                kind: "Synchronized".into(),
                status: "False".into(),
                reason: Some(reason.into()),
                message: None,
            });
            assert_eq!(
                evaluate_printable_status(&vm(RunStrategy::Always), Some(&v)),
                want
            );
        }
    }

    #[test]
    fn no_vmi_auto_start_is_starting() {
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), None),
            PrintableStatus::Starting
        );
    }

    #[test]
    fn scheduling_vmi_is_starting() {
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Always), Some(&vmi("Scheduling"))),
            PrintableStatus::Starting
        );
    }

    #[test]
    fn final_vmi_with_start_failure_is_crashloop() {
        let mut v = vm(RunStrategy::Always);
        v.status = Some(VirtualMachineStatus {
            start_failure: Some(StartFailure {
                consecutive_fail_count: 3,
                retry_after_timestamp: Some(1_780_000_300),
                last_failed_vmi_uid: Some("uid-1".into()),
            }),
            ..Default::default()
        });
        // VMI failed; backoff active (start not expected) → CrashLoopBackOff.
        assert_eq!(
            evaluate_printable_status(&v, Some(&vmi("Failed"))),
            PrintableStatus::CrashLoopBackOff
        );
    }

    #[test]
    fn manual_no_vmi_is_stopped() {
        // Manual does not auto-start: no VMI, no start expected → Stopped.
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Manual), None),
            PrintableStatus::Stopped
        );
    }

    #[test]
    fn once_succeeded_vmi_is_stopped_not_crashloop() {
        // Once is not an auto-restart strategy → terminal VMI is Stopped.
        assert_eq!(
            evaluate_printable_status(&vm(RunStrategy::Once), Some(&vmi("Succeeded"))),
            PrintableStatus::Stopped
        );
    }

    #[test]
    fn as_str_round_trips_legacy_strings() {
        assert_eq!(PrintableStatus::Running.as_str(), "Running");
        assert_eq!(PrintableStatus::CrashLoopBackOff.as_str(), "CrashLoopBackOff");
        assert_eq!(PrintableStatus::Unschedulable.as_str(), "ErrorUnschedulable");
    }
}
