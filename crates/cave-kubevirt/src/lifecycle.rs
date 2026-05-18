// SPDX-License-Identifier: AGPL-3.0-or-later
//! VM lifecycle stub.
//!
//! Upstream reference: pkg/virt-controller/watch/vm.go (Reconcile)
//!
//! The real controller resolves desired phase from `RunStrategy` + observed
//! VMI status + DataVolume readiness. This stub exposes the decision function
//! only; production logic (defragmentation, live-migration, hotplug) is pending.

use crate::models::{RunStrategy, VirtualMachine, VmPhase};

/// Compute the desired VM phase given the VM spec and the *current* observed phase.
/// Mirrors the upstream `desiredState` switch but covers only the basic
/// start/stop transitions — migration, manual override, and pause are not yet
/// modeled.
pub fn desired_phase(vm: &VirtualMachine, observed: VmPhase) -> VmPhase {
    match vm.spec.run_strategy {
        RunStrategy::Always => match observed {
            VmPhase::Stopped | VmPhase::Error | VmPhase::Terminating => VmPhase::Starting,
            other => other,
        },
        RunStrategy::Halted => match observed {
            VmPhase::Running | VmPhase::Starting => VmPhase::Stopping,
            VmPhase::Stopping => VmPhase::Stopped,
            other => other,
        },
        RunStrategy::RerunOnFailure => match observed {
            VmPhase::Error => VmPhase::Starting,
            VmPhase::Stopped => VmPhase::Stopped, // intentional: don't auto-restart on clean stop
            other => other,
        },
        RunStrategy::Manual => observed,
        RunStrategy::Once => match observed {
            VmPhase::Stopped => VmPhase::Stopped, // already ran once
            other => other,
        },
    }
}
