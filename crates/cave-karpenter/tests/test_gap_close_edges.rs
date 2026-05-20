// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close edge tests for cave-karpenter.
//!
//! Targets the three modules without `#[cfg(test)]` blocks
//! (`store.rs`, `scheduler.rs`, `models/mod.rs`) plus boundary
//! conditions across the Phase 2 deep-port modules
//! (binpack / disruption / lifecycle / batcher / provider).
//!
//! Design philosophy: failure modes, boundary values, state
//! transitions, and serde round-trips. Each test is hermetic and
//! deterministic — no real time / IO outside of the batcher
//! window tests already covered upstream.

use cave_karpenter::batcher::{Batcher, PodSpec};
use cave_karpenter::binpack::{BinpackResult, InstanceType, binpack};
use cave_karpenter::disruption::{
    Decision, DisruptionReason, consolidation_candidates, drift_candidates, expiration_candidates,
    parse_duration,
};
use cave_karpenter::nodeclaim_lifecycle::{LaunchOutcome, drain, ensure_status, launch, terminate};
use cave_karpenter::provider::{
    AzureNodeClassSpec, CloudProvider, HetznerNodeClassSpec, ProviderError, StaticProvider,
};
use cave_karpenter::{
    Budget, Disruption, Limits, NodeClaim, NodeClass, NodeClassRef, NodePool, Requirement,
    RequirementOperator, ScheduleOutcome, Store, Taint, schedule_first_match,
};

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

// ----------------------------------------------------------------------------
// store.rs — covers claim + class CRUD paths not exercised inline
// ----------------------------------------------------------------------------

#[test]
fn store_get_pool_missing_returns_none() {
    let s = Store::new();
    assert!(s.get_pool("ghost").is_none());
}

#[test]
fn store_delete_missing_pool_returns_false() {
    let s = Store::new();
    assert!(!s.delete_pool("ghost"));
}

#[test]
fn store_put_pool_overwrites_same_name() {
    let s = Store::new();
    let mut p1 = NodePool::default();
    p1.name = "x".into();
    p1.weight = Some(10);
    s.put_pool(p1);
    let mut p2 = NodePool::default();
    p2.name = "x".into();
    p2.weight = Some(99);
    s.put_pool(p2);
    assert_eq!(s.list_pools().len(), 1);
    assert_eq!(s.get_pool("x").unwrap().weight, Some(99));
}

#[test]
fn store_claims_roundtrip_and_list() {
    let s = Store::new();
    let mut c1 = NodeClaim::default();
    c1.name = "a".into();
    let mut c2 = NodeClaim::default();
    c2.name = "b".into();
    s.put_claim(c1);
    s.put_claim(c2);
    let claims = s.list_claims();
    assert_eq!(claims.len(), 2);
    assert!(claims.iter().any(|c| c.name == "a"));
    assert!(claims.iter().any(|c| c.name == "b"));
}

#[test]
fn store_classes_keyed_by_kind_and_name() {
    let s = Store::new();
    let c1 = NodeClass {
        group: "karpenter.sh".into(),
        kind: "HetznerNodeClass".into(),
        name: "default".into(),
        spec: serde_json::json!({}),
    };
    // Same name, different kind — must not collide.
    let c2 = NodeClass {
        group: "karpenter.sh".into(),
        kind: "AzureNodeClass".into(),
        name: "default".into(),
        spec: serde_json::json!({}),
    };
    s.put_class(c1);
    s.put_class(c2);
    assert_eq!(s.list_classes().len(), 2);
}

// ----------------------------------------------------------------------------
// scheduler.rs — branch coverage for Exists / DoesNotExist / Gt / Lt
// ----------------------------------------------------------------------------

fn req(key: &str, op: RequirementOperator, vals: &[&str]) -> Requirement {
    Requirement {
        key: key.to_string(),
        operator: op,
        values: vals.iter().map(|s| s.to_string()).collect(),
        min_values: None,
    }
}

fn pool_with_req(name: &str, r: Requirement) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.into();
    p.template.spec.requirements.push(r);
    p
}

#[test]
fn schedule_exists_operator_always_matches() {
    let p = pool_with_req("any", req("region", RequirementOperator::Exists, &[]));
    let outcome = schedule_first_match(&[p], &[("region".into(), "us-west-1".into())]);
    assert!(matches!(outcome, ScheduleOutcome::Provisioned { .. }));
}

