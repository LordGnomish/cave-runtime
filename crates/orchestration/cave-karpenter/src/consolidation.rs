// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consolidation decision engine — merge under-utilised nodes into fewer.
//!
//! Faithful port of kubernetes-sigs/karpenter v1.4.0:
//!   pkg/controllers/disruption/consolidation.go            → [`compute_consolidation`]
//!   pkg/controllers/disruption/multinodeconsolidation.go   → [`multi_node_consolidation`]
//!   pkg/controllers/disruption/singlenodeconsolidation.go  → [`single_node_consolidation`]
//!   pkg/controllers/disruption/types.go                    → [`Command`] / [`Decision`]
//!
//! Upstream reconciles against a live API server and a cluster-state
//! snapshot; it computes a consolidation option by *simulating scheduling*
//! the candidates' reschedulable pods onto the rest of the cluster. If the
//! pods fit the surviving nodes the candidates are simply removed
//! (`DeleteDecision`); if they fit on a single cheaper node the candidates
//! are replaced by it (`ReplaceDecision`); otherwise nothing is done
//! (`NoOpDecision`).
//!
//! The Cave port keeps that decision logic pure: it takes a snapshot of the
//! candidate nodes (with their price + reschedulable pods), the free
//! capacity left on the surviving cluster, and the launchable instance
//! offerings, and returns a [`Command`]. The orchestration queue, validation
//! TTL re-check, and spot-to-spot price churn guards are out of scope here
//! (they live with the cluster-state controller); the multi-node binary
//! search and the Delete/Replace economics are ported exactly.

use crate::batcher::PodSpec;
use std::collections::HashMap;

/// A launchable instance offering — `cloudprovider.InstanceType.Offerings`
/// flattened to the single (capacity, zone, price) tuple the simulator
/// needs. Price is what `getCandidatePrices` / `OrderByPrice` compare on.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceOffering {
    pub name: String,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub zone: String,
    pub price: f64,
}

/// Free capacity left on a surviving (non-candidate) node — the in-flight
/// reservation the scheduler can pack onto before opening new nodes.
#[derive(Debug, Clone, Copy)]
pub struct NodeCapacity {
    pub free_cpu_millis: u32,
    pub free_memory_mib: u32,
}

/// A node under consideration for disruption. Mirrors `disruption.Candidate`
/// reduced to the fields the consolidation math touches: the owning
/// NodePool, the node's current instance type + price, its zone, the pods
/// that would need rescheduling, and the disruption cost used for ordering.
#[derive(Debug, Clone)]
pub struct ConsolidationCandidate {
    pub claim_name: String,
    pub nodepool: String,
    pub instance_type: String,
    pub zone: String,
    pub price: f64,
    pub disruption_cost: f64,
    pub reschedulable_pods: Vec<PodSpec>,
}

/// A node the simulation proposes launching to absorb displaced pods —
/// `scheduling.NodeClaim` reduced to its chosen offering + bound pods.
#[derive(Debug, Clone, PartialEq)]
pub struct NewNodeClaim {
    pub offering: InstanceOffering,
    pub pods: Vec<String>,
}

/// Result of [`simulate_scheduling`] — `scheduling.Results`.
#[derive(Debug, Clone, Default)]
pub struct SimulationResult {
    /// True iff every reschedulable pod found a home — upstream
    /// `Results.AllNonPendingPodsScheduled()`.
    pub all_scheduled: bool,
    /// New nodes the simulation had to open — `Results.NewNodeClaims`.
    pub new_node_claims: Vec<NewNodeClaim>,
}

/// `disruption.Decision` — the three outcomes of a consolidation command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    NoOp,
    /// Remove the candidates outright; their pods fit the surviving cluster.
    Delete,
    /// Replace the candidates with a single cheaper node.
    Replace,
}

/// `disruption.Command` — the candidates to remove and the replacements to
/// launch. `Decision()` is derived exactly as upstream from the two lengths.
#[derive(Debug, Clone, Default)]
pub struct Command {
    candidates: Vec<ConsolidationCandidate>,
    replacements: Vec<NewNodeClaim>,
}

impl Command {
    pub fn candidates(&self) -> &[ConsolidationCandidate] {
        &self.candidates
    }

    pub fn replacements(&self) -> &[NewNodeClaim] {
        &self.replacements
    }

    /// `func (c Command) Decision() Decision` — types.go.
    pub fn decision(&self) -> Decision {
        match (self.candidates.is_empty(), self.replacements.is_empty()) {
            (false, false) => Decision::Replace,
            (false, true) => Decision::Delete,
            _ => Decision::NoOp,
        }
    }
}

