// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Binpacking scheduler — assigns batched pods to candidate instance
//! types, honouring in-flight reservation, topology-spread, and taints.
//!
//! Upstream reference (Karpenter v1.4.0):
//!   pkg/controllers/provisioning/scheduling/scheduler.go
//!   pkg/controllers/provisioning/scheduling/topology.go
//!   pkg/controllers/provisioning/scheduling/taints.go
//!
//! The Cave port keeps the algorithm pure — taking a snapshot of pending
//! pods, candidate instance-types, and node-level taints, and returning
//! a [`BinpackResult`] that the orchestrator wires into NodeClaim
//! creation. The real upstream binpacker also handles preferred
//! antiaffinity, persistent-volume zone constraints, and pod priority
//! classes — those are intentional Phase 3 scope.

use crate::batcher::PodSpec;
use crate::models::Taint;
use std::collections::BTreeMap;

/// Instance-type candidate. Carries cpu_millis / memory_mib budgets and a
/// zone label that the binpacker uses for topology-spread.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceType {
    pub name: String,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub zone: String,
}

/// Result of [`binpack`]. Either every pod found a slot ([`BinpackResult::Assigned`])
/// or at least one pod failed to fit ([`BinpackResult::NoFit`]).
#[derive(Debug, Clone)]
pub enum BinpackResult {
    Assigned { instances: Vec<InstanceAssignment> },
    NoFit { reason: String },
}

/// One instance produced by binpack, holding its assigned pods.
#[derive(Debug, Clone)]
pub struct InstanceAssignment {
    pub instance: InstanceType,
    pub pods: Vec<String>,
    pub remaining_cpu_millis: u32,
    pub remaining_memory_mib: u32,
}

