// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-kubevirt: KubeVirt VM-as-K8s-pod reimplementation (scaffold).
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!
//! Modules:
//!   models      — VirtualMachine / VirtualMachineInstance / DataVolume CRDs
//!   store       — In-memory store (RwLock) for the scaffold; real persistence pending
//!   lifecycle   — RunStrategy → desired phase decision; full reconcile pending
//!
//! 4-track status (honest):
//!   Backend    1/4 — this scaffold
//!   Portal     0/4 — admin page not yet wired
//!   cavectl    0/4 — `cavectl kubevirt` not yet wired
//!   Observ.    0/4 — alerts + dashboard not yet authored

/// Re-export the `lifecycle` module for public access.
pub mod lifecycle;

/// Re-export the `models` module containing CRD structs.
pub mod models;

/// Re-export the `store` module for in-memory state management.
pub mod store;

/// Re-export the `desired_phase` function from the lifecycle module.
pub use lifecycle::desired_phase;

/// Re-export core model structs from the models module.
pub use models::{
    Condition, DataVolume, DataVolumeSource, DataVolumeSpec, DataVolumeStatus,
    DataVolumeTemplate, Domain, DomainCpu, DomainMemory, Firmware, HugepagesSpec, InstancetypeRef,
    Network, NetworkInterfaceStatus, PreferenceRef, PvcSpec, RunStrategy, VirtualMachine,
    VirtualMachineInstance, VirtualMachineInstanceSpec, VirtualMachineInstanceStatus,
    VirtualMachineInstanceTemplateSpec, VirtualMachineSpec, VirtualMachineStatus, VmPhase, Volume,
};

/// Re-export the `Store` type from the store module.
pub use store::Store;

/// The name of this module/crate.
pub const MODULE_NAME: &str = "cave-kubevirt";

/// The upstream KubeVirt repository identifier.
pub const UPSTREAM_REPO: &str = "kubevirt/kubevirt";

/// The upstream KubeVirt version string.
pub const UPSTREAM_VERSION: &str = "v1.8.2";

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(run: RunStrategy) -> VirtualMachine {
        let mut v = VirtualMachine::default();
        v.name = "vm-1".to_string();
        v.namespace = Some("default".to_string());
        v.spec.run_strategy = run;
        v
    }

    #[test]
    fn store_round_trips_vm() {
        let s = Store::new();
        s.put_vm(vm(RunStrategy::Halted));
        assert_eq!(s.list_vms().len(), 1);
        assert!(s.get_vm("default", "vm-1").is_some());
        assert!(s.delete_vm("default", "vm-1"));
        assert!(s.list_vms().is_empty());
    }

    #[test]
    fn run_strategy_always_starts_stopped_vm() {
        assert_eq!(desired_phase(&vm(RunStrategy::Always), VmPhase::Stopped), VmPhase::Starting);
    }

    #[test]
    fn run_strategy_always_does_not_disturb_running_vm() {
        assert_eq!(desired_phase(&vm(RunStrategy::Always), VmPhase::Running), VmPhase::Running);
    }

    #[test]
    fn run_strategy_halted_stops_running_vm() {
        assert_eq!(desired_phase(&vm(RunStrategy::Halted), VmPhase::Running), VmPhase::Stopping);
    }

    #[test]
    fn run_strategy_rerun_on_failure_restarts_after_error() {
        assert_eq!(desired_phase(&vm(RunStrategy::RerunOnFailure), VmPhase::Error), VmPhase::Starting);
    }

    #[test]
    fn run_strategy_rerun_on_failure_does_not_restart_after_clean_stop() {
        assert_eq!(desired_phase(&vm(RunStrategy::RerunOnFailure), VmPhase::Stopped), VmPhase::Stopped);
    }

    #[test]
    fn run_strategy_manual_is_no_op() {
        assert_eq!(desired_phase(&vm(RunStrategy::Manual), VmPhase::Running), VmPhase::Running);
        assert_eq!(desired_phase(&vm(RunStrategy::Manual), VmPhase::Stopped), VmPhase::Stopped);
    }

    #[test]
    fn module_constants_exposed() {
        assert_eq!(MODULE_NAME, "cave-kubevirt");
        assert_eq!(UPSTREAM_REPO, "kubevirt/kubevirt");
        assert!(UPSTREAM_VERSION.starts_with('v'));
    }
}