#[test]
fn schedule_does_not_exist_operator_always_blocks() {
    let p = pool_with_req("blocked", req("region", RequirementOperator::DoesNotExist, &[]));
    let outcome = schedule_first_match(&[p], &[("region".into(), "us-west-1".into())]);
    assert!(matches!(outcome, ScheduleOutcome::NoMatch { .. }));
}

#[test]
fn schedule_gt_lt_numeric_thresholds() {
    let gt = pool_with_req("gt", req("cpu", RequirementOperator::Gt, &["4"]));
    // value 8 > threshold 4 → match
    let ok = schedule_first_match(&[gt.clone()], &[("cpu".into(), "8".into())]);
    assert!(matches!(ok, ScheduleOutcome::Provisioned { .. }));
    // value 2 > threshold 4 → no match
    let no = schedule_first_match(&[gt], &[("cpu".into(), "2".into())]);
    assert!(matches!(no, ScheduleOutcome::NoMatch { .. }));

    let lt = pool_with_req("lt", req("cpu", RequirementOperator::Lt, &["4"]));
    let ok = schedule_first_match(&[lt.clone()], &[("cpu".into(), "1".into())]);
    assert!(matches!(ok, ScheduleOutcome::Provisioned { .. }));
    let no = schedule_first_match(&[lt], &[("cpu".into(), "9".into())]);
    assert!(matches!(no, ScheduleOutcome::NoMatch { .. }));
}

#[test]
fn schedule_gt_non_numeric_value_does_not_match() {
    let p = pool_with_req("gt", req("cpu", RequirementOperator::Gt, &["4"]));
    let outcome = schedule_first_match(&[p], &[("cpu".into(), "not-a-number".into())]);
    assert!(matches!(outcome, ScheduleOutcome::NoMatch { .. }));
}

#[test]
fn schedule_permissive_when_pool_has_no_requirement_for_key() {
    // Pool requires zone=us-east, pod asks for unrelated `arch=arm64`.
    let p = pool_with_req(
        "zonepool",
        req("zone", RequirementOperator::In, &["us-east-1a"]),
    );
    let outcome = schedule_first_match(&[p], &[("arch".into(), "arm64".into())]);
    assert!(matches!(outcome, ScheduleOutcome::Provisioned { .. }));
}

#[test]
fn schedule_claim_carries_pool_template_envelope() {
    let mut p = NodePool::default();
    p.name = "envelope".into();
    p.template_hash = Some("h1".into());
    p.template.spec.taints.push(Taint {
        key: "k".into(),
        value: Some("v".into()),
        effect: "NoSchedule".into(),
    });
    p.template.spec.expire_after = Some("1h".into());
    match schedule_first_match(&[p], &[]) {
        ScheduleOutcome::Provisioned { claim, .. } => {
            assert_eq!(claim.template_hash.as_deref(), Some("h1"));
            assert_eq!(claim.spec.taints.len(), 1);
            assert_eq!(claim.spec.expire_after.as_deref(), Some("1h"));
            assert_eq!(claim.pool_name.as_deref(), Some("envelope"));
        }
        _ => panic!("expected provision"),
    }
}

// ----------------------------------------------------------------------------
// models/mod.rs — serde round-trip for the CRD shapes
// ----------------------------------------------------------------------------

#[test]
fn nodepool_serde_roundtrip_preserves_full_envelope() {
    let mut np = NodePool::default();
    np.name = "default".into();
    np.namespace = Some("kube-system".into());
    np.weight = Some(10);
    np.template_hash = Some("abc123".into());
    np.template.labels.insert("tier".into(), "frontend".into());
    np.template.annotations.insert("note".into(), "test".into());
    np.template.spec.requirements.push(Requirement {
        key: "instance-type".into(),
        operator: RequirementOperator::In,
        values: vec!["m5.large".into()],
        min_values: Some(2),
    });
    np.template.spec.node_class_ref = Some(NodeClassRef {
        group: "karpenter.sh".into(),
        kind: "HetznerNodeClass".into(),
        name: "primary".into(),
    });
    np.disruption = Some(Disruption {
        consolidation_policy: Some("WhenUnderutilized".into()),
        consolidate_after: Some("30s".into()),
        budgets: vec![Budget {
            nodes: "10%".into(),
            schedule: Some("0 0 * * *".into()),
            duration: Some("1h".into()),
            reasons: vec!["Underutilized".into()],
        }],
    });
    let mut res = BTreeMap::new();
    res.insert("cpu".into(), "100".into());
    np.limits = Some(Limits { resources: res });

    let json = serde_json::to_string(&np).unwrap();
    let back: NodePool = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "default");
    assert_eq!(back.weight, Some(10));
    assert_eq!(back.template_hash.as_deref(), Some("abc123"));
    assert_eq!(back.template.spec.requirements[0].min_values, Some(2));
    assert_eq!(back.disruption.as_ref().unwrap().budgets.len(), 1);
    assert_eq!(back.limits.as_ref().unwrap().resources.get("cpu").unwrap(), "100");
}

