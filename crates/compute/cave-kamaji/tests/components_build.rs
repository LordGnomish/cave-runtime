// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for the per-tenant control-plane component lifecycle:
//! the kube-apiserver + kube-controller-manager + kube-scheduler containers
//! Kamaji renders into the tenant's management-cluster Deployment.
//!
//! Faithful port target (Kamaji v1.0.0):
//!   internal/builders/controlplane/deployment.go
//!     buildKubeAPIServer / buildControllerManager / buildScheduler /
//!     buildKubeAPIServerCommand
//!   internal/utilities/args.go  — ArgsFromMapToSlice (sorted, bare flags)

use cave_kamaji::components::{
    ComponentImages, ControlPlaneInput, DatastoreBinding, NetworkProfile, args_from_map_to_slice,
    build_apiserver, build_control_plane, build_controller_manager, build_scheduler,
};
use cave_kamaji::connection::Driver;
use std::collections::BTreeMap;

fn input(driver: Driver) -> ControlPlaneInput {
    ControlPlaneInput {
        name: "alpha".into(),
        version: "v1.31.0".into(),
        advertise_address: "10.0.0.5".into(),
        network: NetworkProfile {
            service_cidr: "10.96.0.0/12".into(),
            pod_cidr: "10.244.0.0/16".into(),
            port: 6443,
        },
        datastore: DatastoreBinding {
            driver,
            endpoints: vec!["etcd-0:2379".into(), "etcd-1:2379".into()],
            schema: "tenants_alpha".into(),
        },
        admission_plugins: vec!["NodeRestriction".into()],
        preferred_address_types: vec!["InternalIP".into(), "ExternalIP".into()],
    }
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .find_map(|a| a.strip_prefix(&format!("{flag}=")))
}

// ── ArgsFromMapToSlice (args.go) ────────────────────────────────────────────

#[test]
fn args_render_sorted_with_bare_valueless_flags() {
    let mut m = BTreeMap::new();
    m.insert("--zeta".to_string(), "1".to_string());
    m.insert("--alpha".to_string(), "2".to_string());
    m.insert("--flag".to_string(), String::new()); // valueless -> bare
    let out = args_from_map_to_slice(&m);
    assert_eq!(out, vec!["--alpha=2", "--flag", "--zeta=1"]);
}

// ── Images (RegistrySettings.*Image) ────────────────────────────────────────

#[test]
fn images_default_to_upstream_registry() {
    let img = ComponentImages::new("v1.31.0");
    assert_eq!(img.apiserver(), "registry.k8s.io/kube-apiserver:v1.31.0");
    assert_eq!(
        img.controller_manager(),
        "registry.k8s.io/kube-controller-manager:v1.31.0"
    );
    assert_eq!(img.scheduler(), "registry.k8s.io/kube-scheduler:v1.31.0");
}

// ── Scheduler (buildScheduler) ──────────────────────────────────────────────

#[test]
fn scheduler_container_shape() {
    let c = build_scheduler(&input(Driver::Etcd));
    assert_eq!(c.name, "kube-scheduler");
    assert_eq!(c.command, vec!["kube-scheduler"]);
    assert_eq!(c.image, "registry.k8s.io/kube-scheduler:v1.31.0");
    assert_eq!(arg(&c.args, "--kubeconfig"), Some("/etc/kubernetes/scheduler.conf"));
    assert_eq!(arg(&c.args, "--bind-address"), Some("0.0.0.0"));
    assert_eq!(arg(&c.args, "--leader-elect"), Some("true"));
    assert_eq!(c.liveness_port, 10259);
    // args are sorted
    let mut sorted = c.args.clone();
    sorted.sort();
    assert_eq!(c.args, sorted);
}

// ── Controller-manager (buildControllerManager) ─────────────────────────────

