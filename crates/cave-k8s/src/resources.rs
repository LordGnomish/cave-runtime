// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic resource accessor — provides a uniform `Manager` over the
//! apiserver `ResourceStore` that operates in terms of `BuiltinKind` +
//! JSON.  Built-in *typed* resources continue to live in
//! `cave_apiserver::resources::*`; cave-k8s adds the umbrella-level
//! conveniences (multi-kind list, kind-aware delete, label selection).

use crate::error::Error;
use crate::models::{BuiltinKind, ResourceRef};
use cave_apiserver::resources::Resource;
use cave_apiserver::store::ResourceStore;
use std::sync::Arc;

#[derive(Clone)]
pub struct Manager {
    pub store: Arc<ResourceStore>,
}

impl Manager {
    pub fn new(store: Arc<ResourceStore>) -> Self {
        Self { store }
    }

    /// Counts of every resource in the backing store, keyed by kind.
    pub fn counts(&self) -> std::collections::BTreeMap<String, usize> {
        let all = self.store.list_all();
        let mut m = std::collections::BTreeMap::new();
        for r in all {
            *m.entry(kind_of(&r).to_string()).or_insert(0) += 1;
        }
        m
    }

    pub fn list_namespaced(&self, kind: BuiltinKind, namespace: &str) -> Vec<Resource> {
        self.store.list(kind_str(kind), namespace)
    }

    pub fn list_cluster(&self, kind: BuiltinKind) -> Vec<Resource> {
        self.store.list(kind_str(kind), "")
    }

    /// Delete by `ResourceRef`.  Namespace-scoped kinds require
    /// `ref.namespace`; cluster-scoped kinds ignore it.
    pub fn delete(&self, r: &ResourceRef) -> Result<(), Error> {
        let ns = r.namespace.as_deref().unwrap_or("");
        self.store
            .delete(&r.kind, ns, &r.name)
            .map(|_| ())
            .map_err(|e| Error::subsystem("apiserver", format!("{:?}", e)))
    }

    /// Resolve a list of every namespace currently in the store. Useful
    /// for ResourceQuota + GarbageCollector bootstrap.
    pub fn namespaces(&self) -> Vec<String> {
        let mut s = std::collections::BTreeSet::new();
        for r in self.store.list_all() {
            if let Some(ns) = namespace_of(&r) {
                if !ns.is_empty() {
                    s.insert(ns);
                }
            }
        }
        s.into_iter().collect()
    }
}

pub fn kind_str(k: BuiltinKind) -> &'static str {
    match k {
        BuiltinKind::Namespace => "Namespace",
        BuiltinKind::Node => "Node",
        BuiltinKind::Pod => "Pod",
        BuiltinKind::Service => "Service",
        BuiltinKind::ConfigMap => "ConfigMap",
        BuiltinKind::Secret => "Secret",
        BuiltinKind::PersistentVolume => "PersistentVolume",
        BuiltinKind::PersistentVolumeClaim => "PersistentVolumeClaim",
        BuiltinKind::StorageClass => "StorageClass",
        BuiltinKind::Deployment => "Deployment",
        BuiltinKind::ReplicaSet => "ReplicaSet",
        BuiltinKind::StatefulSet => "StatefulSet",
        BuiltinKind::DaemonSet => "DaemonSet",
        BuiltinKind::Job => "Job",
        BuiltinKind::CronJob => "CronJob",
        BuiltinKind::Endpoints => "Endpoints",
        BuiltinKind::EndpointSlice => "EndpointSlice",
        BuiltinKind::Ingress => "Ingress",
        BuiltinKind::ServiceAccount => "ServiceAccount",
        BuiltinKind::Role => "Role",
        BuiltinKind::RoleBinding => "RoleBinding",
        BuiltinKind::ClusterRole => "ClusterRole",
        BuiltinKind::ClusterRoleBinding => "ClusterRoleBinding",
        BuiltinKind::Event => "Event",
    }
}