#[test]
fn requirement_operator_serde_all_variants() {
    for op in [
        RequirementOperator::In,
        RequirementOperator::NotIn,
        RequirementOperator::Exists,
        RequirementOperator::DoesNotExist,
        RequirementOperator::Gt,
        RequirementOperator::Lt,
    ] {
        let j = serde_json::to_value(op).unwrap();
        let back: RequirementOperator = serde_json::from_value(j).unwrap();
        assert_eq!(back, op);
    }
}

#[test]
fn nodeclaim_default_is_unterminated_and_undrained() {
    let c = NodeClaim::default();
    assert!(!c.terminated);
    assert!(!c.drained);
    assert!(c.status.is_none());
    assert!(c.created_at.is_none());
    assert_eq!(c.utilization, 0.0);
}

#[test]
fn nodeclass_envelope_carries_opaque_spec() {
    let nc = NodeClass {
        group: "karpenter.sh".into(),
        kind: "HetznerNodeClass".into(),
        name: "primary".into(),
        spec: serde_json::json!({"server_type": "cx32", "location": "hel1"}),
    };
    let s = serde_json::to_string(&nc).unwrap();
    let back: NodeClass = serde_json::from_str(&s).unwrap();
    assert_eq!(back.spec["server_type"], "cx32");
}

// ----------------------------------------------------------------------------
// binpack — boundary + topology spread + tolerated multi-pod taint
// ----------------------------------------------------------------------------

#[test]
fn binpack_exact_fit_consumes_full_capacity() {
    let inst = InstanceType {
        name: "i".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "z".into(),
    };
    let pods = vec![PodSpec::with_resources("p", 1000, 1024)];
    match binpack(&pods, &[inst], &[]) {
        BinpackResult::Assigned { instances } => {
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0].remaining_cpu_millis, 0);
            assert_eq!(instances[0].remaining_memory_mib, 0);
        }
        _ => panic!("expected fit"),
    }
}

#[test]
fn binpack_topology_spread_distributes_across_zones() {
    // Three large instances across z1/z2/z3; three spread-requesting pods.
    let mk = |zone: &str| InstanceType {
        name: format!("i-{zone}"),
        cpu_millis: 4000,
        memory_mib: 4096,
        zone: zone.into(),
    };
    let insts = vec![mk("z1"), mk("z2"), mk("z3")];
    let pods: Vec<_> = (0..3)
        .map(|i| PodSpec::with_resources(&format!("p{i}"), 1000, 1024).with_zone_spread("zone"))
        .collect();
    match binpack(&pods, &insts, &[]) {
        BinpackResult::Assigned { instances } => {
            // Three pods, three zones → one pod per zone (round-robin via least-loaded).
            let mut zones: Vec<_> = instances.iter().map(|a| a.instance.zone.clone()).collect();
            zones.sort();
            zones.dedup();
            assert_eq!(zones.len(), 3, "pods should span 3 zones, got {zones:?}");
        }
        _ => panic!("expected fit"),
    }
}

#[test]
fn binpack_blocked_by_pool_wide_untolerated_taint() {
    let inst = InstanceType {
        name: "gpu".into(),
        cpu_millis: 4000,
        memory_mib: 8192,
        zone: "z".into(),
    };
    // Two pods, neither tolerates the GPU taint.
    let pods = vec![
        PodSpec::with_resources("p1", 500, 512),
        PodSpec::with_resources("p2", 500, 512),
    ];
    let taints = vec![Taint {
        key: "nvidia.com/gpu".into(),
        value: None,
        effect: "NoSchedule".into(),
    }];
    assert!(matches!(
        binpack(&pods, &inst.clone().into_vec(), &taints),
        BinpackResult::NoFit { .. }
    ));
}