/// Pack `pods` onto the cheapest launchable offerings, first-fit-decreasing.
/// Returns `None` if any pod fits no offering at all (cannot be rescheduled).
/// Mirrors the new-node half of the scheduler's solve loop, choosing the
/// cheapest offering when a fresh node must be opened (consolidation always
/// drives toward the lowest-price option).
fn pack_new_nodes(pods: &[PodSpec], offerings: &[InstanceOffering]) -> Option<Vec<NewNodeClaim>> {
    if pods.is_empty() {
        return Some(Vec::new());
    }
    // Cheapest offerings first — consolidation minimises price.
    let mut sorted_offerings: Vec<&InstanceOffering> = offerings.iter().collect();
    sorted_offerings.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

    // Largest pods first — first-fit-decreasing.
    let mut sorted_pods: Vec<&PodSpec> = pods.iter().collect();
    sorted_pods.sort_by_key(|p| std::cmp::Reverse(p.cpu_millis));

    struct OpenNode {
        offering: InstanceOffering,
        free_cpu: u32,
        free_mem: u32,
        pods: Vec<String>,
    }
    let mut nodes: Vec<OpenNode> = Vec::new();

    for pod in sorted_pods {
        // Try an already-opened node first.
        if let Some(n) = nodes
            .iter_mut()
            .find(|n| n.free_cpu >= pod.cpu_millis && n.free_mem >= pod.memory_mib)
        {
            n.free_cpu -= pod.cpu_millis;
            n.free_mem -= pod.memory_mib;
            n.pods.push(pod.name.clone());
            continue;
        }
        // Open the cheapest offering that fits this pod.
        let off = sorted_offerings
            .iter()
            .find(|o| o.cpu_millis >= pod.cpu_millis && o.memory_mib >= pod.memory_mib)?;
        nodes.push(OpenNode {
            offering: (*off).clone(),
            free_cpu: off.cpu_millis - pod.cpu_millis,
            free_mem: off.memory_mib - pod.memory_mib,
            pods: vec![pod.name.clone()],
        });
    }

    Some(
        nodes
            .into_iter()
            .map(|n| NewNodeClaim {
                offering: n.offering,
                pods: n.pods,
            })
            .collect(),
    )
}

/// Simulate rescheduling `pods` after the candidate nodes are removed:
/// first absorb pods onto the surviving cluster's free capacity (in-flight
/// reservation), then open new nodes for the remainder. Mirrors
/// `SimulateScheduling` — returns whether everything scheduled and the new
/// nodes that had to be launched.
pub fn simulate_scheduling(
    pods: &[PodSpec],
    remaining: &[NodeCapacity],
    offerings: &[InstanceOffering],
) -> SimulationResult {
    // Largest pods first.
    let mut sorted_pods: Vec<&PodSpec> = pods.iter().collect();
    sorted_pods.sort_by_key(|p| std::cmp::Reverse(p.cpu_millis));

    // Mutable view of surviving free capacity.
    let mut free: Vec<NodeCapacity> = remaining.to_vec();
    let mut displaced: Vec<PodSpec> = Vec::new();

    for pod in sorted_pods {
        if let Some(cap) = free
            .iter_mut()
            .find(|c| c.free_cpu_millis >= pod.cpu_millis && c.free_memory_mib >= pod.memory_mib)
        {
            cap.free_cpu_millis -= pod.cpu_millis;
            cap.free_memory_mib -= pod.memory_mib;
        } else {
            displaced.push(pod.clone());
        }
    }

    match pack_new_nodes(&displaced, offerings) {
        Some(new_node_claims) => SimulationResult {
            all_scheduled: true,
            new_node_claims,
        },
        None => SimulationResult {
            all_scheduled: false,
            new_node_claims: Vec::new(),
        },
    }
}

/// `computeConsolidation` — compute the consolidation action for a batch of
/// candidates. Delete if their pods fit the surviving cluster with no new
/// node; Replace if they fit on exactly one node cheaper than the combined
/// candidate price; NoOp otherwise (including the "would create multiple
/// nodes" guard — we never turn one node into many).
pub fn compute_consolidation(
    candidates: &[ConsolidationCandidate],
    remaining: &[NodeCapacity],
    offerings: &[InstanceOffering],
) -> Command {
    let pods: Vec<PodSpec> = candidates
        .iter()
        .flat_map(|c| c.reschedulable_pods.iter().cloned())
        .collect();

    let results = simulate_scheduling(&pods, remaining, offerings);

    // if not all of the pods were scheduled, we can't do anything
    if !results.all_scheduled {
        return Command::default();
    }
    // were we able to schedule all the pods on the inflight candidates?
    if results.new_node_claims.is_empty() {
        return Command {
            candidates: candidates.to_vec(),
            replacements: Vec::new(),
        };
    }
    // we're not going to turn a single node into multiple candidates
    if results.new_node_claims.len() != 1 {
        return Command::default();
    }
    // the replacement must be cheaper than the combined price of the nodes
    // being removed — `RemoveInstanceTypeOptionsByPriceAndMinValues`.
    let candidate_price: f64 = candidates.iter().map(|c| c.price).sum();
    if results.new_node_claims[0].offering.price < candidate_price {
        Command {
            candidates: candidates.to_vec(),
            replacements: results.new_node_claims,
        }
    } else {
        Command::default()
    }
}

