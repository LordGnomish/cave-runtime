//! Resource store — namespaced, versioned storage for K8s resources.

use crate::error::{ApiError, ApiResult};
use crate::resources::Resource;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

type ResourceKey = (String, String, String); // (kind, namespace, name)

/// K8s-compatible resource store with watch support.
pub struct ResourceStore {
    resources: DashMap<ResourceKey, Resource>,
    revision: AtomicU64,
    watch_tx: broadcast::Sender<WatchEvent>,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub event_type: WatchEventType,
    pub resource: Resource,
}

#[derive(Debug, Clone)]
pub enum WatchEventType {
    Added,
    Modified,
    Deleted,
}

impl ResourceStore {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(4096);
        Self {
            resources: DashMap::new(),
            revision: AtomicU64::new(1),
            watch_tx,
        }
    }

    #[allow(dead_code)]
    fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn create(&self, resource: Resource) -> ApiResult<Resource> {
        let key = (resource.kind().to_string(), resource.namespace().to_string(), resource.name().to_string());
        if self.resources.contains_key(&key) {
            return Err(ApiError::AlreadyExists { kind: key.0, name: key.2 });
        }
        self.resources.insert(key, resource.clone());
        let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Added, resource: resource.clone() });
        Ok(resource)
    }

    pub fn get(&self, kind: &str, namespace: &str, name: &str) -> ApiResult<Resource> {
        let key = (kind.to_string(), namespace.to_string(), name.to_string());
        self.resources.get(&key)
            .map(|r| r.value().clone())
            .ok_or(ApiError::NotFound { kind: kind.to_string(), name: name.to_string() })
    }

    pub fn list(&self, kind: &str, namespace: &str) -> Vec<Resource> {
        self.resources.iter()
            .filter(|r| r.key().0 == kind && (namespace.is_empty() || r.key().1 == namespace))
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn update(&self, resource: Resource) -> ApiResult<Resource> {
        let key = (resource.kind().to_string(), resource.namespace().to_string(), resource.name().to_string());
        if !self.resources.contains_key(&key) {
            return Err(ApiError::NotFound { kind: key.0, name: key.2 });
        }
        self.resources.insert(key, resource.clone());
        let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Modified, resource: resource.clone() });
        Ok(resource)
    }

    pub fn delete(&self, kind: &str, namespace: &str, name: &str) -> ApiResult<Resource> {
        let key = (kind.to_string(), namespace.to_string(), name.to_string());
        self.resources.remove(&key)
            .map(|(_, r)| {
                let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Deleted, resource: r.clone() });
                r
            })
            .ok_or(ApiError::NotFound { kind: kind.to_string(), name: name.to_string() })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.watch_tx.subscribe()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.resources.iter().filter(|r| r.key().0 == kind).count()
    }
}

impl Default for ResourceStore {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    fn make_configmap(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(),
        })
    }

    #[test]
    fn test_create_and_get() {
        let store = ResourceStore::new();
        let cm = make_configmap("myconfig", "default");
        store.create(cm).unwrap();
        let got = store.get("ConfigMap", "default", "myconfig").unwrap();
        assert_eq!(got.name(), "myconfig");
    }

    #[test]
    fn test_create_duplicate_fails() {
        let store = ResourceStore::new();
        store.create(make_configmap("dup", "default")).unwrap();
        assert!(store.create(make_configmap("dup", "default")).is_err());
    }

    #[test]
    fn test_list_by_namespace() {
        let store = ResourceStore::new();
        store.create(make_configmap("a", "ns1")).unwrap();
        store.create(make_configmap("b", "ns1")).unwrap();
        store.create(make_configmap("c", "ns2")).unwrap();
        assert_eq!(store.list("ConfigMap", "ns1").len(), 2);
        assert_eq!(store.list("ConfigMap", "ns2").len(), 1);
    }

    #[test]
    fn test_delete() {
        let store = ResourceStore::new();
        store.create(make_configmap("del", "default")).unwrap();
        store.delete("ConfigMap", "default", "del").unwrap();
        assert!(store.get("ConfigMap", "default", "del").is_err());
    }

    #[test]
    fn test_watch() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_configmap("w", "default")).unwrap();
        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, WatchEventType::Added));
    }
}
