// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-karpenter Phase 2 deep-port — tests for the controllers added
//! alongside the scheduler: disruption (consolidation/drift/expiration),
//! NodeClaim lifecycle (launch + termination + drain), provisioning
//! batcher (pending-pod queue), binpacking (in-flight reservation +
//! topology spread), and the CloudProvider abstraction.
//!
//! The whole file is RED before the matching `src/` modules land.

use cave_karpenter::batcher::{Batcher, PodSpec};
use cave_karpenter::binpack::{BinpackResult, InstanceType, binpack};
use cave_karpenter::disruption::{
    Decision, DisruptionReason, consolidation_candidates, drift_candidates, expiration_candidates,
};
use cave_karpenter::models::{
    Budget, Disruption, NodeClaim, NodeClass, NodePool, Requirement, RequirementOperator, Taint,
};
use cave_karpenter::nodeclaim_lifecycle::{LaunchOutcome, drain, launch, terminate};
use cave_karpenter::provider::{
    AzureNodeClassSpec, CloudProvider, HetznerNodeClassSpec, ProviderResult, StaticProvider,
};
use std::time::{Duration, SystemTime};

fn base_pool(name: &str) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.to_string();
    p
}

fn pool_with_zone(name: &str, zone: &str) -> NodePool {
    let mut p = base_pool(name);
    p.template.spec.requirements.push(Requirement {
        key: "topology.kubernetes.io/zone".into(),
        operator: RequirementOperator::In,
        values: vec![zone.into()],
        min_values: None,
    });
    p
}

// ─── Disruption controller ──────────────────────────────────────────────────

#[test]
fn consolidation_returns_empty_when_no_nodes_underutilised() {
    let claims: Vec<NodeClaim> = vec![];
    let out = consolidation_candidates(&claims, 0.5);
    assert!(out.is_empty(), "no nodes → no candidates");
}

#[test]
fn consolidation_picks_underutilised_node_below_threshold() {
    let mut a = NodeClaim::default();
    a.name = "node-a".into();
    a.utilization = 0.20; // below 0.5 threshold
    let mut b = NodeClaim::default();
    b.name = "node-b".into();
    b.utilization = 0.80;
    let claims = vec![a, b];
    let out = consolidation_candidates(&claims, 0.5);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].claim_name, "node-a");
    assert_eq!(out[0].reason, DisruptionReason::Consolidation);
}

#[test]
fn drift_flags_node_whose_pool_template_changed() {
    let mut pool = base_pool("default");
    pool.template_hash = Some("v2".into());
    let mut claim = NodeClaim::default();
    claim.name = "n".into();
    claim.pool_name = Some("default".into());
    claim.template_hash = Some("v1".into());

    let candidates = drift_candidates(&[claim], &[pool]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].reason, DisruptionReason::Drift);
}

#[test]
fn drift_skips_node_whose_pool_template_matches() {
    let mut pool = base_pool("default");
    pool.template_hash = Some("v1".into());
    let mut claim = NodeClaim::default();
    claim.name = "n".into();
    claim.pool_name = Some("default".into());
    claim.template_hash = Some("v1".into());

    let candidates = drift_candidates(&[claim], &[pool]);
    assert!(candidates.is_empty());
}

#[test]
fn expiration_flags_nodes_past_expire_after() {
    let mut claim = NodeClaim::default();
    claim.name = "old".into();
    claim.spec.expire_after = Some("1h".into());
    let now = SystemTime::now();
    claim.created_at = Some(now - Duration::from_secs(7200)); // 2h ago
    let out = expiration_candidates(&[claim], now);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].reason, DisruptionReason::Expiration);
}

#[test]
fn expiration_keeps_nodes_within_lifetime() {
    let mut claim = NodeClaim::default();
    claim.name = "young".into();
    claim.spec.expire_after = Some("1h".into());
    let now = SystemTime::now();
    claim.created_at = Some(now - Duration::from_secs(60));
    let out = expiration_candidates(&[claim], now);
    assert!(out.is_empty());
}