#[test]
fn controller_manager_args_are_faithful() {
    let c = build_controller_manager(&input(Driver::Etcd));
    assert_eq!(c.name, "kube-controller-manager");
    assert_eq!(c.command, vec!["kube-controller-manager"]);
    assert_eq!(c.liveness_port, 10257);
    assert_eq!(arg(&c.args, "--allocate-node-cidrs"), Some("true"));
    assert_eq!(arg(&c.args, "--cluster-name"), Some("alpha"));
    assert_eq!(arg(&c.args, "--cluster-cidr"), Some("10.244.0.0/16"));
    assert_eq!(
        arg(&c.args, "--service-cluster-ip-range"),
        Some("10.96.0.0/12")
    );
    assert_eq!(
        arg(&c.args, "--controllers"),
        Some("*,bootstrapsigner,tokencleaner")
    );
    assert_eq!(
        arg(&c.args, "--cluster-signing-key-file"),
        Some("/etc/kubernetes/pki/ca.key")
    );
    assert_eq!(
        arg(&c.args, "--service-account-private-key-file"),
        Some("/etc/kubernetes/pki/sa.key")
    );
    assert_eq!(
        arg(&c.args, "--use-service-account-credentials"),
        Some("true")
    );
}

// ── API server (buildKubeAPIServerCommand) ──────────────────────────────────

#[test]
fn apiserver_core_args_are_faithful() {
    let c = build_apiserver(&input(Driver::Etcd));
    assert_eq!(c.name, "kube-apiserver");
    assert_eq!(c.command, vec!["kube-apiserver"]);
    assert_eq!(c.liveness_port, 6443);
    assert_eq!(arg(&c.args, "--advertise-address"), Some("10.0.0.5"));
    assert_eq!(arg(&c.args, "--authorization-mode"), Some("Node,RBAC"));
    assert_eq!(arg(&c.args, "--allow-privileged"), Some("true"));
    assert_eq!(arg(&c.args, "--secure-port"), Some("6443"));
    assert_eq!(
        arg(&c.args, "--enable-admission-plugins"),
        Some("NodeRestriction")
    );
    assert_eq!(
        arg(&c.args, "--kubelet-preferred-address-types"),
        Some("InternalIP,ExternalIP")
    );
    assert_eq!(
        arg(&c.args, "--service-account-issuer"),
        Some("https://kubernetes.default.svc.cluster.local")
    );
    assert_eq!(
        arg(&c.args, "--requestheader-allowed-names"),
        Some("front-proxy-client")
    );
    assert_eq!(
        arg(&c.args, "--tls-cert-file"),
        Some("/etc/kubernetes/pki/apiserver.crt")
    );
}

#[test]
fn apiserver_uses_kine_local_etcd_for_sql_drivers() {
    // KineMySQL/KinePostgreSQL/KineNATS -> --etcd-servers=http://127.0.0.1:2379
    for d in [Driver::MySql, Driver::PostgreSql, Driver::Nats] {
        let c = build_apiserver(&input(d));
        assert_eq!(arg(&c.args, "--etcd-servers"), Some("http://127.0.0.1:2379"));
        assert!(arg(&c.args, "--etcd-prefix").is_none(), "kine has no etcd-prefix");
    }
}

#[test]
fn apiserver_uses_external_etcd_for_etcd_driver() {
    let c = build_apiserver(&input(Driver::Etcd));
    assert_eq!(
        arg(&c.args, "--etcd-servers"),
        Some("https://etcd-0:2379,https://etcd-1:2379")
    );
    assert_eq!(arg(&c.args, "--etcd-prefix"), Some("/tenants_alpha"));
    assert_eq!(arg(&c.args, "--etcd-compaction-interval"), Some("0"));
    assert_eq!(
        arg(&c.args, "--etcd-cafile"),
        Some("/etc/kubernetes/pki/etcd/ca.crt")
    );
}

// ── Full control plane (the 3 deployment containers) ────────────────────────

#[test]
fn control_plane_emits_three_named_components() {
    let cs = build_control_plane(&input(Driver::Etcd));
    let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["kube-apiserver", "kube-controller-manager", "kube-scheduler"]
    );
}
