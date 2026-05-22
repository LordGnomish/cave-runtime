// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cross-controller crosscut tests for cave-cloud-controller-manager.
//!
//! Verifies behaviours that span multiple controller loops in upstream
//! `cmd/cloud-controller-manager`:
//!
//! * Node controller — provider-id parsing, label drift, address sync,
//!   taint state machine, instance lifecycle.
//! * Service controller — LB lifecycle phases, spec-drift detection,
//!   `loadBalancerClass` skipping.
//! * Route controller — CIDR family detection, planner, blackhole detection,
//!   collision detection.
//! * Route IPAM — CidrAllocator allocate/reserve/release.
//! * Admin surface — `CLOUD_CONTROLLERS`, `PROVIDERS`, parity report.

#![cfg(test)]

use crate::test_ctx;
use crate::types::TenantId;
use crate::{node_controller, route_controller, route_ipam, service_controller};

// ── Node controller — provider-id parsing ───────────────────────────────────

#[test]
fn node_provider_id_parses_canonical_scheme_and_id() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "getInstanceProviderID",
        "tenant-ccm-node-pid"
    );
    let parsed = node_controller::parse_provider_id("hcloud://1234");
    assert_eq!(parsed, Some(("hcloud", "1234")));
}

#[test]
fn node_provider_id_rejects_missing_scheme() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "getInstanceProviderID",
        "tenant-ccm-node-pid-bad"
    );
    assert!(node_controller::parse_provider_id("://1234").is_none());
    assert!(node_controller::parse_provider_id("hcloud://").is_none());
    assert!(node_controller::parse_provider_id("noscheme").is_none());
}

#[test]
fn node_provider_id_format_is_round_trip_stable() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "buildProviderID",
        "tenant-ccm-node-pid-rt"
    );
    let formatted = node_controller::format_provider_id("azure", "vm-9");
    assert_eq!(
        node_controller::parse_provider_id(&formatted),
        Some(("azure", "vm-9"))
    );
}

#[test]
fn node_provider_id_scheme_returns_just_scheme() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "getInstanceProviderID",
        "tenant-ccm-node-scheme"
    );
    assert_eq!(
        node_controller::provider_id_scheme("hcloud://x"),
        Some("hcloud")
    );
}

// ── Node controller — initialization & drift ────────────────────────────────

#[test]
fn node_is_initialised_only_when_all_topology_labels_set() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "syncNode",
        "tenant-ccm-node-init"
    );
    let mut node = node_controller::NodeView::fresh("worker-1");
    assert!(!node_controller::is_initialised(&node));
    node.provider_id = Some("hcloud://1".into());
    node.zone = Some("fsn1-dc14".into());
    node.region = Some("fsn1".into());
    node.instance_type = Some("cpx21".into());
    assert!(node_controller::is_initialised(&node));
}

#[test]
fn node_label_drift_counts_each_mismatched_field() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
        "labelDrift",
        "tenant-ccm-node-drift"
    );
    let node = node_controller::NodeView::fresh("worker-1");
    let facts = node_controller::CloudFacts::minimal("hcloud://1", "fsn1-dc14", "fsn1", "cpx21");
    // No labels set yet → all four fields drift.
    assert_eq!(node_controller::label_drift(&node, &facts), 4);
}

// ── Node controller — address canonicalisation ──────────────────────────────

#[test]
fn node_address_canonicalisation_dedupes_and_sorts_by_precedence() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
        "mergeNodeAddresses",
        "tenant-ccm-node-addr"
    );
    let input = vec![
        node_controller::NodeAddress::new(node_controller::NodeAddressType::Hostname, "host"),
        node_controller::NodeAddress::new(
            node_controller::NodeAddressType::ExternalIP,
            "203.0.113.5",
        ),
        node_controller::NodeAddress::new(node_controller::NodeAddressType::InternalIP, "10.0.0.7"),
        node_controller::NodeAddress::new(
            node_controller::NodeAddressType::ExternalIP,
            "203.0.113.5",
        ),
    ];
    let canon = node_controller::canonicalize_addresses(&input);
    assert_eq!(canon.len(), 3);
    // InternalIP first by precedence.
    assert_eq!(canon[0].kind, node_controller::NodeAddressType::InternalIP);
    assert_eq!(canon[1].kind, node_controller::NodeAddressType::ExternalIP);
    assert_eq!(canon[2].kind, node_controller::NodeAddressType::Hostname);
}