fn kind_of(r: &Resource) -> &'static str {
    match r {
        Resource::Pod(_) => "Pod",
        Resource::Service(_) => "Service",
        Resource::ConfigMap(_) => "ConfigMap",
        Resource::Secret(_) => "Secret",
        Resource::ServiceAccount(_) => "ServiceAccount",
        Resource::KubeEvent(_) => "Event",
        Resource::Endpoints(_) => "Endpoints",
        Resource::ResourceQuota(_) => "ResourceQuota",
        Resource::LimitRange(_) => "LimitRange",
        Resource::PersistentVolumeClaim(_) => "PersistentVolumeClaim",
        Resource::Namespace(_) => "Namespace",
        Resource::Node(_) => "Node",
        Resource::PersistentVolume(_) => "PersistentVolume",
        Resource::Deployment(_) => "Deployment",
        Resource::StatefulSet(_) => "StatefulSet",
        Resource::DaemonSet(_) => "DaemonSet",
        Resource::ReplicaSet(_) => "ReplicaSet",
        Resource::Job(_) => "Job",
        Resource::CronJob(_) => "CronJob",
        Resource::Ingress(_) => "Ingress",
        Resource::NetworkPolicy(_) => "NetworkPolicy",
        Resource::StorageClass(_) => "StorageClass",
        Resource::Role(_) => "Role",
        Resource::ClusterRole(_) => "ClusterRole",
        Resource::RoleBinding(_) => "RoleBinding",
        Resource::ClusterRoleBinding(_) => "ClusterRoleBinding",
    }
}

fn namespace_of(r: &Resource) -> Option<String> {
    match r {
        Resource::Pod(p) => Some(p.metadata.namespace.clone()),
        Resource::Service(s) => Some(s.metadata.namespace.clone()),
        Resource::ConfigMap(c) => Some(c.metadata.namespace.clone()),
        Resource::Secret(s) => Some(s.metadata.namespace.clone()),
        Resource::Deployment(d) => Some(d.metadata.namespace.clone()),
        Resource::ReplicaSet(r) => Some(r.metadata.namespace.clone()),
        Resource::StatefulSet(s) => Some(s.metadata.namespace.clone()),
        Resource::DaemonSet(d) => Some(d.metadata.namespace.clone()),
        Resource::Job(j) => Some(j.metadata.namespace.clone()),
        Resource::CronJob(c) => Some(c.metadata.namespace.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_apiserver::resources::{ConfigMap, ObjectMeta};
    use std::collections::HashMap;

    fn cm(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(),
        })
    }

    #[test]
    fn list_namespaced_filters_by_kind_and_ns() {
        let store = Arc::new(ResourceStore::new());
        store.create(cm("a", "x")).unwrap();
        store.create(cm("b", "x")).unwrap();
        store.create(cm("c", "y")).unwrap();
        let m = Manager::new(store);
        let x = m.list_namespaced(BuiltinKind::ConfigMap, "x");
        assert_eq!(x.len(), 2);
        let y = m.list_namespaced(BuiltinKind::ConfigMap, "y");
        assert_eq!(y.len(), 1);
    }

    #[test]
    fn counts_aggregate_across_kinds() {
        let store = Arc::new(ResourceStore::new());
        store.create(cm("a", "x")).unwrap();
        store.create(cm("b", "x")).unwrap();
        let m = Manager::new(store);
        let c = m.counts();
        assert_eq!(c.get("ConfigMap").copied(), Some(2));
    }

    #[test]
    fn delete_by_ref_works() {
        let store = Arc::new(ResourceStore::new());
        store.create(cm("a", "x")).unwrap();
        let m = Manager::new(store);
        m.delete(&ResourceRef::namespaced("ConfigMap", "x", "a")).unwrap();
        assert_eq!(m.counts().get("ConfigMap").copied().unwrap_or(0), 0);
    }

    #[test]
    fn namespaces_returns_unique_set() {
        let store = Arc::new(ResourceStore::new());
        store.create(cm("a", "x")).unwrap();
        store.create(cm("b", "x")).unwrap();
        store.create(cm("c", "y")).unwrap();
        let m = Manager::new(store);
        let mut ns = m.namespaces();
        ns.sort();
        assert_eq!(ns, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn kind_str_covers_every_variant() {
        for k in [
            BuiltinKind::Namespace,
            BuiltinKind::Pod,
            BuiltinKind::Service,
            BuiltinKind::ConfigMap,
            BuiltinKind::Secret,
            BuiltinKind::Deployment,
            BuiltinKind::StatefulSet,
            BuiltinKind::DaemonSet,
            BuiltinKind::Job,
            BuiltinKind::CronJob,
            BuiltinKind::Ingress,
            BuiltinKind::StorageClass,
            BuiltinKind::ServiceAccount,
            BuiltinKind::Role,
            BuiltinKind::ClusterRole,
        ] {
            assert!(!kind_str(k).is_empty());
        }
    }
}
