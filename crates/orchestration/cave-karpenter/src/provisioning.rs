// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodePool provisioning controller — workload-aware capacity creation.
//!
//! Faithful port of kubernetes-sigs/karpenter v1.4.0
//! pkg/controllers/provisioning/provisioner.go. The upstream `Provisioner`
//! is a singleton reconcile loop:
//!
//! ```text
//! Reconcile: batch pods → Schedule → if NewNodeClaims: CreateNodeClaims
//! Schedule:  GetPendingPods → first-match a NodePool per pod → binpack
//! CreateNodeClaims: launch each NodeClaim through the CloudProvider
//! ```
//!
//! The Cave port keeps the loop body pure. [`schedule`] routes each pending
//! pod to the first NodePool whose requirements it satisfies (the
//! workload-aware step — `cluster.schedule`), binpacks the matched pods onto
//! candidate instance types (honouring the pool's taints), and emits a
//! [`ScheduledNodeClaim`] per opened node. [`create_node_claims`] launches
//! each claim on the [`CloudProvider`]. [`reconcile`] is the end-to-end glue
//! a caller drives on its reconcile cadence.

use crate::batcher::PodSpec;
use crate::binpack::{BinpackResult, InstanceType, binpack};
use crate::models::{NodeClaim, NodeClaimStatus, NodePool};
use crate::provider::CloudProvider;
use crate::scheduler::pool_satisfies;

/// A pod awaiting capacity. `node_selector` is the set of `(label, value)`
/// constraints the pod requires (nodeSelector + required node affinity),
/// used to route it to a NodePool; `pod` carries the resource request the
/// binpacker reserves.
#[derive(Debug, Clone)]
pub struct PendingPod {
    pub pod: PodSpec,
    pub node_selector: Vec<(String, String)>,
}

/// One NodeClaim the scheduler decided to open, with the instance type the
/// binpacker chose and the pods bound to it. Mirrors `scheduling.NodeClaim`.
#[derive(Debug, Clone)]
pub struct ScheduledNodeClaim {
    pub pool: String,
    pub claim: NodeClaim,
    pub instance: InstanceType,
    pub pods: Vec<String>,
}

/// Outcome of [`schedule`] — `scheduling.Results`. `claims` are the nodes to
/// launch; `unschedulable` names the pods that matched no pool or did not fit.
#[derive(Debug, Clone, Default)]
pub struct ProvisioningResult {
    pub claims: Vec<ScheduledNodeClaim>,
    pub unschedulable: Vec<String>,
}

/// Build the NodeClaim template a NodePool would produce — same shape as
/// `scheduler::schedule_first_match`, with a sequence suffix so multiple
/// claims from one pool get distinct names.
fn claim_from_pool(pool: &NodePool, seq: usize) -> NodeClaim {
    NodeClaim {
        name: format!("{}-{}", pool.name, seq),
        namespace: pool.namespace.clone(),
        spec: pool.template.spec.clone(),
        status: None,
        pool_name: Some(pool.name.clone()),
        template_hash: pool.template_hash.clone(),
        utilization: 0.0,
        created_at: None,
        terminated: false,
        drained: false,
    }
}