// Tiny helper trait to make the test above tidy without touching src/.
trait IntoVec {
    fn into_vec(self) -> Vec<InstanceType>;
}
impl IntoVec for InstanceType {
    fn into_vec(self) -> Vec<InstanceType> {
        vec![self]
    }
}

#[test]
fn binpack_picks_smallest_fitting_instance_type() {
    let big = InstanceType {
        name: "big".into(),
        cpu_millis: 8000,
        memory_mib: 16384,
        zone: "z".into(),
    };
    let small = InstanceType {
        name: "small".into(),
        cpu_millis: 1000,
        memory_mib: 1024,
        zone: "z".into(),
    };
    let pods = vec![PodSpec::with_resources("p", 500, 512)];
    match binpack(&pods, &[big, small], &[]) {
        BinpackResult::Assigned { instances } => {
            assert_eq!(instances.len(), 1);
            // Smallest-fit should pick "small".
            assert_eq!(instances[0].instance.name, "small");
        }
        _ => panic!("expected fit"),
    }
}

#[test]
fn binpack_first_fit_decreasing_orders_by_cpu() {
    // Instance only holds one big pod's worth of CPU.
    let inst = InstanceType {
        name: "i".into(),
        cpu_millis: 1500,
        memory_mib: 2048,
        zone: "z".into(),
    };
    // Two pods: big + small. Big should be scheduled first; small packs onto same.
    let pods = vec![
        PodSpec::with_resources("small", 100, 128),
        PodSpec::with_resources("big", 1400, 1024),
    ];
    match binpack(&pods, &[inst], &[]) {
        BinpackResult::Assigned { instances } => {
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0].pods.len(), 2);
            // Big should appear before small in the assignment order.
            assert_eq!(instances[0].pods[0], "big");
            assert_eq!(instances[0].pods[1], "small");
        }
        _ => panic!("expected fit"),
    }
}

// ----------------------------------------------------------------------------
// disruption — duration parse edges + budget arithmetic + state filters
// ----------------------------------------------------------------------------

#[test]
fn parse_duration_zero_value_ok() {
    assert_eq!(parse_duration("0s").unwrap(), Duration::ZERO);
    assert_eq!(parse_duration("0").unwrap(), Duration::ZERO);
}

#[test]
fn parse_duration_rejects_empty_string() {
    // Empty splits to ("", "") — `"".parse::<u64>()` fails.
    assert!(parse_duration("").is_err());
}

#[test]
fn parse_duration_implicit_seconds() {
    // Bare integer is treated as seconds.
    assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
}

#[test]
fn consolidation_skips_terminated_claims() {
    let mut alive = NodeClaim::default();
    alive.name = "alive".into();
    alive.utilization = 0.1;
    let mut dead = NodeClaim::default();
    dead.name = "dead".into();
    dead.utilization = 0.1;
    dead.terminated = true;
    let cands = consolidation_candidates(&[alive, dead], 0.5);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].claim_name, "alive");
}

#[test]
fn consolidation_threshold_boundary_inclusive() {
    // Utilization exactly at threshold should match (<=).
    let mut c = NodeClaim::default();
    c.name = "n".into();
    c.utilization = 0.5;
    assert_eq!(consolidation_candidates(&[c], 0.5).len(), 1);
}

#[test]
fn drift_skips_claim_with_no_pool_match() {
    let mut c = NodeClaim::default();
    c.name = "orphan".into();
    c.pool_name = Some("missing-pool".into());
    c.template_hash = Some("h1".into());
    let mut p = NodePool::default();
    p.name = "other-pool".into();
    p.template_hash = Some("h2".into());
    assert!(drift_candidates(&[c], &[p]).is_empty());
}

#[test]
fn drift_skips_claim_when_hashes_match() {
    let mut c = NodeClaim::default();
    c.name = "n".into();
    c.pool_name = Some("p".into());
    c.template_hash = Some("h".into());
    let mut p = NodePool::default();
    p.name = "p".into();
    p.template_hash = Some("h".into());
    assert!(drift_candidates(&[c], &[p]).is_empty());
}

#[test]
fn drift_flags_claim_with_stale_hash() {
    let mut c = NodeClaim::default();
    c.name = "stale".into();
    c.pool_name = Some("p".into());
    c.template_hash = Some("old".into());
    let mut p = NodePool::default();
    p.name = "p".into();
    p.template_hash = Some("new".into());
    let out = drift_candidates(&[c], &[p]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].reason, DisruptionReason::Drift);
}

