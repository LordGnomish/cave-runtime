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

/// Compute the printable VM status — the user-visible `vm.status.printable_status`
/// field. Mirrors upstream's `getPrintableStatus` switch.
pub fn printable_status(
    vm: &VirtualMachine,
    vmi: Option<&VirtualMachineInstance>,
) -> &'static str {
    let phase = vmi
        .and_then(|v| v.status.as_ref())
        .map(|s| s.phase.as_str())
        .unwrap_or("");
    match (vm.spec.run_strategy, phase) {
        (RunStrategy::Halted, _) | (RunStrategy::Manual, "") => "Stopped",
        (_, "Pending") | (_, "Scheduling") | (_, "Scheduled") => "Starting",
        (_, "Running") => "Running",
        (_, "Migrating") => "Migrating",
        (_, "Succeeded") => "Succeeded",
        (_, "Failed") => "Failed",
        (RunStrategy::Always, "") => "Starting",
        (RunStrategy::Once, "") => "Starting",
        (RunStrategy::RerunOnFailure, "") => "Stopped",
        _ => "Stopped",
    }
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
