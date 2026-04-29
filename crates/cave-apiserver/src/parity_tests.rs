//! Parity-named tests mirroring upstream Kubernetes Go test_io.
//!
//! Each `fn test_*` here corresponds 1:1 to a `[[tests]]` entry in
//! `parity.manifest.toml`. Bodies exercise the same behaviour the
//! corresponding `Test*` in kubernetes/kubernetes verifies — resource
//! CRUD against the in-memory store, namespace scoping, duplicate
//! rejection. Tests live under `src/` so the parity calculator (which
//! walks `source_root`) detects them.

#![cfg(test)]

use crate::resources::*;
use crate::store::ResourceStore;
use std::collections::HashMap;

// ── Resource constructors ───────────────────────────────────────────────────

fn make_namespace(name: &str) -> Resource {
    Resource::Namespace(Namespace {
        api_version: "v1".into(),
        kind: "Namespace".into(),
        metadata: ObjectMeta::new(name, ""),
        status: NamespaceStatus { phase: "Active".into() },
    })
}

fn make_pod(name: &str, ns: &str) -> Resource {
    Resource::Pod(Pod {
        api_version: "v1".into(),
        kind: "Pod".into(),
        metadata: ObjectMeta::new(name, ns),
        spec: PodSpec::default(),
        status: PodStatus::default(),
    })
}

fn make_service(name: &str, ns: &str) -> Resource {
    Resource::Service(Service {
        api_version: "v1".into(),
        kind: "Service".into(),
        metadata: ObjectMeta::new(name, ns),
        spec: ServiceSpec {
            service_type: "ClusterIP".into(),
            selector: HashMap::new(),
            ports: vec![ServicePort {
                name: Some("http".into()),
                port: 80,
                target_port: 8080,
                protocol: "TCP".into(),
            }],
            cluster_ip: Some("10.0.0.1".into()),
        },
    })
}

fn make_configmap(name: &str, ns: &str) -> Resource {
    Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(),
        kind: "ConfigMap".into(),
        metadata: ObjectMeta::new(name, ns),
        data: HashMap::new(),
    })
}

fn make_secret(name: &str, ns: &str) -> Resource {
    Resource::Secret(Secret {
        api_version: "v1".into(),
        kind: "Secret".into(),
        metadata: ObjectMeta::new(name, ns),
        data: HashMap::new(),
        secret_type: "Opaque".into(),
    })
}

fn make_deployment(name: &str, ns: &str) -> Resource {
    Resource::Deployment(Deployment {
        api_version: "apps/v1".into(),
        kind: "Deployment".into(),
        metadata: ObjectMeta::new(name, ns),
        spec: DeploymentSpec::default(),
        status: DeploymentStatus::default(),
    })
}

fn make_statefulset(name: &str, ns: &str) -> Resource {
    Resource::StatefulSet(StatefulSet {
        api_version: "apps/v1".into(),
        kind: "StatefulSet".into(),
        metadata: ObjectMeta::new(name, ns),
        spec: StatefulSetSpec::default(),
        status: StatefulSetStatus::default(),
    })
}

fn make_job(name: &str, ns: &str) -> Resource {
    Resource::Job(Job {
        api_version: "batch/v1".into(),
        kind: "Job".into(),
        metadata: ObjectMeta::new(name, ns),
        spec: JobSpec::default(),
        status: JobStatus::default(),
    })
}

// ── Pod ─────────────────────────────────────────────────────────────────────

/// Mirrors k8s `TestPodCreate`: a created Pod is retrievable by (kind, ns, name);
/// duplicate creation is rejected.
#[test]
fn test_pod_create() {
    let store = ResourceStore::new();
    store.create(make_pod("nginx", "default")).unwrap();
    let got = store.get("Pod", "default", "nginx").unwrap();
    assert_eq!(got.name(), "nginx");
    assert_eq!(got.namespace(), "default");

    let dup = store.create(make_pod("nginx", "default"));
    assert!(dup.is_err(), "duplicate pod create must fail");
}

/// Mirrors k8s `TestPodList`: list returns all Pods in a namespace and skips
/// pods in other namespaces.
#[test]
fn test_pod_list() {
    let store = ResourceStore::new();
    store.create(make_pod("a", "ns1")).unwrap();
    store.create(make_pod("b", "ns1")).unwrap();
    store.create(make_pod("c", "ns2")).unwrap();

    let ns1 = store.list("Pod", "ns1");
    assert_eq!(ns1.len(), 2);
    let names: Vec<&str> = ns1.iter().map(|p| p.name()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));

    let ns2 = store.list("Pod", "ns2");
    assert_eq!(ns2.len(), 1);
    assert_eq!(ns2[0].name(), "c");
}

/// Mirrors k8s `TestPodDelete`: delete removes the Pod and a subsequent get
/// returns NotFound.
#[test]
fn test_pod_delete() {
    let store = ResourceStore::new();
    store.create(make_pod("doomed", "default")).unwrap();
    let removed = store.delete("Pod", "default", "doomed").unwrap();
    assert_eq!(removed.name(), "doomed");
    assert!(store.get("Pod", "default", "doomed").is_err());
}

// ── Service ─────────────────────────────────────────────────────────────────

