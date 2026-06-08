// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-tenant control-plane component lifecycle — the kube-apiserver,
//! kube-controller-manager, and kube-scheduler containers Kamaji renders into
//! each tenant's management-cluster Deployment.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/builders/controlplane/deployment.go
//!     buildKubeAPIServer / buildKubeAPIServerCommand /
//!     buildControllerManager / buildScheduler
//!   internal/utilities/args.go  — ArgsFromMapToSlice
//!
//! Kamaji runs every tenant's control plane as ordinary pods in the management
//! cluster: a single Deployment whose pod carries three containers (plus an
//! optional Kine sidecar for SQL datastores). This module ports the exact
//! command/args each container is given. The Deployment envelope, probes
//! scheduling, and volume plumbing are owned by cave-controller-manager /
//! cave-kubelet; here we produce the deterministic [`Container`] plans.

use crate::connection::Driver;
use std::collections::BTreeMap;

/// kubeadm default PKI directory and cert/key filenames (v1beta3).
const PKI: &str = "/etc/kubernetes/pki";

fn pki(name: &str) -> String {
    format!("{PKI}/{name}")
}

/// `utilities.ArgsFromMapToSlice` — render `flag=value` pairs, emitting a bare
/// flag when the value is empty, and sort the result for idempotency.
pub fn args_from_map_to_slice(args: &BTreeMap<String, String>) -> Vec<String> {
    let mut slice: Vec<String> = args
        .iter()
        .map(|(flag, value)| {
            if value.is_empty() {
                flag.clone()
            } else {
                format!("{flag}={value}")
            }
        })
        .collect();
    slice.sort();
    slice
}

/// Container images for the control-plane components, resolved from the
/// registry + Kubernetes version (`RegistrySettings.*Image`). The default
/// registry is upstream's `registry.k8s.io`.
#[derive(Debug, Clone)]
pub struct ComponentImages {
    pub registry: String,
    pub version: String,
}

impl ComponentImages {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            registry: "registry.k8s.io".to_string(),
            version: version.into(),
        }
    }

    fn image(&self, component: &str) -> String {
        format!("{}/{}:{}", self.registry, component, self.version)
    }

    pub fn apiserver(&self) -> String {
        self.image("kube-apiserver")
    }
    pub fn controller_manager(&self) -> String {
        self.image("kube-controller-manager")
    }
    pub fn scheduler(&self) -> String {
        self.image("kube-scheduler")
    }
}

/// `spec.networkProfile` subset that drives the component args.
#[derive(Debug, Clone)]
pub struct NetworkProfile {
    pub service_cidr: String,
    pub pod_cidr: String,
    pub port: u16,
}

/// The tenant's bound DataStore as the apiserver sees it.
#[derive(Debug, Clone)]
pub struct DatastoreBinding {
    pub driver: Driver,
    /// Bare `host:port` endpoints (etcd driver only); Kine drivers ignore this.
    pub endpoints: Vec<String>,
    /// Per-tenant schema — becomes the etcd key prefix for the etcd driver.
    pub schema: String,
}

/// Everything the component builders read off a TenantControlPlane.
#[derive(Debug, Clone)]
pub struct ControlPlaneInput {
    pub name: String,
    pub version: String,
    pub advertise_address: String,
    pub network: NetworkProfile,
    pub datastore: DatastoreBinding,
    pub admission_plugins: Vec<String>,
    pub preferred_address_types: Vec<String>,
}

/// A rendered control-plane container plan (the fields cave-controller-manager
/// projects into a `corev1.Container`).
#[derive(Debug, Clone)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub args: Vec<String>,
    /// Health-probe port (`/healthz` for cm+scheduler, `/livez` for apiserver).
    pub liveness_port: u16,
}

/// `buildScheduler` — kube-scheduler container.
pub fn build_scheduler(input: &ControlPlaneInput) -> Container {
    let kubeconfig = "/etc/kubernetes/scheduler.conf";
    let mut args = BTreeMap::new();
    args.insert("--authentication-kubeconfig".into(), kubeconfig.into());
    args.insert("--authorization-kubeconfig".into(), kubeconfig.into());
    args.insert("--bind-address".into(), "0.0.0.0".into());
    args.insert("--kubeconfig".into(), kubeconfig.into());
    args.insert("--leader-elect".into(), "true".into());

    Container {
        name: "kube-scheduler".into(),
        image: ComponentImages::new(input.version.clone()).scheduler(),
        command: vec!["kube-scheduler".into()],
        args: args_from_map_to_slice(&args),
        liveness_port: 10259,
    }
}

