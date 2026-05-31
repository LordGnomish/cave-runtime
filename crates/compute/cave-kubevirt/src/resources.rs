// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Resource-rendering helpers — deterministic pure-logic ports of
//! `pkg/virt-controller/services/renderresources.go`.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   pkg/virt-controller/services/renderresources.go  (calcVCPUs, getMemoryLimitsRatio)
//!   pkg/util/hardware/hw_utils.go                     (GetNumberOfVCPUs, ParseCPUSetLine)
//!
//! These are the schedulable-resource computations the virt-controller runs
//! before it hands a pod spec to the kubelet — vCPU accounting, CPU-set
//! pinning math, and the auto-memory-limit ratio. They carry no host/FFI
//! dependency, so they live in-tree (the privileged spawn path stays in
//! cave-runtime host-preflight).

use crate::models::DomainCpu;

/// `hardware.GetNumberOfVCPUs(cpuSpec)` — total vCPUs implied by a CPU
/// topology, using upstream's zero-fallback semantics.
///
/// vCPUs starts at `cores`. Each of `sockets`/`threads`, when non-zero, either
/// seeds the running total (if it is still zero) or multiplies into it. This
/// means `{sockets: 4}` alone yields 4, not 0 — matching upstream exactly.
pub fn number_of_vcpus(cpu: &DomainCpu) -> i64 {
    let mut vcpus = cpu.cores.unwrap_or(0) as i64;
    if let Some(sockets) = cpu.sockets.filter(|&s| s != 0) {
        let sockets = sockets as i64;
        vcpus = if vcpus == 0 { sockets } else { vcpus * sockets };
    }
    if let Some(threads) = cpu.threads.filter(|&t| t != 0) {
        let threads = threads as i64;
        vcpus = if vcpus == 0 { threads } else { vcpus * threads };
    }
    vcpus
}

/// `calcVCPUs(cpu)` — vCPU count for a VMI domain, defaulting to a single
/// vCPU when the CPU topology is entirely absent.
pub fn calc_vcpus(cpu: Option<&DomainCpu>) -> i64 {
    match cpu {
        Some(c) => number_of_vcpus(c),
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(cores: u32, sockets: u32, threads: u32) -> DomainCpu {
        DomainCpu {
            cores: (cores != 0).then_some(cores),
            sockets: (sockets != 0).then_some(sockets),
            threads: (threads != 0).then_some(threads),
            model: None,
        }
    }

    #[test]
    fn full_topology_multiplies_all_three() {
        assert_eq!(number_of_vcpus(&cpu(2, 2, 2)), 8);
        assert_eq!(number_of_vcpus(&cpu(4, 1, 1)), 4);
    }

    #[test]
    fn sockets_seed_total_when_cores_unset() {
        // cores=0 → sockets seeds the total rather than multiplying 0.
        assert_eq!(number_of_vcpus(&cpu(0, 4, 0)), 4);
        // cores=0, sockets=2, threads=2 → 2 then *2 = 4.
        assert_eq!(number_of_vcpus(&cpu(0, 2, 2)), 4);
    }

    #[test]
    fn threads_seed_total_when_cores_and_sockets_unset() {
        assert_eq!(number_of_vcpus(&cpu(0, 0, 2)), 2);
    }

    #[test]
    fn fully_unset_topology_is_zero() {
        assert_eq!(number_of_vcpus(&cpu(0, 0, 0)), 0);
    }

    #[test]
    fn calc_vcpus_defaults_to_one_when_absent() {
        assert_eq!(calc_vcpus(None), 1);
    }

    #[test]
    fn calc_vcpus_delegates_when_present() {
        assert_eq!(calc_vcpus(Some(&cpu(2, 1, 1))), 2);
        assert_eq!(calc_vcpus(Some(&cpu(0, 0, 0))), 0);
    }
}