/// Filter candidates against the per-NodePool disruption budget, decrementing
/// as it goes, and drop candidates with no reschedulable pods (empty nodes
/// are handled by the emptiness path). Mirrors the budget pre-filter shared
/// by both consolidation methods.
fn disruptable_candidates(
    candidates: &[ConsolidationCandidate],
    budgets: &mut HashMap<String, i32>,
) -> Vec<ConsolidationCandidate> {
    let mut out = Vec::new();
    for c in candidates {
        let allowed = budgets.get(&c.nodepool).copied().unwrap_or(0);
        if allowed <= 0 {
            continue;
        }
        if c.reschedulable_pods.is_empty() {
            continue;
        }
        out.push(c.clone());
        *budgets.entry(c.nodepool.clone()).or_insert(0) -= 1;
    }
    out
}

/// Sort candidates by ascending disruption cost — `sortCandidates`.
fn sort_candidates(mut candidates: Vec<ConsolidationCandidate>) -> Vec<ConsolidationCandidate> {
    candidates.sort_by(|a, b| {
        a.disruption_cost
            .partial_cmp(&b.disruption_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

/// `MultiNodeConsolidation.ComputeCommand` + `firstNConsolidationOption`.
///
/// Sort candidates by disruption cost, drop budget-blocked + empty nodes,
/// then binary-search the largest prefix `[0, mid]` that can be consolidated
/// at once. The search climbs (`min = mid + 1`) only when the prefix yields a
/// Delete or a valid Replace, so it returns the largest consolidatable batch.
pub fn multi_node_consolidation(
    candidates: &[ConsolidationCandidate],
    mut budgets: HashMap<String, i32>,
    remaining: &[NodeCapacity],
    offerings: &[InstanceOffering],
) -> Command {
    let sorted = sort_candidates(candidates.to_vec());
    let disruptable = disruptable_candidates(&sorted, &mut budgets);
    // Only consider a maximum batch of 100 NodeClaims (upstream lo.Clamp).
    let max_parallel = disruptable.len().min(100);
    first_n_consolidation_option(&disruptable, max_parallel, remaining, offerings)
}

fn first_n_consolidation_option(
    candidates: &[ConsolidationCandidate],
    max: usize,
    remaining: &[NodeCapacity],
    offerings: &[InstanceOffering],
) -> Command {
    // we always operate on at least two NodeClaims at once
    if candidates.len() < 2 {
        return Command::default();
    }
    let mut min: i64 = 1;
    let mut max: i64 = if candidates.len() <= max {
        candidates.len() as i64 - 1
    } else {
        max as i64
    };

    let mut last_saved = Command::default();
    while min <= max {
        let mid = (min + max) / 2;
        let batch = &candidates[0..(mid as usize + 1)];
        let cmd = compute_consolidation(batch, remaining, offerings);
        match cmd.decision() {
            // We can consolidate NodeClaims [0, mid] — climb higher.
            Decision::Delete | Decision::Replace => {
                last_saved = cmd;
                min = mid + 1;
            }
            Decision::NoOp => {
                max = mid - 1;
            }
        }
    }
    last_saved
}

/// `SingleNodeConsolidation.ComputeCommand` — walk candidates in disruption
/// order and return the first non-NoOp command. Single-node commands only
/// ever carry one candidate, so no budget counter is decremented; a zero
/// budget simply skips the candidate.
pub fn single_node_consolidation(
    candidates: &[ConsolidationCandidate],
    budgets: HashMap<String, i32>,
    remaining: &[NodeCapacity],
    offerings: &[InstanceOffering],
) -> Command {
    let sorted = sort_candidates(candidates.to_vec());
    for candidate in &sorted {
        if budgets.get(&candidate.nodepool).copied().unwrap_or(0) == 0 {
            continue;
        }
        // Filter out empty candidates (handled by the emptiness path).
        if candidate.reschedulable_pods.is_empty() {
            continue;
        }
        let cmd = compute_consolidation(std::slice::from_ref(candidate), remaining, offerings);
        if cmd.decision() != Decision::NoOp {
            return cmd;
        }
    }
    Command::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, cpu: u32, mem: u32) -> PodSpec {
        PodSpec::with_resources(name, cpu, mem)
    }

    #[test]
    fn pack_new_nodes_uses_cheapest_fitting_offering() {
        let pods = vec![pod("a", 500, 512)];
        let offerings = vec![
            InstanceOffering { name: "pricey".into(), cpu_millis: 4000, memory_mib: 8192, zone: "z".into(), price: 9.0 },
            InstanceOffering { name: "cheap".into(), cpu_millis: 1000, memory_mib: 1024, zone: "z".into(), price: 2.0 },
        ];
        let claims = pack_new_nodes(&pods, &offerings).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].offering.name, "cheap");
    }

    #[test]
    fn pack_new_nodes_none_when_pod_fits_nothing() {
        let pods = vec![pod("huge", 9000, 9000)];
        let offerings = vec![InstanceOffering { name: "s".into(), cpu_millis: 1000, memory_mib: 1024, zone: "z".into(), price: 1.0 }];
        assert!(pack_new_nodes(&pods, &offerings).is_none());
    }

    #[test]
    fn empty_command_is_noop() {
        assert_eq!(Command::default().decision(), Decision::NoOp);
    }
}
