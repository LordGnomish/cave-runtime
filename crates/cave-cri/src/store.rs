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
}
