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

/// `hardware.safeAppend` — push `cpu_num` onto `list`, refusing once the list
/// has already grown past `limit`. Upstream checks `len > limit` *before* the
/// append, so a list may legitimately reach `limit + 1` elements.
fn safe_append(list: &mut Vec<i32>, cpu_num: i32, limit: usize) -> Result<(), String> {
    if list.len() > limit {
        return Err(format!(
            "cpuset line exceeds the limit of {limit} cpus"
        ));
    }
    list.push(cpu_num);
    Ok(())
}

/// `hardware.ParseCPUSetLine(cpusetLine, limit)` — expand a Linux cpuset string
/// such as `"0-3,7"` into the explicit CPU id list `[0,1,2,3,7]`.
///
/// Comma-separated items; an item containing `-` is an inclusive range. Every
/// id is bounds-checked against `limit` via [`safe_append`]. A non-numeric
/// token, or overflowing the limit, returns `Err`.
pub fn parse_cpu_set_line(cpuset_line: &str, limit: usize) -> Result<Vec<i32>, String> {
    let mut cpus_list: Vec<i32> = Vec::new();
    for item in cpuset_line.split(',') {
        // Mirror upstream `strings.Split(item, "-")` + index access: a range
        // uses elements [0] and [1] and ignores any further dashes.
        let cpu_range: Vec<&str> = item.split('-').collect();
        if cpu_range.len() > 1 {
            // Provided a range: "1-3".
            let start: i32 = cpu_range[0].parse().map_err(|_| invalid(cpu_range[0]))?;
            let end: i32 = cpu_range[1].parse().map_err(|_| invalid(cpu_range[1]))?;
            let mut n = start;
            while n <= end {
                safe_append(&mut cpus_list, n, limit)?;
                n += 1;
            }
        } else {
            let n: i32 = cpu_range[0].parse().map_err(|_| invalid(cpu_range[0]))?;
            safe_append(&mut cpus_list, n, limit)?;
        }
    }
    Ok(cpus_list)
}

fn invalid(token: &str) -> String {
    format!("invalid cpuset token: {token:?}")
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

    #[test]
    fn parse_single_range() {
        assert_eq!(parse_cpu_set_line("0-3", 100).unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_mixed_ranges_and_singletons() {
        assert_eq!(
            parse_cpu_set_line("0-3,7", 100).unwrap(),
            vec![0, 1, 2, 3, 7]
        );
        assert_eq!(parse_cpu_set_line("1,3,5", 100).unwrap(), vec![1, 3, 5]);
    }

    #[test]
    fn parse_degenerate_range_is_one_cpu() {
        assert_eq!(parse_cpu_set_line("5-5", 100).unwrap(), vec![5]);
    }

    #[test]
    fn parse_rejects_non_numeric_tokens() {
        assert!(parse_cpu_set_line("a-b", 100).is_err());
        assert!(parse_cpu_set_line("x", 100).is_err());
    }

    #[test]
    fn parse_enforces_limit_with_upstream_off_by_one() {
        // Upstream checks len > limit *before* the append, so a list may reach
        // limit + 1 elements: "0-2" (3 ids) is accepted with limit 2 ...
        assert_eq!(parse_cpu_set_line("0-2", 2).unwrap(), vec![0, 1, 2]);
        // ... but a 4th id (len already 3 > 2 at the next check) is rejected.
        assert!(parse_cpu_set_line("0-3", 2).is_err());
    }
}