/// Schedule pending pods onto NodePool capacity. Each pod is routed to the
/// first NodePool whose requirements it satisfies (`pool_satisfies`); the
/// matched pods are binpacked onto `instance_types` honouring the pool's
/// taints. Pods that match no pool — or whose pool can't binpack them — are
/// reported in `unschedulable`, preserving their original order.
pub fn schedule(
    pools: &[NodePool],
    pods: &[PendingPod],
    instance_types: &[InstanceType],
) -> ProvisioningResult {
    let mut result = ProvisioningResult::default();

    // Group matched pods by pool, preserving pool order and pod order.
    let mut buckets: Vec<(usize, Vec<PodSpec>)> = Vec::new();
    for pending in pods {
        match pools
            .iter()
            .position(|p| pool_satisfies(p, &pending.node_selector))
        {
            Some(pool_idx) => {
                if let Some(bucket) = buckets.iter_mut().find(|(i, _)| *i == pool_idx) {
                    bucket.1.push(pending.pod.clone());
                } else {
                    buckets.push((pool_idx, vec![pending.pod.clone()]));
                }
            }
            None => result.unschedulable.push(pending.pod.name.clone()),
        }
    }

    // Binpack each pool's pods into NodeClaims.
    for (pool_idx, bucket_pods) in buckets {
        let pool = &pools[pool_idx];
        match binpack(&bucket_pods, instance_types, &pool.template.spec.taints) {
            BinpackResult::Assigned { instances } => {
                for assignment in instances {
                    let seq = result.claims.len();
                    result.claims.push(ScheduledNodeClaim {
                        pool: pool.name.clone(),
                        claim: claim_from_pool(pool, seq),
                        instance: assignment.instance,
                        pods: assignment.pods,
                    });
                }
            }
            BinpackResult::NoFit { .. } => {
                // None of the pool's pods could be placed — report them all.
                for p in &bucket_pods {
                    result.unschedulable.push(p.name.clone());
                }
            }
        }
    }

    result
}

/// Launch every scheduled NodeClaim on the cloud provider, populating each
/// claim's `status.provider_id` + `node_name`. Mirrors
/// `Provisioner.CreateNodeClaims` → `Create`. Returns the provider IDs of the
/// successfully launched claims, in claim order.
pub fn create_node_claims<P: CloudProvider>(
    result: &mut ProvisioningResult,
    provider: &P,
) -> Vec<String> {
    let mut launched = Vec::with_capacity(result.claims.len());
    for sc in &mut result.claims {
        if let Ok(provider_id) = provider.create(&sc.instance.name, &sc.instance.zone) {
            let mut status = sc.claim.status.clone().unwrap_or_else(NodeClaimStatus::default);
            status.node_name = Some(format!("{}-node", sc.claim.name));
            status.provider_id = Some(provider_id.clone());
            sc.claim.status = Some(status);
            launched.push(provider_id);
        }
    }
    launched
}

/// End-to-end reconcile: [`schedule`] the pending pods, then
/// [`create_node_claims`] for the resulting NodeClaims. Returns the schedule
/// result (with launched-claim statuses filled in) and the launched provider
/// IDs. Mirrors `Provisioner.Reconcile`.
pub fn reconcile<P: CloudProvider>(
    pools: &[NodePool],
    pods: &[PendingPod],
    instance_types: &[InstanceType],
    provider: &P,
) -> (ProvisioningResult, Vec<String>) {
    let mut result = schedule(pools, pods, instance_types);
    let launched = create_node_claims(&mut result, provider);
    (result, launched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::StaticProvider;

    fn open_pool(name: &str) -> NodePool {
        let mut p = NodePool::default();
        p.name = name.to_string();
        p
    }

    #[test]
    fn claims_get_sequential_names() {
        let pools = vec![open_pool("default")];
        let pods = vec![
            PendingPod { pod: PodSpec::with_resources("a", 800, 512), node_selector: vec![] },
            PendingPod { pod: PodSpec::with_resources("b", 800, 512), node_selector: vec![] },
        ];
        let instances = vec![InstanceType { name: "small".into(), cpu_millis: 1000, memory_mib: 1024, zone: "z1".into() }];
        let res = schedule(&pools, &pods, &instances);
        assert_eq!(res.claims.len(), 2);
        assert_ne!(res.claims[0].claim.name, res.claims[1].claim.name);
    }

    #[test]
    fn create_node_claims_is_a_noop_for_empty_result() {
        let mut res = ProvisioningResult::default();
        let launched = create_node_claims(&mut res, &StaticProvider::new());
        assert!(launched.is_empty());
    }
}