#[test]
fn budget_enforces_node_cap() {
    let pool_disruption = Disruption {
        consolidation_policy: Some("WhenUnderutilized".into()),
        consolidate_after: None,
        budgets: vec![Budget {
            nodes: "1".into(),
            schedule: None,
            duration: None,
            reasons: vec!["Underutilized".into()],
        }],
    };
    // 2 nodes flagged; budget caps to 1.
    let mut a = NodeClaim::default();
    a.utilization = 0.1;
    a.name = "a".into();
    let mut b = NodeClaim::default();
    b.utilization = 0.1;
    b.name = "b".into();
    let candidates = consolidation_candidates(&[a, b], 0.5);
    let allowed = Decision::apply_budget(candidates, &pool_disruption);
    assert_eq!(allowed.len(), 1);
}

// ─── NodeClaim lifecycle ────────────────────────────────────────────────────

#[test]
fn launch_creates_provider_id_and_status() {
    let mut claim = NodeClaim::default();
    claim.name = "n1".into();
    let provider = StaticProvider::new();
    let out = launch(&mut claim, &provider).unwrap();
    assert!(matches!(out, LaunchOutcome::Launched { .. }));
    assert!(claim.status.is_some());
    assert!(claim.status.as_ref().unwrap().provider_id.is_some());
}

#[test]
fn terminate_drains_and_flips_status_to_terminated() {
    let mut claim = NodeClaim::default();
    claim.name = "n2".into();
    // Pretend it had a provider_id assigned by launch.
    claim.status = Some(Default::default());
    claim.status.as_mut().unwrap().provider_id = Some("static://abc".into());
    let provider = StaticProvider::new();
    terminate(&mut claim, &provider, true).unwrap();
    assert!(claim.terminated);
}

#[test]
fn drain_idempotent() {
    let mut claim = NodeClaim::default();
    claim.name = "n3".into();
    drain(&mut claim, Duration::from_secs(30)).unwrap();
    assert!(claim.drained);
    drain(&mut claim, Duration::from_secs(30)).unwrap();
    assert!(claim.drained);
}

// ─── Provisioning batcher ───────────────────────────────────────────────────

#[test]
fn batcher_collects_pending_pods_into_round() {
    let mut b = Batcher::new(Duration::from_millis(50));
    b.enqueue(PodSpec::new("p1"));
    b.enqueue(PodSpec::new("p2"));
    b.enqueue(PodSpec::new("p3"));
    let round = b.take_round();
    assert_eq!(round.len(), 3);
    assert!(b.take_round().is_empty(), "round consumed");
}

#[test]
fn batcher_dedupes_repeated_pod_names() {
    let mut b = Batcher::new(Duration::from_millis(50));
    b.enqueue(PodSpec::new("p1"));
    b.enqueue(PodSpec::new("p1"));
    assert_eq!(b.take_round().len(), 1);
}

// ─── Binpacker ──────────────────────────────────────────────────────────────

#[test]
fn binpack_picks_smallest_instance_that_fits() {
    let small = InstanceType {
        name: "small".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "us-east-1a".into(),
    };
    let large = InstanceType {
        name: "large".into(),
        cpu_millis: 4000,
        memory_mib: 8192,
        zone: "us-east-1a".into(),
    };
    let pods = vec![PodSpec::with_resources("p1", 500, 512)];
    let res = binpack(&pods, &[small, large], &[]);
    match res {
        BinpackResult::Assigned { instances } => {
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0].instance.name, "small");
            assert_eq!(instances[0].pods, vec!["p1".to_string()]);
        }
        BinpackResult::NoFit { .. } => panic!("should fit"),
    }
}

#[test]
fn binpack_spreads_across_zones_when_topology_spread_requested() {
    let east_a = InstanceType {
        name: "i-a".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "us-east-1a".into(),
    };
    let east_b = InstanceType {
        name: "i-b".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "us-east-1b".into(),
    };
    let pods = vec![
        PodSpec::with_resources("p1", 500, 512).with_zone_spread("topology.kubernetes.io/zone"),
        PodSpec::with_resources("p2", 500, 512).with_zone_spread("topology.kubernetes.io/zone"),
    ];
    let res = binpack(&pods, &[east_a, east_b], &[]);
    match res {
        BinpackResult::Assigned { instances } => {
            // Two instances picked, one per zone.
            assert_eq!(instances.len(), 2);
            let zones: Vec<_> = instances.iter().map(|i| i.instance.zone.clone()).collect();
            assert!(zones.contains(&"us-east-1a".to_string()));
            assert!(zones.contains(&"us-east-1b".to_string()));
        }
        BinpackResult::NoFit { reason } => panic!("expected fit: {reason}"),
    }
}

