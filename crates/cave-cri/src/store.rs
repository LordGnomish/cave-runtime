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

    // --- ContainerStore edge cases ---

    #[test]
    fn test_store_get_nonexistent() {
        let store = ContainerStore::new();
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_store_remove_nonexistent() {
        let store = ContainerStore::new();
        // Removing a non-existent key should return None, not panic
        let result = store.remove(&Uuid::new_v4());
        assert!(result.is_none());
    }

    #[test]
    fn test_store_update_replaces_existing() {
        let store = ContainerStore::new();
        let mut c = make_test_container();
        let id = c.id;
        store.insert(c.clone());

        c.status = ContainerStatus::Running;
        store.update(c);

        let got = store.get(&id).unwrap();
        assert_eq!(got.status, ContainerStatus::Running);
    }

    #[test]
    fn test_store_count_tracks_inserts_and_removes() {
        let store = ContainerStore::new();
        assert_eq!(store.count(), 0);

        let c1 = make_test_container();
        let c2 = make_test_container();
        let id1 = c1.id;

        store.insert(c1);
        assert_eq!(store.count(), 1);
        store.insert(c2);
        assert_eq!(store.count(), 2);

        store.remove(&id1);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_store_concurrent_inserts() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(ContainerStore::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                s.insert(make_test_container());
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(store.count(), 10);
    }

    #[test]
    fn test_store_concurrent_reads_while_writing() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(ContainerStore::new());
        // Pre-populate
        for _ in 0..5 {
            store.insert(make_test_container());
        }

        let mut handles = vec![];
        for _ in 0..5 {
            let s = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let _ = s.list();
                let _ = s.count();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    // --- ImageStore ---

    fn make_test_image() -> crate::models::OciImage {
        crate::models::OciImage {
            reference: format!("nginx:{}", Uuid::new_v4()),
            digest: "sha256:abc".into(),
            layers: vec![],
            config: crate::models::ImageConfig::default(),
            size_bytes: 1024,
            pulled_at: Utc::now(),
        }
    }

    #[test]
    fn test_image_store_crud() {
        let store = super::ImageStore::new();
        let img = make_test_image();
        let reference = img.reference.clone();

        store.insert(img);
        assert!(store.get(&reference).is_some());

        let removed = store.remove(&reference).unwrap();
        assert_eq!(removed.reference, reference);
        assert!(store.get(&reference).is_none());
    }

    #[test]
    fn test_image_store_get_nonexistent() {
        let store = super::ImageStore::new();
        assert!(store.get("nonexistent:image").is_none());
    }

    #[test]
    fn test_image_store_remove_nonexistent() {
        let store = super::ImageStore::new();
        assert!(store.remove("nonexistent:image").is_none());
    }

    #[test]
    fn test_image_store_list_empty() {
        let store = super::ImageStore::new();
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_image_store_list_multiple() {
        let store = super::ImageStore::new();
        store.insert(make_test_image());
        store.insert(make_test_image());
        store.insert(make_test_image());
        assert_eq!(store.list().len(), 3);
    }

    #[test]
    fn test_image_store_overwrite() {
        let store = super::ImageStore::new();
        let mut img = make_test_image();
        img.reference = "nginx:latest".into();
        img.size_bytes = 100;
        store.insert(img.clone());

        img.size_bytes = 200;
        store.insert(img);

        let got = store.get("nginx:latest").unwrap();
        assert_eq!(got.size_bytes, 200);
    }
}
