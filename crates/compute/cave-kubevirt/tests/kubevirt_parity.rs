// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests vs. upstream kubevirt/kubevirt v1.8.2.
//!
//! All tests are `#[cfg(feature = "live-integration")]` until the corresponding upstream behaviour is
//! reimplemented. They exist so the compatibility surface is enumerated rather
//! than silently missing.

use cave_kubevirt::*;

#[test]
#[cfg(feature = "live-integration")]
fn parity_vm_admission_rejects_invalid_runstrategy() {
    // upstream: pkg/virt-api/webhooks/validating-webhook/admitters/vm-admitter.go
    // expectation: a VM whose spec.runStrategy is not one of the enum values
    // is rejected at admission.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_vmi_launch_creates_virt_launcher_pod() {
    // upstream: pkg/virt-controller/watch/vmi.go
    // expectation: a VMI in Pending phase causes a virt-launcher Pod to be
    // created in the same namespace and the VMI moves to Scheduling.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_live_migration_succeeds_under_node_drain() {
    // upstream: pkg/virt-controller/watch/migration.go
    // expectation: under EvictionStrategy=LiveMigrate, draining a node
    // triggers VirtualMachineInstanceMigration → succeeded transition.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_data_volume_population_blocks_vm_start() {
    // upstream: pkg/virt-controller/watch/vm.go (DataVolume readiness gate)
    // expectation: a VM with a DataVolumeTemplate whose status is not yet
    // Succeeded must not transition past WaitingForVolumeBinding.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_instancetype_overrides_match_inflation() {
    // upstream: pkg/instancetype/instancetype.go
    // expectation: spec.instancetype.name pulls the cluster-scoped Instancetype
    // and applies CPU/memory/GPU constraints; conflicts in vmi.spec.domain
    // are rejected.
    unimplemented!()
}