#[test]
fn expiration_skips_claim_without_created_at() {
    let mut c = NodeClaim::default();
    c.name = "n".into();
    c.spec.expire_after = Some("1s".into());
    assert!(expiration_candidates(&[c], SystemTime::now()).is_empty());
}

#[test]
fn expiration_skips_claim_with_unparseable_duration() {
    let mut c = NodeClaim::default();
    c.name = "n".into();
    c.spec.expire_after = Some("1y".into()); // 'y' not supported
    c.created_at = Some(SystemTime::UNIX_EPOCH);
    assert!(expiration_candidates(&[c], SystemTime::now()).is_empty());
}

#[test]
fn expiration_flags_overdue_claim() {
    let mut c = NodeClaim::default();
    c.name = "old".into();
    c.spec.expire_after = Some("1s".into());
    c.created_at = Some(SystemTime::UNIX_EPOCH);
    let out = expiration_candidates(&[c], SystemTime::now());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].reason, DisruptionReason::Expiration);
}

#[test]
fn budget_integer_cap_limits_decisions() {
    let d = Disruption {
        consolidation_policy: None,
        consolidate_after: None,
        budgets: vec![Budget {
            nodes: "2".into(),
            schedule: None,
            duration: None,
            reasons: vec![],
        }],
    };
    let candidates: Vec<Decision> = (0..5)
        .map(|i| Decision {
            claim_name: format!("n{i}"),
            reason: DisruptionReason::Drift,
            message: String::new(),
        })
        .collect();
    let allowed = Decision::apply_budget(candidates, &d);
    assert_eq!(allowed.len(), 2);
}

#[test]
fn budget_zero_blocks_all_for_matching_reason() {
    let d = Disruption {
        consolidation_policy: None,
        consolidate_after: None,
        budgets: vec![Budget {
            nodes: "0".into(),
            schedule: None,
            duration: None,
            reasons: vec!["Drifted".into()],
        }],
    };
    let candidates = vec![Decision {
        claim_name: "n".into(),
        reason: DisruptionReason::Drift,
        message: String::new(),
    }];
    assert_eq!(Decision::apply_budget(candidates, &d).len(), 0);
}

#[test]
fn budget_empty_budgets_is_noop_passthrough() {
    let d = Disruption::default();
    let candidates = vec![Decision {
        claim_name: "n".into(),
        reason: DisruptionReason::Consolidation,
        message: String::new(),
    }];
    assert_eq!(Decision::apply_budget(candidates, &d).len(), 1);
}

// ----------------------------------------------------------------------------
// nodeclaim_lifecycle — state-transition matrix
// ----------------------------------------------------------------------------

#[test]
fn launch_uses_default_instance_when_no_hint() {
    let mut c = NodeClaim::default();
    c.name = "no-hint".into();
    let outcome = launch(&mut c, &StaticProvider::new()).unwrap();
    match outcome {
        LaunchOutcome::Launched { provider_id } => {
            assert!(provider_id.contains("default-instance"));
            assert!(provider_id.contains("default"));
        }
        _ => panic!("expected fresh launch"),
    }
    assert!(c.created_at.is_some());
    assert_eq!(c.status.as_ref().unwrap().node_name.as_deref(), Some("no-hint-node"));
}

#[test]
fn drain_sets_drained_flag() {
    let mut c = NodeClaim::default();
    drain(&mut c, Duration::from_secs(5)).unwrap();
    assert!(c.drained);
}

#[test]
fn terminate_drain_first_drains_before_delete() {
    let mut c = NodeClaim::default();
    c.name = "n".into();
    let p = StaticProvider::new();
    launch(&mut c, &p).unwrap();
    let id_before = c.status.clone().unwrap().provider_id.unwrap();
    assert!(p.exists(&id_before).unwrap());
    terminate(&mut c, &p, true).unwrap();
    assert!(c.drained);
    assert!(c.terminated);
    assert!(!p.exists(&id_before).unwrap());
    assert!(c.status.as_ref().unwrap().provider_id.is_none());
}

