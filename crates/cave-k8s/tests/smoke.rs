// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Five end-to-end smoke scenarios:
//!
//!   1. Pod scheduling roundtrip (request → schedule → assign → start)
//!   2. Deployment rolling update (5 replicas, max_surge=2)
//!   3. Service ↔ EndpointSlice derivation across 6 pods
//!   4. RBAC deny path (alice has no role binding)
//!   5. Namespace quota enforce (Pod-count hard limit hits)

use cave_k8s::admission::{Chain, NamespaceLifecycle, Operation, PodSecurityRestricted, Request, ServiceAccountDefaulter};
use cave_k8s::authn::{ChainAuthenticator, X509ClientCertAuthenticator};
use cave_k8s::authz::{
    Attributes, Binding, ChainAuthorizer, PolicyRule as RbacRule, RbacAuthorizer, Role, Subject,
    SubjectKind, Verb,
};
use cave_k8s::cluster::{ClusterConfig, ControlPlane};
use cave_k8s::error::Error;
use cave_k8s::kubelet_facade::{drive_pod_action, LifecycleAction, PodAssignment, PodPhase};
use cave_k8s::models::ClusterPhase;
use cave_k8s::networking::{derive_slices, IpFamily, ServicePort};
use cave_k8s::quota::{Dimension, Quota, QuotaTracker};
use cave_k8s::scheduler_facade::{place, NodeCandidate, PlacementOutcome, PlacementRequest};
use cave_k8s::workloads::plan_rolling_update;

#[test]
fn smoke_1_pod_scheduling_roundtrip() {
    // ── 1. Bootstrap a cluster ─────────────────────────────────────────────
    let cp = ControlPlane::new(ClusterConfig::default());
    cp.start();
    assert_eq!(cp.phase(), ClusterPhase::Running);

    // ── 2. Three worker nodes with varying capacity ────────────────────────
    let nodes = vec![
        NodeCandidate {
            name: "n1".into(),
            cpu_allocatable_millis: 1000,
            memory_allocatable_bytes: 1 << 30,
            labels: Default::default(),
            taints: vec![],
        },
        NodeCandidate {
            name: "n2".into(),
            cpu_allocatable_millis: 4000,
            memory_allocatable_bytes: 8 << 30,
            labels: Default::default(),
            taints: vec![],
        },
        NodeCandidate {
            name: "n3".into(),
            cpu_allocatable_millis: 2000,
            memory_allocatable_bytes: 4 << 30,
            labels: Default::default(),
            taints: vec![],
        },
    ];

    // ── 3. Place a pod — scheduler picks the largest remaining capacity ───
    let outcome = place(
        &PlacementRequest {
            namespace: "default".into(),
            pod_name: "web-1".into(),
            scheduler_name: "default-scheduler".into(),
            cpu_request_millis: 250,
            memory_request_bytes: 256 * 1024 * 1024,
            node_selector: Default::default(),
            tolerations: vec![],
        },
        &nodes,
    );
    let node_name = match outcome {
        PlacementOutcome::Bound { node, .. } => node,
        other => panic!("expected Bound, got {:?}", other),
    };
    assert_eq!(node_name, "n2", "scheduler should pick fattest node");

    // ── 4. Kubelet picks up the assignment and runs the pod ────────────────
    let mut pa = PodAssignment {
        namespace: "default".into(),
        name: "web-1".into(),
        uid: "u1".into(),
        node: node_name,
        phase: PodPhase::Pending,
        started_at: chrono::Utc::now(),
        restart_count: 0,
    };
    let new = drive_pod_action(&mut pa, LifecycleAction::Start);
    assert_eq!(new, PodPhase::Running);
}

#[test]
fn smoke_2_deployment_rolling_update() {
    // 1 -> 5 with max_surge = 2 should produce monotone increasing steps
    let plan = plan_rolling_update(5, 1, 2, 0);
    assert!(!plan.is_empty(), "rollout should produce at least one batch");
    // Final step must reach desired replica count
    assert_eq!(plan.last().unwrap().replicas, 5);
    // Steps must be monotonically increasing
    let replicas: Vec<u32> = plan.iter().map(|s| s.replicas).collect();
    for w in replicas.windows(2) {
        assert!(w[0] <= w[1], "step replica counts must be non-decreasing: {:?}", replicas);
    }
}