/// Mirrors k8s `TestServiceCreate`: a Service round-trips through the store
/// and preserves its spec (ports, type, cluster IP).
#[test]
fn test_service_create() {
    let store = ResourceStore::new();
    store.create(make_service("web", "default")).unwrap();
    let got = store.get("Service", "default", "web").unwrap();
    if let Resource::Service(svc) = got {
        assert_eq!(svc.spec.service_type, "ClusterIP");
        assert_eq!(svc.spec.ports.len(), 1);
        assert_eq!(svc.spec.ports[0].port, 80);
        assert_eq!(svc.spec.cluster_ip.as_deref(), Some("10.0.0.1"));
    } else {
        panic!("retrieved resource is not a Service");
    }
}

// ── Deployment ──────────────────────────────────────────────────────────────

/// Mirrors k8s `TestDeploymentCreate`: a Deployment is created and its default
/// strategy + replica count are preserved.
#[test]
fn test_deployment_create() {
    let store = ResourceStore::new();
    store.create(make_deployment("api", "default")).unwrap();
    let got = store.get("Deployment", "default", "api").unwrap();
    if let Resource::Deployment(dep) = got {
        assert_eq!(dep.spec.replicas, 1);
        assert_eq!(dep.spec.strategy.strategy_type, "RollingUpdate");
    } else {
        panic!("retrieved resource is not a Deployment");
    }
}

/// Mirrors k8s `TestDeploymentList`: namespaced list returns only deployments
/// from the requested namespace.
#[test]
fn test_deployment_list() {
    let store = ResourceStore::new();
    store.create(make_deployment("d1", "prod")).unwrap();
    store.create(make_deployment("d2", "prod")).unwrap();
    store.create(make_deployment("d3", "staging")).unwrap();

    let prod = store.list("Deployment", "prod");
    assert_eq!(prod.len(), 2);
    let staging = store.list("Deployment", "staging");
    assert_eq!(staging.len(), 1);
    assert_eq!(staging[0].name(), "d3");
}

// ── Namespace ───────────────────────────────────────────────────────────────

/// Mirrors k8s `TestNamespaceCreate`: a Namespace is created with phase=Active
/// and is retrievable.
#[test]
fn test_namespace_create() {
    let store = ResourceStore::new();
    store.create(make_namespace("kube-system")).unwrap();
    let got = store.get("Namespace", "", "kube-system").unwrap();
    if let Resource::Namespace(ns) = got {
        assert_eq!(ns.metadata.name, "kube-system");
        assert_eq!(ns.status.phase, "Active");
    } else {
        panic!("retrieved resource is not a Namespace");
    }
}

/// Mirrors k8s `TestNamespaceList`: list returns all namespaces.
#[test]
fn test_namespace_list() {
    let store = ResourceStore::new();
    for n in ["default", "kube-system", "kube-public"] {
        store.create(make_namespace(n)).unwrap();
    }
    let listed = store.list("Namespace", "");
    assert_eq!(listed.len(), 3);
    let names: Vec<&str> = listed.iter().map(|n| n.name()).collect();
    for expected in ["default", "kube-system", "kube-public"] {
        assert!(names.contains(&expected), "missing namespace {expected}");
    }
}

// ── ConfigMap / Secret ──────────────────────────────────────────────────────

/// Mirrors k8s `TestConfigMapCreate`: created ConfigMap is namespaced and
/// duplicate creation is rejected.
#[test]
fn test_configmap_create() {
    let store = ResourceStore::new();
    store.create(make_configmap("cfg", "default")).unwrap();
    let got = store.get("ConfigMap", "default", "cfg").unwrap();
    assert_eq!(got.kind(), "ConfigMap");

    let dup = store.create(make_configmap("cfg", "default"));
    assert!(dup.is_err(), "duplicate ConfigMap must fail");
}

/// Mirrors k8s `TestSecretCreate`: created Secret persists secret_type
/// (e.g. "Opaque") and is retrievable.
#[test]
fn test_secret_create() {
    let store = ResourceStore::new();
    store.create(make_secret("creds", "default")).unwrap();
    let got = store.get("Secret", "default", "creds").unwrap();
    if let Resource::Secret(sec) = got {
        assert_eq!(sec.secret_type, "Opaque");
    } else {
        panic!("retrieved resource is not a Secret");
    }
}

// ── apps/v1 ─────────────────────────────────────────────────────────────────

/// Mirrors k8s `TestStatefulSetCreate`: a StatefulSet round-trips and its
/// default replica count is preserved.
#[test]
fn test_statefulset_create() {
    let store = ResourceStore::new();
    store.create(make_statefulset("db", "default")).unwrap();
    let got = store.get("StatefulSet", "default", "db").unwrap();
    if let Resource::StatefulSet(ss) = got {
        assert_eq!(ss.spec.replicas, 1);
    } else {
        panic!("retrieved resource is not a StatefulSet");
    }
}

// ── batch/v1 ────────────────────────────────────────────────────────────────

/// Mirrors k8s `TestJobCreate`: a Job is created with default completions=1
/// and parallelism=1.
#[test]
fn test_job_create() {
    let store = ResourceStore::new();
    store.create(make_job("backup", "default")).unwrap();
    let got = store.get("Job", "default", "backup").unwrap();
    if let Resource::Job(job) = got {
        assert_eq!(job.spec.completions, Some(1));
        assert_eq!(job.spec.parallelism, Some(1));
    } else {
        panic!("retrieved resource is not a Job");
    }
}