#[test]
fn node_address_diff_reports_added_and_removed() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
        "addressDiff",
        "tenant-ccm-node-addr-diff"
    );
    let have = vec![node_controller::NodeAddress::new(
        node_controller::NodeAddressType::InternalIP,
        "10.0.0.1",
    )];
    let want = vec![node_controller::NodeAddress::new(
        node_controller::NodeAddressType::InternalIP,
        "10.0.0.2",
    )];
    let (added, removed) = node_controller::address_diff(&have, &want);
    assert_eq!(added.len(), 1);
    assert_eq!(removed.len(), 1);
    assert_eq!(added[0].address, "10.0.0.2");
    assert_eq!(removed[0].address, "10.0.0.1");
}

#[test]
fn node_preferred_address_picks_internal_ip_first() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
        "getNodeAddressesFromNodeIP",
        "tenant-ccm-node-pref-addr"
    );
    let addrs = vec![
        node_controller::NodeAddress::new(node_controller::NodeAddressType::ExternalIP, "1.2.3.4"),
        node_controller::NodeAddress::new(node_controller::NodeAddressType::InternalIP, "10.0.0.1"),
    ];
    let pref = node_controller::preferred_address(&addrs).unwrap();
    assert_eq!(pref.kind, node_controller::NodeAddressType::InternalIP);
}

// ── Node controller — instance state ────────────────────────────────────────

#[test]
fn node_instance_state_terminated_requires_deletion() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
        "monitorNode",
        "tenant-ccm-node-term"
    );
    assert!(node_controller::InstanceState::Terminated.requires_deletion());
    assert!(node_controller::InstanceState::NotFound.requires_deletion());
    assert!(!node_controller::InstanceState::Running.requires_deletion());
    assert!(!node_controller::InstanceState::Shutdown.requires_deletion());
}

#[test]
fn node_instance_state_failure_taint_only_for_unhealthy_alive() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
        "monitorNode",
        "tenant-ccm-node-taint"
    );
    assert!(
        node_controller::InstanceState::Shutdown
            .failure_taint()
            .is_some()
    );
    assert!(
        node_controller::InstanceState::Unreachable
            .failure_taint()
            .is_some()
    );
    assert!(
        node_controller::InstanceState::Running
            .failure_taint()
            .is_none()
    );
    assert!(
        node_controller::InstanceState::Terminated
            .failure_taint()
            .is_none()
    );
}

// ── Service controller — LB phase machine ───────────────────────────────────

#[test]
fn service_lb_phase_skip_when_load_balancer_class_does_not_match() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "syncLoadBalancerIfNeeded",
        "tenant-ccm-svc-skip"
    );
    let mut spec = service_controller::ServiceSpec::http("web", "default");
    spec.load_balancer_class = Some("aws-nlb".into());
    let obs = service_controller::ServiceObservation {
        spec,
        status: service_controller::LoadBalancerStatus::empty(),
        deletion_pending: false,
        last_applied_spec: None,
        finalizer_present: false,
        legacy_external_ip: None,
    };
    assert_eq!(
        service_controller::next_phase(&obs, "hetzner-lb"),
        service_controller::LbPhase::Skip,
    );
}

#[test]
fn service_lb_phase_add_finalizer_before_first_ensure() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "syncLoadBalancerIfNeeded",
        "tenant-ccm-svc-finalizer"
    );
    let obs = service_controller::ServiceObservation {
        spec: service_controller::ServiceSpec::http("web", "default"),
        status: service_controller::LoadBalancerStatus::empty(),
        deletion_pending: false,
        last_applied_spec: None,
        finalizer_present: false,
        legacy_external_ip: None,
    };
    assert_eq!(
        service_controller::next_phase(&obs, "hetzner-lb"),
        service_controller::LbPhase::AddFinalizer,
    );
}

#[test]
fn service_lb_phase_ensure_when_no_status_published() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "syncLoadBalancerIfNeeded",
        "tenant-ccm-svc-ensure"
    );
    let obs = service_controller::ServiceObservation {
        spec: service_controller::ServiceSpec::http("web", "default"),
        status: service_controller::LoadBalancerStatus::empty(),
        deletion_pending: false,
        last_applied_spec: None,
        finalizer_present: true,
        legacy_external_ip: None,
    };
    assert_eq!(
        service_controller::next_phase(&obs, "hetzner-lb"),
        service_controller::LbPhase::Ensure,
    );
}

#[test]
fn service_lb_phase_delete_when_deletion_pending_and_published() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "syncLoadBalancerIfNeeded",
        "tenant-ccm-svc-delete"
    );
    let obs = service_controller::ServiceObservation {
        spec: service_controller::ServiceSpec::http("web", "default"),
        status: service_controller::LoadBalancerStatus::ip("203.0.113.50"),
        deletion_pending: true,
        last_applied_spec: None,
        finalizer_present: true,
        legacy_external_ip: None,
    };
    assert_eq!(
        service_controller::next_phase(&obs, "hetzner-lb"),
        service_controller::LbPhase::Delete,
    );
}