#[test]
fn smoke_3_service_endpoint_binding() {
    // Six pods backing a service, max 4 endpoints per slice
    let pods: Vec<_> = (0..6)
        .map(|i| {
            (
                format!("backend-{}", i),
                format!("n{}", i % 3),
                format!("10.244.0.{}", 10 + i),
                true,
            )
        })
        .collect();
    let ports = vec![ServicePort {
        name: "http".into(),
        port: 80,
        target_port: 8080,
        node_port: None,
        protocol: "TCP".into(),
    }];
    let slices = derive_slices("prod", "web", IpFamily::Ipv4, ports, &pods, 4);
    assert_eq!(slices.len(), 2, "6 pods / 4 per slice = 2 slices");
    assert_eq!(slices[0].endpoints.len(), 4);
    assert_eq!(slices[1].endpoints.len(), 2);
    let ready_total: usize = slices.iter().map(|s| s.ready_count()).sum();
    assert_eq!(ready_total, 6);
}

#[test]
fn smoke_4_rbac_deny_path() {
    // alice has no role binding -> chain returns Forbidden
    let rbac = RbacAuthorizer::default();
    // Only bob is bound.
    rbac.add_role(Role {
        name: "view".into(),
        namespace: Some("prod".into()),
        rules: vec![RbacRule {
            api_groups: vec!["".into()],
            resources: vec!["pods".into()],
            resource_names: vec![],
            verbs: vec![Verb::Get],
        }],
    });
    rbac.bind(Binding {
        name: "bob-view".into(),
        namespace: Some("prod".into()),
        role_name: "view".into(),
        cluster_role: false,
        subjects: vec![Subject {
            kind: SubjectKind::User,
            name: "bob".into(),
        }],
    });

    let x509 = X509ClientCertAuthenticator::default();
    x509.add_cn("alice", vec![]);
    let authn = ChainAuthenticator::new().add(Box::new(x509));
    let id = authn.authenticate("x509://alice").unwrap();

    let authz = ChainAuthorizer::new().add(Box::new(rbac));
    let r = authz.authorize(&Attributes {
        user: id,
        verb: Verb::Get,
        api_group: "".into(),
        resource: "pods".into(),
        namespace: Some("prod".into()),
        name: None,
    });
    assert!(matches!(r, Err(Error::Forbidden(_))));
}

#[test]
fn smoke_5_namespace_quota_enforce() {
    let q = QuotaTracker::new();
    q.install(Quota::new("prod", "pods").with_limit(Dimension::Pods, 3));
    // Commit three pod creations -> ok
    for _ in 0..3 {
        q.admit_and_commit("prod", &Dimension::Pods, 1).unwrap();
    }
    // Fourth pod must be rejected
    let err = q.check_admit("prod", &Dimension::Pods, 1).unwrap_err();
    match err {
        Error::QuotaExceeded { quota, detail } => {
            assert!(quota.contains("pods"));
            assert!(detail.contains("pods"));
        }
        other => panic!("expected QuotaExceeded, got {:?}", other),
    }
}

#[test]
fn smoke_6_admission_chain_end_to_end() {
    // A full admission chain admitting a benign pod (covers NamespaceLifecycle
    // protected list + ServiceAccount defaulter mutation + PodSecurity
    // restricted profile gate).
    let chain = Chain::new()
        .add(Box::new(NamespaceLifecycle::default()))
        .add(Box::new(ServiceAccountDefaulter))
        .add(Box::new(PodSecurityRestricted));
    let mut r = Request {
        operation: Operation::Create,
        namespace: "default".into(),
        kind: "Pod".into(),
        name: "ok".into(),
        user: "alice".into(),
        object: serde_json::json!({
            "spec": {
                "containers": [{"name": "c1", "image": "nginx"}]
            }
        }),
    };
    chain.admit(&mut r).unwrap();
    assert_eq!(
        r.object["spec"]["serviceAccountName"].as_str(),
        Some("default")
    );
}
