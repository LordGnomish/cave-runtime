// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory container state store.

use crate::models::Container;
use dashmap::DashMap;
use uuid::Uuid;

/// Thread-safe container store.
#[derive(Debug, Default)]
pub struct ContainerStore {
    containers: DashMap<Uuid, Container>,
}

impl ContainerStore {
    pub fn new() -> Self {
        Self { containers: DashMap::new() }
    }

    pub fn insert(&self, container: Container) {
        self.containers.insert(container.id, container);
    }

    pub fn get(&self, id: &Uuid) -> Option<Container> {
        self.containers.get(id).map(|c| c.clone())
    }

    pub fn update(&self, container: Container) {
        self.containers.insert(container.id, container);
    }

    pub fn remove(&self, id: &Uuid) -> Option<Container> {
        self.containers.remove(id).map(|(_, c)| c)
    }

    pub fn list(&self) -> Vec<Container> {
        self.containers.iter().map(|r| r.value().clone()).collect()
    }

    pub fn count(&self) -> usize {
        self.containers.len()
    }
}

/// In-memory image store.
#[derive(Debug, Default)]
pub struct ImageStore {
    images: DashMap<String, crate::models::OciImage>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self { images: DashMap::new() }
    }

    pub fn insert(&self, image: crate::models::OciImage) {
        self.images.insert(image.reference.clone(), image);
    }

    pub fn get(&self, reference: &str) -> Option<crate::models::OciImage> {
        self.images.get(reference).map(|i| i.clone())
    }

    pub fn list(&self) -> Vec<crate::models::OciImage> {
        self.images.iter().map(|r| r.value().clone()).collect()
    }

    pub fn remove(&self, reference: &str) -> Option<crate::models::OciImage> {
        self.images.remove(reference).map(|(_, i)| i)
    }
}

/// Thread-safe pod sandbox store.
#[derive(Debug, Default)]
pub struct SandboxStore {
    sandboxes: DashMap<Uuid, crate::models::Sandbox>,
}

impl SandboxStore {
    pub fn new() -> Self {
        Self { sandboxes: DashMap::new() }
    }

    pub fn insert(&self, sandbox: crate::models::Sandbox) {
        self.sandboxes.insert(sandbox.id, sandbox);
    }

    pub fn get(&self, id: &Uuid) -> Option<crate::models::Sandbox> {
        self.sandboxes.get(id).map(|s| s.clone())
    }

    pub fn list(&self) -> Vec<crate::models::Sandbox> {
        self.sandboxes.iter().map(|r| r.value().clone()).collect()
    }

    pub fn remove(&self, id: &Uuid) -> Option<crate::models::Sandbox> {
        self.sandboxes.remove(id).map(|(_, s)| s)
    }

    pub fn count(&self) -> usize {
        self.sandboxes.len()
    }
}

/// Thread-safe OCI snapshot store.
#[derive(Debug, Default)]
pub struct SnapshotStore {
    snapshots: DashMap<Uuid, crate::models::Snapshot>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self { snapshots: DashMap::new() }
    }

    pub fn insert(&self, snapshot: crate::models::Snapshot) {
        self.snapshots.insert(snapshot.id, snapshot);
    }

    pub fn get(&self, id: &Uuid) -> Option<crate::models::Snapshot> {
        self.snapshots.get(id).map(|s| s.clone())
    }

    pub fn list(&self) -> Vec<crate::models::Snapshot> {
        self.snapshots.iter().map(|r| r.value().clone()).collect()
    }

    pub fn remove(&self, id: &Uuid) -> Option<crate::models::Snapshot> {
        self.snapshots.remove(id).map(|(_, s)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;

    fn make_test_container() -> Container {
        Container {
            id: Uuid::new_v4(),
            spec: ContainerSpec {
                name: "test".into(),
                image: "nginx:latest".into(),
                command: vec![],
                args: vec![],
                env: Default::default(),
                mounts: vec![],
                resources: Default::default(),
                labels: Default::default(),
                working_dir: None,
                user: None,
                hostname: None,
                network_mode: NetworkMode::Bridge,
                restart_policy: RestartPolicy::Never,
            },
            status: ContainerStatus::Created,
            pid: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp/test".into(),
            log_path: "/tmp/test.log".into(),
            health: None,
        }
    }

    #[test]
    fn test_store_crud() {
        let store = ContainerStore::new();
        let c = make_test_container();
        let id = c.id;

        store.insert(c);
        assert_eq!(store.count(), 1);

        let got = store.get(&id).unwrap();
        assert_eq!(got.spec.name, "test");

        store.remove(&id);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_store_list() {
        let store = ContainerStore::new();
        store.insert(make_test_container());
        store.insert(make_test_container());
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn test_sandbox_store_crud() {
        use crate::models::{Sandbox, SandboxSpec, SandboxState, DnsConfig};
        let store = SandboxStore::new();
        let id = Uuid::new_v4();
        let sandbox = Sandbox {
            id,
            spec: SandboxSpec {
                name: "test-pod".into(),
                namespace: "default".into(),
                labels: Default::default(),
                annotations: Default::default(),
                hostname: None,
                dns_config: Some(DnsConfig::default()),
                port_mappings: vec![],
                log_directory: None,
                cgroup_parent: None,
                runtime_handler: None,
                user_namespace_mode: crate::models::UserNamespaceMode::Host,
            },
            state: SandboxState::Ready,
            created_at: Utc::now(),
            network_ip: Some("10.0.0.1".into()),
        };
        store.insert(sandbox);
        assert_eq!(store.count(), 1);
        let got = store.get(&id).unwrap();
        assert_eq!(got.spec.name, "test-pod");
        store.remove(&id);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_snapshot_store_crud() {
        use crate::models::{Snapshot, SnapshotKind};
        let store = SnapshotStore::new();
        let id = Uuid::new_v4();
        let snap = Snapshot {
            id,
            name: "snap-1".into(),
            parent: None,
            labels: Default::default(),
            created_at: Utc::now(),
            kind: SnapshotKind::Committed,
        };
        store.insert(snap);
        assert_eq!(store.list().len(), 1);
        let got = store.get(&id).unwrap();
        assert_eq!(got.name, "snap-1");
        store.remove(&id);
        assert_eq!(store.list().len(), 0);
    }
}