#[test]
fn service_lb_drift_detected_when_ports_change() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "processServiceUpdate",
        "tenant-ccm-svc-drift"
    );
    let mut current = service_controller::ServiceSpec::http("web", "default");
    let last = current.clone();
    current.ports.push(service_controller::ServicePort::tcp(
        "https", 443, 8443, 30443,
    ));
    assert!(service_controller::lb_spec_drifted(&current, &last));
}

#[test]
fn service_validate_rejects_empty_ports() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "validateServiceLBStatus",
        "tenant-ccm-svc-noports"
    );
    let mut spec = service_controller::ServiceSpec::http("web", "default");
    spec.ports.clear();
    assert!(spec.validate().is_err());
}

#[test]
fn service_validate_rejects_local_policy_without_health_check_port() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
        "validateServiceLBStatus",
        "tenant-ccm-svc-local"
    );
    let mut spec = service_controller::ServiceSpec::http("web", "default");
    spec.external_traffic_policy = service_controller::ExternalTrafficPolicy::Local;
    spec.health_check_node_port = None;
    assert!(spec.validate().is_err());
}

// ── Route controller — CIDR helpers ─────────────────────────────────────────

#[test]
fn route_cidr_family_detects_v4_and_v6() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/utils/net/ipnet.go",
        "IsIPv6CIDR",
        "tenant-ccm-route-fam"
    );
    assert_eq!(
        route_controller::cidr_family("10.0.0.0/24"),
        Some(route_controller::CidrFamily::V4)
    );
    assert_eq!(
        route_controller::cidr_family("2001:db8::/64"),
        Some(route_controller::CidrFamily::V6)
    );
    assert_eq!(route_controller::cidr_family("not a cidr"), None);
}

#[test]
fn route_split_by_family_partitions_correctly() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "perFamilyPlan",
        "tenant-ccm-route-split"
    );
    let desired = vec![
        route_controller::DesiredRoute {
            node_name: "n1".into(),
            pod_cidr: "10.0.1.0/24".into(),
        },
        route_controller::DesiredRoute {
            node_name: "n2".into(),
            pod_cidr: "2001:db8:1::/64".into(),
        },
        route_controller::DesiredRoute {
            node_name: "n3".into(),
            pod_cidr: "10.0.2.0/24".into(),
        },
    ];
    let (v4, v6) = route_controller::split_by_family(&desired);
    assert_eq!(v4.len(), 2);
    assert_eq!(v6.len(), 1);
}

#[test]
fn route_plan_is_empty_when_current_matches_desired() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "reconcile",
        "tenant-ccm-route-noop"
    );
    let desired = vec![route_controller::DesiredRoute {
        node_name: "n1".into(),
        pod_cidr: "10.0.1.0/24".into(),
    }];
    let current = vec!["cluster-x-n1".into()];
    let plan = route_controller::plan_routes("cluster-x", &desired, &current);
    assert!(plan.is_empty());
}

#[test]
fn route_plan_emits_create_for_missing_route() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "reconcile",
        "tenant-ccm-route-create"
    );
    let desired = vec![route_controller::DesiredRoute {
        node_name: "n1".into(),
        pod_cidr: "10.0.1.0/24".into(),
    }];
    let plan = route_controller::plan_routes("cluster-x", &desired, &[]);
    assert_eq!(plan.creates.len(), 1);
    assert_eq!(plan.deletes.len(), 0);
}

#[test]
fn route_plan_emits_delete_for_stale_route() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "reconcile",
        "tenant-ccm-route-delete"
    );
    let plan = route_controller::plan_routes("cluster-x", &[], &["cluster-x-orphan".into()]);
    assert_eq!(plan.creates.len(), 0);
    assert_eq!(plan.deletes.len(), 1);
    assert_eq!(plan.deletes[0], "cluster-x-orphan");
}

#[test]
fn route_blackhole_detection_finds_routes_with_no_live_node() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "findBlackholeRoutes",
        "tenant-ccm-route-blackhole"
    );
    let current = vec!["cluster-x-n1".into(), "cluster-x-orphan".into()];
    let live = vec!["cluster-x-n1".into()];
    let bh = route_controller::detect_blackhole(&current, &live);
    assert_eq!(bh, vec!["cluster-x-orphan".to_string()]);
}

#[test]
fn route_collision_detection_flags_duplicate_cidrs() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
        "validateClusterCIDR",
        "tenant-ccm-route-collide"
    );
    let desired = vec![
        route_controller::DesiredRoute {
            node_name: "n1".into(),
            pod_cidr: "10.0.1.0/24".into(),
        },
        route_controller::DesiredRoute {
            node_name: "n2".into(),
            pod_cidr: "10.0.1.0/24".into(),
        },
    ];
    let dupes = route_controller::detect_cidr_collisions(&desired);
    assert_eq!(dupes.len(), 1);
}