/// `buildControllerManager` — kube-controller-manager container.
pub fn build_controller_manager(input: &ControlPlaneInput) -> Container {
    let kubeconfig = "/etc/kubernetes/controller-manager.conf";
    let mut args = BTreeMap::new();
    args.insert("--allocate-node-cidrs".into(), "true".into());
    args.insert("--authentication-kubeconfig".into(), kubeconfig.into());
    args.insert("--authorization-kubeconfig".into(), kubeconfig.into());
    args.insert("--bind-address".into(), "0.0.0.0".into());
    args.insert("--client-ca-file".into(), pki("ca.crt"));
    args.insert("--cluster-name".into(), input.name.clone());
    args.insert("--cluster-signing-cert-file".into(), pki("ca.crt"));
    args.insert("--cluster-signing-key-file".into(), pki("ca.key"));
    args.insert("--controllers".into(), "*,bootstrapsigner,tokencleaner".into());
    args.insert("--kubeconfig".into(), kubeconfig.into());
    args.insert("--leader-elect".into(), "true".into());
    args.insert(
        "--service-cluster-ip-range".into(),
        input.network.service_cidr.clone(),
    );
    args.insert("--cluster-cidr".into(), input.network.pod_cidr.clone());
    args.insert(
        "--requestheader-client-ca-file".into(),
        pki("front-proxy-ca.crt"),
    );
    args.insert("--root-ca-file".into(), pki("ca.crt"));
    args.insert("--service-account-private-key-file".into(), pki("sa.key"));
    args.insert("--use-service-account-credentials".into(), "true".into());

    Container {
        name: "kube-controller-manager".into(),
        image: ComponentImages::new(input.version.clone()).controller_manager(),
        command: vec!["kube-controller-manager".into()],
        args: args_from_map_to_slice(&args),
        liveness_port: 10257,
    }
}

/// `buildKubeAPIServerCommand` — the kube-apiserver arg map, including the
/// etcd-vs-Kine `--etcd-servers` switch.
fn apiserver_command(input: &ControlPlaneInput) -> BTreeMap<String, String> {
    let mut args = BTreeMap::new();
    args.insert("--allow-privileged".into(), "true".into());
    args.insert("--authorization-mode".into(), "Node,RBAC".into());
    args.insert("--advertise-address".into(), input.advertise_address.clone());
    args.insert("--client-ca-file".into(), pki("ca.crt"));
    args.insert(
        "--enable-admission-plugins".into(),
        input.admission_plugins.join(","),
    );
    args.insert("--enable-bootstrap-token-auth".into(), "true".into());
    args.insert(
        "--service-cluster-ip-range".into(),
        input.network.service_cidr.clone(),
    );
    args.insert(
        "--kubelet-client-certificate".into(),
        pki("apiserver-kubelet-client.crt"),
    );
    args.insert(
        "--kubelet-client-key".into(),
        pki("apiserver-kubelet-client.key"),
    );
    args.insert(
        "--kubelet-preferred-address-types".into(),
        input.preferred_address_types.join(","),
    );
    args.insert(
        "--proxy-client-cert-file".into(),
        pki("front-proxy-client.crt"),
    );
    args.insert(
        "--proxy-client-key-file".into(),
        pki("front-proxy-client.key"),
    );
    args.insert(
        "--requestheader-allowed-names".into(),
        "front-proxy-client".into(),
    );
    args.insert(
        "--requestheader-client-ca-file".into(),
        pki("front-proxy-ca.crt"),
    );
    args.insert(
        "--requestheader-extra-headers-prefix".into(),
        "X-Remote-Extra-".into(),
    );
    args.insert(
        "--requestheader-group-headers".into(),
        "X-Remote-Group".into(),
    );
    args.insert(
        "--requestheader-username-headers".into(),
        "X-Remote-User".into(),
    );
    args.insert("--secure-port".into(), input.network.port.to_string());
    args.insert(
        "--service-account-issuer".into(),
        "https://kubernetes.default.svc.cluster.local".into(),
    );
    args.insert("--service-account-key-file".into(), pki("sa.pub"));
    args.insert("--service-account-signing-key-file".into(), pki("sa.key"));
    args.insert("--tls-cert-file".into(), pki("apiserver.crt"));
    args.insert("--tls-private-key-file".into(), pki("apiserver.key"));

    match input.datastore.driver {
        Driver::MySql | Driver::PostgreSql | Driver::Nats => {
            // Kine runs as a sidecar exposing a local etcd shim.
            args.insert("--etcd-servers".into(), "http://127.0.0.1:2379".into());
        }
        Driver::Etcd => {
            let https: Vec<String> = input
                .datastore
                .endpoints
                .iter()
                .map(|ep| format!("https://{ep}"))
                .collect();
            args.insert("--etcd-compaction-interval".into(), "0".into());
            args.insert(
                "--etcd-prefix".into(),
                format!("/{}", input.datastore.schema),
            );
            args.insert("--etcd-servers".into(), https.join(","));
            args.insert("--etcd-cafile".into(), pki("etcd/ca.crt"));
            args.insert("--etcd-certfile".into(), pki("etcd/server.crt"));
            args.insert("--etcd-keyfile".into(), pki("etcd/server.key"));
        }
    }

    args
}

/// `buildKubeAPIServer` — kube-apiserver container.
pub fn build_apiserver(input: &ControlPlaneInput) -> Container {
    Container {
        name: "kube-apiserver".into(),
        image: ComponentImages::new(input.version.clone()).apiserver(),
        command: vec!["kube-apiserver".into()],
        args: args_from_map_to_slice(&apiserver_command(input)),
        liveness_port: input.network.port,
    }
}

/// The three control-plane containers Kamaji places in the tenant Deployment,
/// in the order `buildKubeAPIServer -> buildControllerManager -> buildScheduler`.
pub fn build_control_plane(input: &ControlPlaneInput) -> Vec<Container> {
    vec![
        build_apiserver(input),
        build_controller_manager(input),
        build_scheduler(input),
    ]
}