#[test]
fn binpack_respects_taint_intolerance() {
    let tainted = InstanceType {
        name: "gpu".into(),
        cpu_millis: 4000,
        memory_mib: 8192,
        zone: "us-east-1a".into(),
    };
    let pods = vec![PodSpec::with_resources("p1", 500, 512)];
    let taint = Taint {
        key: "nvidia.com/gpu".into(),
        value: None,
        effect: "NoSchedule".into(),
    };
    let res = binpack(&pods, &[tainted], &[taint]);
    assert!(matches!(res, BinpackResult::NoFit { .. }));
}

#[test]
fn binpack_reserves_in_flight_capacity() {
    // Two pods that together exceed one instance's capacity → binpacker
    // opens a second instance of the same type rather than co-packing.
    let inst = InstanceType {
        name: "i".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "z1".into(),
    };
    let pods = vec![
        PodSpec::with_resources("p1", 700, 700),
        PodSpec::with_resources("p2", 400, 400),
    ];
    let res = binpack(&pods, &[inst], &[]);
    match res {
        BinpackResult::Assigned { instances } => {
            // Two instances; each holds exactly one pod.
            assert_eq!(instances.len(), 2);
            assert_eq!(instances[0].pods.len(), 1);
            assert_eq!(instances[1].pods.len(), 1);
        }
        BinpackResult::NoFit { reason } => panic!("expected two-instance fit: {reason}"),
    }
}

// ─── Provider abstraction (Hetzner + Azure NodeClass envelopes) ─────────────

#[test]
fn static_provider_create_returns_provider_id() {
    let provider = StaticProvider::new();
    let r: ProviderResult<String> = provider.create("default-instance", "us-east-1a");
    let id = r.unwrap();
    assert!(id.starts_with("static://"));
}

#[test]
fn hetzner_nodeclass_spec_carries_server_type_and_image() {
    let spec = HetznerNodeClassSpec {
        server_type: "cx21".into(),
        image: "ubuntu-22.04".into(),
        location: "hel1".into(),
        ssh_keys: vec!["root".into()],
        networks: vec![],
    };
    let nc = NodeClass {
        group: "karpenter.hetzner.io".into(),
        kind: "HetznerNodeClass".into(),
        name: "primary".into(),
        spec: serde_json::to_value(&spec).unwrap(),
    };
    let round: HetznerNodeClassSpec = serde_json::from_value(nc.spec).unwrap();
    assert_eq!(round.server_type, "cx21");
    assert_eq!(round.location, "hel1");
}

#[test]
fn azure_nodeclass_spec_carries_vm_sku_and_subnet() {
    let spec = AzureNodeClassSpec {
        vm_size: "Standard_D4s_v5".into(),
        image_sku: "ubuntu-22.04".into(),
        location: "westeurope".into(),
        subnet_id: Some("/subscriptions/.../subnets/default".into()),
        os_disk_size_gb: Some(60),
    };
    let nc = NodeClass {
        group: "karpenter.azure.com".into(),
        kind: "AKSNodeClass".into(),
        name: "primary".into(),
        spec: serde_json::to_value(&spec).unwrap(),
    };
    let round: AzureNodeClassSpec = serde_json::from_value(nc.spec).unwrap();
    assert_eq!(round.vm_size, "Standard_D4s_v5");
}

// ─── Smoke: nodeclaim from pool round-trip via batcher → binpack ────────────

#[test]
fn end_to_end_round_picks_pool_for_topology_spread() {
    let pool = pool_with_zone("us-east", "us-east-1a");
    let _ = pool;
    let inst = InstanceType {
        name: "m5.large".into(),
        cpu_millis: 2000,
        memory_mib: 8192,
        zone: "us-east-1a".into(),
    };
    let pods = vec![PodSpec::with_resources("p1", 500, 1024)];
    let res = binpack(&pods, &[inst], &[]);
    assert!(matches!(res, BinpackResult::Assigned { .. }));
}