#[test]
fn terminate_is_idempotent_after_first_call() {
    let mut c = NodeClaim::default();
    c.name = "n".into();
    let p = StaticProvider::new();
    launch(&mut c, &p).unwrap();
    terminate(&mut c, &p, false).unwrap();
    // Second call is a no-op — must not error and must not panic on already-deleted id.
    terminate(&mut c, &p, false).unwrap();
    assert!(c.terminated);
}

#[test]
fn ensure_status_creates_default_when_absent() {
    let mut c = NodeClaim::default();
    assert!(c.status.is_none());
    let st = ensure_status(&mut c);
    assert!(st.provider_id.is_none());
    assert!(c.status.is_some());
    // Second call should return existing — not overwrite.
    c.status.as_mut().unwrap().provider_id = Some("kept".into());
    let st2 = ensure_status(&mut c);
    assert_eq!(st2.provider_id.as_deref(), Some("kept"));
}

// ----------------------------------------------------------------------------
// provider — error variants + Hetzner/Azure serde shape edges
// ----------------------------------------------------------------------------

#[test]
fn provider_error_display_messages() {
    let e1 = ProviderError::Unavailable("rate-limited".into());
    assert!(e1.to_string().contains("rate-limited"));
    let e2 = ProviderError::InvalidRequest("bad zone".into());
    assert!(e2.to_string().contains("bad zone"));
    let e3 = ProviderError::NotFound("ghost-id".into());
    assert!(e3.to_string().contains("ghost-id"));
}

#[test]
fn hetzner_spec_round_trips_through_json_value() {
    let s = HetznerNodeClassSpec {
        server_type: "cx32".into(),
        image: "ubuntu-24.04".into(),
        location: "fsn1".into(),
        ssh_keys: vec!["k1".into(), "k2".into()],
        networks: vec!["10.0.0.0/8".into()],
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["server_type"], "cx32");
    assert_eq!(v["ssh_keys"].as_array().unwrap().len(), 2);
    let back: HetznerNodeClassSpec = serde_json::from_value(v).unwrap();
    assert_eq!(back.location, "fsn1");
}

#[test]
fn azure_spec_handles_optional_subnet_and_disk() {
    let s = AzureNodeClassSpec {
        vm_size: "Standard_D8s_v5".into(),
        image_sku: "ubuntu-22.04".into(),
        location: "westeurope".into(),
        subnet_id: Some("subnet-abc".into()),
        os_disk_size_gb: None,
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["subnet_id"], "subnet-abc");
    assert!(v["os_disk_size_gb"].is_null());
    let back: AzureNodeClassSpec = serde_json::from_value(v).unwrap();
    assert_eq!(back.subnet_id.as_deref(), Some("subnet-abc"));
    assert!(back.os_disk_size_gb.is_none());
}

#[test]
fn static_provider_create_increments_counter_per_call() {
    let p = StaticProvider::new();
    let a = p.create("m5.large", "z1").unwrap();
    let b = p.create("m5.large", "z1").unwrap();
    assert_ne!(a, b, "consecutive creates must return distinct ids");
    assert!(p.exists(&a).unwrap());
    assert!(p.exists(&b).unwrap());
}

// ----------------------------------------------------------------------------
// batcher — dedup + idle-window semantics
// ----------------------------------------------------------------------------

#[test]
fn batcher_dedup_repeated_pod_name_in_window() {
    let mut b = Batcher::new(Duration::from_millis(100));
    b.enqueue(PodSpec::new("dup"));
    b.enqueue(PodSpec::new("dup"));
    b.enqueue(PodSpec::new("dup"));
    assert_eq!(b.pending_len(), 1);
}

#[test]
fn batcher_is_ready_false_when_no_enqueue() {
    let b = Batcher::new(Duration::from_millis(1));
    // Wait past the duration — without an enqueue, window_start stays None.
    std::thread::sleep(Duration::from_millis(5));
    assert!(!b.is_ready(std::time::Instant::now()));
}

#[test]
fn batcher_take_round_returns_all_pods_in_enqueue_order() {
    let mut b = Batcher::new(Duration::from_millis(1));
    b.enqueue(PodSpec::new("a"));
    b.enqueue(PodSpec::new("b"));
    b.enqueue(PodSpec::new("c"));
    let round = b.take_round();
    let names: Vec<_> = round.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    assert_eq!(b.pending_len(), 0);
}

#[test]
fn batcher_window_accessor_exposes_configured_duration() {
    let b = Batcher::new(Duration::from_secs(7));
    assert_eq!(b.window(), Duration::from_secs(7));
}
