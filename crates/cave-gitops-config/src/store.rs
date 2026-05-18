// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory store for cave-gitops-config.

use crate::models::{
    ClusterDestination, ClusterStatus, PipelineRun, Promise, ResourceRequest,
    ResourceRequestStatus, StateStoreEntry,
};
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct GitOpsStore {
    promises: Arc<Mutex<Vec<Promise>>>,
    resource_requests: Arc<Mutex<Vec<ResourceRequest>>>,
    pipeline_runs: Arc<Mutex<Vec<PipelineRun>>>,
    state_store: Arc<Mutex<Vec<StateStoreEntry>>>,
    clusters: Arc<Mutex<Vec<ClusterDestination>>>,
}

impl GitOpsStore {
    pub fn new() -> Self {
        Self {
            promises: Arc::new(Mutex::new(vec![])),
            resource_requests: Arc::new(Mutex::new(vec![])),
            pipeline_runs: Arc::new(Mutex::new(vec![])),
            state_store: Arc::new(Mutex::new(vec![])),
            clusters: Arc::new(Mutex::new(vec![])),
        }
    }

    // ─── Promises ─────────────────────────────────────────────────────────────

    pub fn list_promises(&self) -> Vec<Promise> {
        self.promises.lock().unwrap().clone()
    }

    pub fn get_promise(&self, name: &str) -> Option<Promise> {
        self.promises
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.name == name)
            .cloned()
    }

    pub fn create_promise(&self, promise: Promise) -> Promise {
        let mut store = self.promises.lock().unwrap();
        store.push(promise.clone());
        promise
    }

    pub fn update_promise(&self, name: &str, updated: Promise) -> Option<Promise> {
        let mut store = self.promises.lock().unwrap();
        if let Some(p) = store.iter_mut().find(|p| p.name == name) {
            *p = updated.clone();
            Some(updated)
        } else {
            None
        }
    }

    pub fn delete_promise(&self, name: &str) -> bool {
        let mut store = self.promises.lock().unwrap();
        let len_before = store.len();
        store.retain(|p| p.name != name);
        store.len() < len_before
    }

    // ─── Resource Requests ────────────────────────────────────────────────────

    pub fn list_resource_requests(&self, promise_name: Option<&str>) -> Vec<ResourceRequest> {
        let store = self.resource_requests.lock().unwrap();
        match promise_name {
            Some(name) => store.iter().filter(|r| r.promise_name == name).cloned().collect(),
            None => store.clone(),
        }
    }

    pub fn get_resource_request(&self, id: Uuid) -> Option<ResourceRequest> {
        self.resource_requests
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    pub fn create_resource_request(&self, request: ResourceRequest) -> ResourceRequest {
        let mut store = self.resource_requests.lock().unwrap();
        store.push(request.clone());
        request
    }

    pub fn update_resource_request_status(
        &self,
        id: Uuid,
        status: ResourceRequestStatus,
        pipeline_run: Option<PipelineRun>,
        destinations: Option<Vec<String>>,
    ) -> bool {
        let mut store = self.resource_requests.lock().unwrap();
        if let Some(req) = store.iter_mut().find(|r| r.id == id) {
            req.status = status;
            req.updated_at = Utc::now();
            if let Some(run) = pipeline_run {
                req.pipeline_run = Some(run);
            }
            if let Some(dests) = destinations {
                req.destinations = dests;
            }
            true
        } else {
            false
        }
    }

    pub fn delete_resource_request(&self, id: Uuid) -> bool {
        let mut store = self.resource_requests.lock().unwrap();
        let len_before = store.len();
        store.retain(|r| r.id != id);
        store.len() < len_before
    }

    // ─── Pipeline Runs ────────────────────────────────────────────────────────

    pub fn get_pipeline_run(&self, resource_request_id: Uuid) -> Option<PipelineRun> {
        self.pipeline_runs
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.resource_request_id == resource_request_id)
            .cloned()
    }

    pub fn add_pipeline_run(&self, run: PipelineRun) -> PipelineRun {
        let mut store = self.pipeline_runs.lock().unwrap();
        store.push(run.clone());
        run
    }

    pub fn update_pipeline_run(&self, id: Uuid, updated: PipelineRun) -> bool {
        let mut store = self.pipeline_runs.lock().unwrap();
        if let Some(run) = store.iter_mut().find(|r| r.id == id) {
            *run = updated;
            true
        } else {
            false
        }
    }

    // ─── State Store ──────────────────────────────────────────────────────────

    pub fn list_state_entries(&self, cluster: Option<&str>) -> Vec<StateStoreEntry> {
        let store = self.state_store.lock().unwrap();
        match cluster {
            Some(c) => store.iter().filter(|e| e.cluster == c).cloned().collect(),
            None => store.clone(),
        }
    }

    pub fn get_state_entry(&self, path: &str) -> Option<StateStoreEntry> {
        self.state_store
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.path == path)
            .cloned()
    }

    /// Insert or update a state store entry by path.
    pub fn upsert_state_entry(&self, entry: StateStoreEntry) -> StateStoreEntry {
        let mut store = self.state_store.lock().unwrap();
        if let Some(existing) = store.iter_mut().find(|e| e.path == entry.path) {
            *existing = entry.clone();
        } else {
            store.push(entry.clone());
        }
        entry
    }

    pub fn delete_state_entry(&self, path: &str) -> bool {
        let mut store = self.state_store.lock().unwrap();
        let len_before = store.len();
        store.retain(|e| e.path != path);
        store.len() < len_before
    }

    // ─── Clusters ─────────────────────────────────────────────────────────────

    pub fn list_clusters(&self) -> Vec<ClusterDestination> {
        self.clusters.lock().unwrap().clone()
    }

    pub fn register_cluster(&self, cluster: ClusterDestination) -> ClusterDestination {
        let mut store = self.clusters.lock().unwrap();
        // Replace if already registered by name
        if let Some(existing) = store.iter_mut().find(|c| c.name == cluster.name) {
            *existing = cluster.clone();
        } else {
            store.push(cluster.clone());
        }
        cluster
    }

    pub fn update_cluster_status(&self, name: &str, status: ClusterStatus) -> bool {
        let mut store = self.clusters.lock().unwrap();
        if let Some(c) = store.iter_mut().find(|c| c.name == name) {
            c.status = status;
            true
        } else {
            false
        }
    }
}

