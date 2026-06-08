// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodePool provisioning controller — faithful port of
//! kubernetes-sigs/karpenter v1.4.0 pkg/controllers/provisioning/provisioner.go
//!   Provisioner.Reconcile : batch pods → Schedule → CreateNodeClaims
//!   Provisioner.Schedule  : first-match NodePool per pod, binpack into claims
//!   Provisioner.CreateNodeClaims : launch each claim through the CloudProvider
//!
//! This is the workload-aware path: pending pods route to the NodePool whose
//! requirements they satisfy, get binpacked onto candidate instance types,
//! and each resulting NodeClaim is launched on the cloud provider.

use cave_karpenter::batcher::PodSpec;
use cave_karpenter::binpack::InstanceType;
use cave_karpenter::models::{NodePool, Requirement, RequirementOperator, Taint};
use cave_karpenter::provider::StaticProvider;
use cave_karpenter::provisioning::{PendingPod, reconcile, schedule};

const IT_LABEL: &str = "node.kubernetes.io/instance-type";

fn pool_in(name: &str, key: &str, vals: &[&str]) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.to_string();
    p.template.spec.requirements.push(Requirement {
        key: key.to_string(),
        operator: RequirementOperator::In,
        values: vals.iter().map(|s| s.to_string()).collect(),
        min_values: None,
    });
    p
}

fn open_pool(name: &str) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.to_string();
    p
}

fn inst(name: &str, cpu: u32, mem: u32) -> InstanceType {
    InstanceType {
        name: name.into(),
        cpu_millis: cpu,
        memory_mib: mem,
        zone: "z1".into(),
    }
}

fn pending(name: &str, cpu: u32, mem: u32, selector: &[(&str, &str)]) -> PendingPod {
    PendingPod {
        pod: PodSpec::with_resources(name, cpu, mem),
        node_selector: selector
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn schedule_provisions_a_claim_for_a_pending_pod() {
    let pools = vec![open_pool("default")];
    let pods = vec![pending("p", 500, 512, &[])];
    let instances = vec![inst("m5.large", 2000, 8192)];
    let res = schedule(&pools, &pods, &instances);
    assert_eq!(res.claims.len(), 1);
    assert_eq!(res.claims[0].pool, "default");
    assert!(res.unschedulable.is_empty());
}

#[test]
fn binpacking_packs_two_pods_onto_one_node() {
    let pools = vec![open_pool("default")];
    let pods = vec![
        pending("a", 400, 256, &[]),
        pending("b", 400, 256, &[]),
    ];
    let instances = vec![inst("big", 2000, 4096)];
    let res = schedule(&pools, &pods, &instances);
    assert_eq!(res.claims.len(), 1, "both pods fit one node");
    assert_eq!(res.claims[0].pods.len(), 2);
}

#[test]
fn binpacking_opens_two_nodes_when_one_is_too_small() {
    let pools = vec![open_pool("default")];
    let pods = vec![
        pending("a", 800, 512, &[]),
        pending("b", 800, 512, &[]),
    ];
    // Each instance holds only one of the two pods.
    let instances = vec![inst("small", 1000, 1024)];
    let res = schedule(&pools, &pods, &instances);
    assert_eq!(res.claims.len(), 2, "two nodes required");
}

#[test]
fn pod_without_a_matching_pool_is_unschedulable() {
    // Pool only accepts instance-type=gpu; the pod asks for cpu.
    let pools = vec![pool_in("gpu", IT_LABEL, &["g4dn.xlarge"])];
    let pods = vec![pending("p", 100, 128, &[(IT_LABEL, "m5.large")])];
    let instances = vec![inst("m5.large", 2000, 4096)];
    let res = schedule(&pools, &pods, &instances);
    assert!(res.claims.is_empty());
    assert_eq!(res.unschedulable, vec!["p".to_string()]);
}

#[test]
fn workload_aware_routing_picks_the_satisfying_pool() {
    // Two pools; the pod's nodeSelector matches only the gpu pool.
    let pools = vec![
        pool_in("gpu", IT_LABEL, &["g4dn.xlarge"]),
        open_pool("default"),
    ];
    let pods = vec![pending("ml", 500, 512, &[(IT_LABEL, "g4dn.xlarge")])];
    let instances = vec![inst("g4dn.xlarge", 4000, 16384)];
    let res = schedule(&pools, &pods, &instances);
    assert_eq!(res.claims.len(), 1);
    assert_eq!(res.claims[0].pool, "gpu");
}

#[test]
fn schedule_honours_pool_taints_in_binpacking() {
    // Pool carries a NoSchedule taint the pod does not tolerate → no fit.
    let mut p = open_pool("tainted");
    p.template.spec.taints.push(Taint {
        key: "dedicated".into(),
        value: None,
        effect: "NoSchedule".into(),
    });
    let pods = vec![pending("p", 100, 128, &[])];
    let instances = vec![inst("m5.large", 2000, 4096)];
    let res = schedule(&[p], &pods, &instances);
    // No instance accepted the pod under the taint → reported unschedulable.
    assert!(res.claims.is_empty());
    assert_eq!(res.unschedulable, vec!["p".to_string()]);
}

#[test]
fn empty_pending_pods_yields_no_claims() {
    let res = schedule(&[open_pool("default")], &[], &[inst("m5.large", 2000, 4096)]);
    assert!(res.claims.is_empty());
    assert!(res.unschedulable.is_empty());
}

#[test]
fn reconcile_launches_every_claim_through_the_provider() {
    let pools = vec![open_pool("default")];
    let pods = vec![
        pending("a", 800, 512, &[]),
        pending("b", 800, 512, &[]),
    ];
    let instances = vec![inst("small", 1000, 1024)];
    let provider = StaticProvider::new();
    let (res, launched) = reconcile(&pools, &pods, &instances, &provider);
    assert_eq!(res.claims.len(), 2);
    assert_eq!(launched.len(), 2, "both claims launched");
    // Every launched claim has a provider_id + node_name populated.
    for sc in &res.claims {
        let status = sc.claim.status.as_ref().expect("status filled by launch");
        assert!(status.provider_id.is_some());
        assert!(status.node_name.is_some());
    }
}