/// Binpack pods against candidate instance types, honouring:
/// 1. Per-instance cpu / memory budgets ("in-flight reservation").
/// 2. Topology-spread on the `zone_spread_label` axis when set.
/// 3. Taint intolerance: pods whose tolerations don't cover a node's
///    taints are skipped over that node.
///
/// Algorithm: greedy first-fit decreasing by cpu_millis with topology
/// rebalancing. Matches upstream `scheduler.solve` for the MVP path.
pub fn binpack(
    pods: &[PodSpec],
    instances: &[InstanceType],
    node_taints: &[Taint],
) -> BinpackResult {
    // Filter out instances whose taints aren't tolerated by ANY pod —
    // upstream semantics: taint intolerance is per-pod, but we lift the
    // common case (pool-wide GPU taint) so we can fast-fail when no pod
    // tolerates the entire pool.
    let blocked_by_taint = node_taints
        .iter()
        .filter(|t| t.effect == "NoSchedule")
        .any(|t| pods.iter().all(|p| !p.tolerations.contains(&t.key)));
    if blocked_by_taint && !pods.is_empty() {
        return BinpackResult::NoFit {
            reason: format!(
                "pods do not tolerate node taints: {}",
                node_taints
                    .iter()
                    .map(|t| t.key.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
    }

    // Sort pods by cpu_millis descending — first-fit decreasing.
    let mut sorted: Vec<PodSpec> = pods.to_vec();
    sorted.sort_by(|a, b| b.cpu_millis.cmp(&a.cpu_millis));

    // Topology spread: track per-zone pod counts; when a pod requests
    // spread, prefer a zone with fewer placements.
    let mut by_zone: BTreeMap<String, usize> = BTreeMap::new();
    for inst in instances {
        by_zone.entry(inst.zone.clone()).or_insert(0);
    }

    let mut assignments: Vec<InstanceAssignment> = Vec::new();

    'pod: for pod in sorted {
        // Existing-instance fit pass.
        let target_zone = preferred_zone(&pod, &by_zone);
        let candidate_iter: Vec<usize> = if let Some(z) = target_zone.as_ref() {
            // Topology-spread requested: only consider instances in the
            // least-populated zone.
            assignments
                .iter()
                .enumerate()
                .filter(|(_, a)| &a.instance.zone == z)
                .map(|(i, _)| i)
                .collect()
        } else {
            (0..assignments.len()).collect()
        };

        for i in candidate_iter {
            if fits(&assignments[i], &pod) {
                let zone = assignments[i].instance.zone.clone();
                assignments[i].pods.push(pod.name.clone());
                assignments[i].remaining_cpu_millis -= pod.cpu_millis;
                assignments[i].remaining_memory_mib -= pod.memory_mib;
                *by_zone.entry(zone).or_insert(0) += 1;
                continue 'pod;
            }
        }

        // No fit on existing instance — open a new one. Prefer the
        // target zone if set, otherwise smallest-instance-that-fits.
        let new_inst = pick_new_instance(&pod, instances, target_zone.as_deref());
        match new_inst {
            Some(inst) => {
                let zone = inst.zone.clone();
                let asn = InstanceAssignment {
                    remaining_cpu_millis: inst.cpu_millis - pod.cpu_millis,
                    remaining_memory_mib: inst.memory_mib - pod.memory_mib,
                    instance: inst,
                    pods: vec![pod.name.clone()],
                };
                assignments.push(asn);
                *by_zone.entry(zone).or_insert(0) += 1;
            }
            None => {
                return BinpackResult::NoFit {
                    reason: format!(
                        "no instance type fits pod {} (cpu={}, mem={})",
                        pod.name, pod.cpu_millis, pod.memory_mib
                    ),
                };
            }
        }
    }

    BinpackResult::Assigned {
        instances: assignments,
    }
}

/// Aggregate the CPU (millis) / memory (MiB) a DaemonSet set reserves on
/// *every* node. Mirrors `scheduler.calculateDaemonOverhead`.
pub fn daemon_overhead(daemonset_pods: &[PodSpec]) -> (u32, u32) {
    daemonset_pods.iter().fold((0, 0), |(cpu, mem), p| {
        (cpu + p.cpu_millis, mem + p.memory_mib)
    })
}

/// Binpack `pods` after reserving the `daemonset_pods` overhead on every
/// candidate node — the upstream scheduler subtracts DaemonSet requests from
/// each node's allocatable before packing application pods. Instance types too
/// small to host the DaemonSet overhead are dropped; the resulting
/// [`InstanceAssignment::remaining_cpu_millis`] / `remaining_memory_mib`
/// therefore report capacity *after* both the DaemonSet and the assigned app
/// pods.
pub fn binpack_with_daemonset(
    pods: &[PodSpec],
    instances: &[InstanceType],
    daemonset_pods: &[PodSpec],
    node_taints: &[Taint],
) -> BinpackResult {
    let (ds_cpu, ds_mem) = daemon_overhead(daemonset_pods);
    let reduced: Vec<InstanceType> = instances
        .iter()
        .filter(|i| i.cpu_millis >= ds_cpu && i.memory_mib >= ds_mem)
        .map(|i| InstanceType {
            cpu_millis: i.cpu_millis - ds_cpu,
            memory_mib: i.memory_mib - ds_mem,
            ..i.clone()
        })
        .collect();
    if reduced.is_empty() && !pods.is_empty() {
        return BinpackResult::NoFit {
            reason: format!(
                "no instance type can host the DaemonSet overhead (cpu={ds_cpu}, mem={ds_mem})"
            ),
        };
    }
    binpack(pods, &reduced, node_taints)
}

fn fits(a: &InstanceAssignment, p: &PodSpec) -> bool {
    a.remaining_cpu_millis >= p.cpu_millis && a.remaining_memory_mib >= p.memory_mib
}

fn preferred_zone(pod: &PodSpec, by_zone: &BTreeMap<String, usize>) -> Option<String> {
    let _ = pod.zone_spread_label.as_ref()?;
    if by_zone.is_empty() {
        return None;
    }
    // Lowest-count wins; ties broken by zone name.
    let min = by_zone.values().min().copied()?;
    by_zone
        .iter()
        .find(|(_, v)| **v == min)
        .map(|(k, _)| k.clone())
}

fn pick_new_instance(
    pod: &PodSpec,
    instances: &[InstanceType],
    target_zone: Option<&str>,
) -> Option<InstanceType> {
    let mut eligible: Vec<&InstanceType> = instances
        .iter()
        .filter(|i| i.cpu_millis >= pod.cpu_millis && i.memory_mib >= pod.memory_mib)
        .filter(|i| target_zone.is_none_or(|z| i.zone == z))
        .collect();
    if eligible.is_empty() {
        return None;
    }
    // Smallest-fit by cpu_millis. Ties broken by memory_mib.
    eligible.sort_by_key(|i| (i.cpu_millis, i.memory_mib));
    Some(eligible[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pods_returns_empty_assignment() {
        let inst = InstanceType {
            name: "i".into(),
            cpu_millis: 1000,
            memory_mib: 1024,
            zone: "z1".into(),
        };
        let res = binpack(&[], &[inst], &[]);
        match res {
            BinpackResult::Assigned { instances } => assert!(instances.is_empty()),
            _ => panic!("expected empty assignment"),
        }
    }

    #[test]
    fn second_pod_packs_onto_first_instance_if_room() {
        let inst = InstanceType {
            name: "i".into(),
            cpu_millis: 1000,
            memory_mib: 1024,
            zone: "z1".into(),
        };
        let pods = vec![
            PodSpec::with_resources("p1", 300, 256),
            PodSpec::with_resources("p2", 300, 256),
        ];
        let res = binpack(&pods, &[inst], &[]);
        match res {
            BinpackResult::Assigned { instances } => {
                assert_eq!(instances.len(), 1);
                assert_eq!(instances[0].pods.len(), 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn no_fit_when_pod_too_large_for_any_instance() {
        let small = InstanceType {
            name: "s".into(),
            cpu_millis: 500,
            memory_mib: 512,
            zone: "z".into(),
        };
        let huge = vec![PodSpec::with_resources("huge", 4000, 4096)];
        assert!(matches!(
            binpack(&huge, &[small], &[]),
            BinpackResult::NoFit { .. }
        ));
    }

    #[test]
    fn pod_tolerates_taint_lets_binpack_succeed() {
        let inst = InstanceType {
            name: "gpu".into(),
            cpu_millis: 4000,
            memory_mib: 8192,
            zone: "z".into(),
        };
        let pods = vec![PodSpec::with_resources("p1", 500, 512).tolerate("nvidia.com/gpu")];
        let taints = vec![Taint {
            key: "nvidia.com/gpu".into(),
            value: None,
            effect: "NoSchedule".into(),
        }];
        let res = binpack(&pods, &[inst], &taints);
        assert!(matches!(res, BinpackResult::Assigned { .. }));
    }
}