impl Default for GitOpsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ClusterStatus, PipelineRunStatus, PromiseStatus, ResourceRequestStatus, SyncStatus,
    };
    use std::collections::HashMap;

    fn make_promise(name: &str) -> Promise {
        Promise {
            id: Uuid::new_v4(),
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: "test".to_string(),
            api_schema: serde_json::json!({}),
            pipeline: vec![],
            dependencies: vec![],
            destination_selectors: vec![],
            status: PromiseStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_request(promise_name: &str) -> ResourceRequest {
        ResourceRequest {
            id: Uuid::new_v4(),
            promise_name: promise_name.to_string(),
            promise_version: "1.0.0".to_string(),
            namespace: "default".to_string(),
            name: "test-resource".to_string(),
            spec: serde_json::json!({}),
            requester: Uuid::new_v4(),
            status: ResourceRequestStatus::Pending,
            pipeline_run: None,
            destinations: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_cluster(name: &str) -> ClusterDestination {
        ClusterDestination {
            name: name.to_string(),
            api_server: format!("https://{}.k8s.example.com", name),
            labels: HashMap::new(),
            status: ClusterStatus::Ready,
            registered_at: Utc::now(),
        }
    }

    fn make_state_entry(path: &str, cluster: &str) -> StateStoreEntry {
        StateStoreEntry {
            id: Uuid::new_v4(),
            path: path.to_string(),
            cluster: cluster.to_string(),
            content: "apiVersion: v1".to_string(),
            checksum: "abc123".to_string(),
            promise_name: "postgresql".to_string(),
            resource_request_id: Uuid::new_v4(),
            last_synced: Some(Utc::now()),
            sync_status: SyncStatus::Synced,
        }
    }

    fn make_pipeline_run(resource_request_id: Uuid) -> PipelineRun {
        PipelineRun {
            id: Uuid::new_v4(),
            resource_request_id,
            promise_name: "postgresql".to_string(),
            stages: vec![],
            status: PipelineRunStatus::Completed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        }
    }

    #[test]
    fn test_create_and_get_promise() {
        let store = GitOpsStore::new();
        let p = make_promise("postgresql");
        store.create_promise(p);
        let found = store.get_promise("postgresql").unwrap();
        assert_eq!(found.name, "postgresql");
    }

    #[test]
    fn test_list_promises() {
        let store = GitOpsStore::new();
        store.create_promise(make_promise("postgresql"));
        store.create_promise(make_promise("redis"));
        assert_eq!(store.list_promises().len(), 2);
    }

    #[test]
    fn test_update_promise() {
        let store = GitOpsStore::new();
        let p = make_promise("postgresql");
        store.create_promise(p.clone());
        let updated = Promise {
            version: "2.0.0".to_string(),
            ..p.clone()
        };
        store.update_promise("postgresql", updated);
        assert_eq!(store.get_promise("postgresql").unwrap().version, "2.0.0");
    }

    #[test]
    fn test_delete_promise() {
        let store = GitOpsStore::new();
        store.create_promise(make_promise("kafka"));
        assert!(store.delete_promise("kafka"));
        assert!(store.get_promise("kafka").is_none());
    }

    #[test]
    fn test_resource_request_crud() {
        let store = GitOpsStore::new();
        let req = make_request("postgresql");
        let id = req.id;
        store.create_resource_request(req);
        let found = store.get_resource_request(id).unwrap();
        assert_eq!(found.promise_name, "postgresql");
    }

    #[test]
    fn test_list_requests_filtered_by_promise() {
        let store = GitOpsStore::new();
        store.create_resource_request(make_request("postgresql"));
        store.create_resource_request(make_request("postgresql"));
        store.create_resource_request(make_request("redis"));
        let pg_reqs = store.list_resource_requests(Some("postgresql"));
        assert_eq!(pg_reqs.len(), 2);
    }

    #[test]
    fn test_multiple_resource_requests_for_same_promise() {
        let store = GitOpsStore::new();
        for i in 0..3 {
            let mut req = make_request("postgresql");
            req.name = format!("db-{i}");
            store.create_resource_request(req);
        }
        assert_eq!(
            store.list_resource_requests(Some("postgresql")).len(),
            3
        );
    }

    #[test]
    fn test_pipeline_run_add_and_get() {
        let store = GitOpsStore::new();
        let req = make_request("postgresql");
        let run = make_pipeline_run(req.id);
        store.add_pipeline_run(run.clone());
        let found = store.get_pipeline_run(req.id).unwrap();
        assert_eq!(found.resource_request_id, req.id);
    }

    #[test]
    fn test_state_store_upsert_and_get() {
        let store = GitOpsStore::new();
        let path = "clusters/prod/postgresql/default/my-db.yaml";
        let entry = make_state_entry(path, "prod");
        store.upsert_state_entry(entry);
        let found = store.get_state_entry(path).unwrap();
        assert_eq!(found.path, path);
        assert_eq!(found.cluster, "prod");
    }

    #[test]
    fn test_state_store_upsert_updates_existing() {
        let store = GitOpsStore::new();
        let path = "clusters/prod/postgresql/default/my-db.yaml";
        let entry = make_state_entry(path, "prod");
        store.upsert_state_entry(entry.clone());
        let updated = StateStoreEntry {
            content: "apiVersion: v2".to_string(),
            checksum: "def456".to_string(),
            sync_status: SyncStatus::OutOfSync,
            ..entry
        };
        store.upsert_state_entry(updated);
        let found = store.get_state_entry(path).unwrap();
        assert_eq!(found.checksum, "def456");
        assert_eq!(found.sync_status, SyncStatus::OutOfSync);
        // Should only have one entry, not two
        assert_eq!(store.list_state_entries(None).len(), 1);
    }

    #[test]
    fn test_list_state_entries_by_cluster() {
        let store = GitOpsStore::new();
        store.upsert_state_entry(make_state_entry("path/a", "prod"));
        store.upsert_state_entry(make_state_entry("path/b", "prod"));
        store.upsert_state_entry(make_state_entry("path/c", "staging"));
        assert_eq!(store.list_state_entries(Some("prod")).len(), 2);
        assert_eq!(store.list_state_entries(Some("staging")).len(), 1);
    }

    #[test]
    fn test_register_cluster_and_update_status() {
        let store = GitOpsStore::new();
        let cluster = make_cluster("prod-1");
        store.register_cluster(cluster);
        assert_eq!(store.list_clusters().len(), 1);
        assert!(store.update_cluster_status("prod-1", ClusterStatus::NotReady));
        let clusters = store.list_clusters();
        assert_eq!(clusters[0].status, ClusterStatus::NotReady);
    }

    #[test]
    fn test_delete_state_entry() {
        let store = GitOpsStore::new();
        let path = "clusters/prod/redis/default/cache.yaml";
        store.upsert_state_entry(make_state_entry(path, "prod"));
        assert!(store.delete_state_entry(path));
        assert!(store.get_state_entry(path).is_none());
    }
}