#[test]
fn route_name_for_uses_cluster_prefix() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/route/route_controller.go",
        "routeNameFor",
        "tenant-ccm-route-name"
    );
    assert_eq!(
        route_controller::route_name_for("clusterX", "node1"),
        "clusterX-node1"
    );
}

// ── Route IPAM — CidrAllocator ───────────────────────────────────────────────

#[test]
fn ipam_allocator_allocates_within_capacity() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
        "AllocateNext",
        "tenant-ccm-ipam-alloc"
    );
    let mut a = route_ipam::CidrAllocator::new("10.0.0.0/16", 24).unwrap();
    let cidr = a.allocate().unwrap();
    assert!(cidr.starts_with("10.0."));
    assert!(cidr.ends_with("/24"));
    assert_eq!(a.used(), 1);
}

#[test]
fn ipam_allocator_release_frees_capacity() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
        "Release",
        "tenant-ccm-ipam-release"
    );
    let mut a = route_ipam::CidrAllocator::new("10.0.0.0/16", 24).unwrap();
    let c = a.allocate().unwrap();
    a.release(&c).unwrap();
    assert_eq!(a.used(), 0);
}

#[test]
fn ipam_reserve_refuses_double_allocation() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
        "Reserve",
        "tenant-ccm-ipam-reserve"
    );
    let mut a = route_ipam::CidrAllocator::new("10.0.0.0/16", 24).unwrap();
    a.reserve("10.0.0.0/24").unwrap();
    assert!(a.reserve("10.0.0.0/24").is_err());
}

#[test]
fn ipam_v6_parent_returns_unimplemented() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/controllers/nodeipam/ipam/range_allocator.go",
        "newV6Allocator",
        "tenant-ccm-ipam-v6"
    );
    let r = route_ipam::CidrAllocator::new("2001:db8::/48", 64);
    assert!(r.is_err());
}

#[test]
fn ipam_backoff_schedule_default_is_valid() {
    let (_c, _t) = test_ctx!(
        "staging/src/k8s.io/client-go/util/retry/util.go",
        "DefaultBackoff",
        "tenant-ccm-ipam-backoff"
    );
    let s = route_ipam::BackoffSchedule::default_route();
    assert!(s.validate().is_ok());
}

// ── Admin surface (CLOUD_CONTROLLERS / PROVIDERS / parity) ──────────────────

#[test]
fn admin_cloud_controllers_includes_node_service_route_lifecycle() {
    let (_c, _t) = test_ctx!(
        "cmd/cloud-controller-manager/app/cloudcontrollermanager.go",
        "DefaultInitFuncConstructors",
        "tenant-ccm-admin-controllers"
    );
    for must in ["node", "node-lifecycle", "service", "route"] {
        assert!(crate::CLOUD_CONTROLLERS.contains(&must), "missing: {must}");
    }
}

#[test]
fn admin_providers_compiled_in_includes_hetzner_and_azure() {
    let (_c, _t) = test_ctx!(
        "providers/cloud_factory.go",
        "RegisterCloudProvider",
        "tenant-ccm-admin-providers"
    );
    assert!(crate::PROVIDERS.contains(&"hetzner"));
    assert!(crate::PROVIDERS.contains(&"azure"));
}

#[test]
fn admin_provider_snapshot_carries_version_and_count() {
    let (_c, _t) = test_ctx!(
        "cmd/cloud-controller-manager/app/cloudcontrollermanager.go",
        "Run",
        "tenant-ccm-admin-snapshot"
    );
    let v = crate::provider_snapshot();
    assert_eq!(v["controllers_active"], crate::CLOUD_CONTROLLERS.len());
    assert_eq!(v["upstream_version"], crate::UPSTREAM_VERSION);
}

#[test]
fn admin_parity_report_returns_module_metadata() {
    let (_c, _t) = test_ctx!(
        "cave-cloud-controller-manager/parity.manifest.toml",
        "calculate_parity",
        "tenant-ccm-admin-parity"
    );
    let report = crate::calculate_parity().expect("parity must succeed");
    assert_eq!(report.module, "cave-cloud-controller-manager");
    assert!(report.upstream_ref.contains("v1.36.0"));
}

#[test]
fn upstream_version_pinned_to_release() {
    let (_c, _t) = test_ctx!("version.go", "RELEASE", "tenant-ccm-pin-ver");
    assert!(crate::UPSTREAM_VERSION.starts_with('v'));
}

#[test]
fn admin_tenant_id_round_trips_through_serde() {
    let (_c, t) = test_ctx!(
        "staging/src/k8s.io/cloud-provider/types.go",
        "TenantId",
        "tenant-ccm-admin-tenant"
    );
    let json = serde_json::to_string(&t).unwrap();
    let back: TenantId = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}
