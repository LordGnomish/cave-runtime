// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Durable workflow persistence — port of `argoproj/argo-workflows`
//! `persist/sqldb` (offloaded node status repo + workflow archive).
//!
//! Argo offloads large `WorkflowStatus.nodes` maps into a SQL table keyed by
//! `(uid, version)`, where `version` is an FNV-32 hash of the marshalled
//! nodes (`nodeStatusVersion`). Saves are idempotent (duplicate `(uid,
//! version)` rows are ignored) and superseded versions older than a TTL are
//! garbage-collected on each save. Terminal workflows are copied into a
//! workflow archive that supports namespace + label-selector queries.
//!
//! This module ports the *algorithm* (version hashing, idempotent save,
//! retention GC, archive label filtering, durable resume) over a repository
//! abstraction. The in-crate [`OffloadStore`]/[`WorkflowArchive`] back it
//! with an in-memory map; the `cave-rdbms` integration implements the same
//! [`NodeStatusRepo`] trait against SQL — cave-workflows owns the reducer
//! logic, not the wire driver.

use crate::workflow_crd::{NodeStatus, Workflow, WorkflowPhase, WorkflowStatus};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// 32-bit FNV-1a hash (matches Go's `hash/fnv.New32a`-class digest used by
/// Argo's `nodeStatusVersion`).
pub fn fnv32a(bytes: &[u8]) -> u32 {
    const OFFSET: u32 = 2166136261;
    const PRIME: u32 = 16777619;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Serialize a node map and compute its `fnv:<hash>` version string.
/// Returns `(nodes_json, version)`. Deterministic for equal node maps.
pub fn node_status_version(nodes: &HashMap<String, NodeStatus>) -> (String, String) {
    // Sort keys so the marshalled form is canonical (Go marshals maps sorted).
    let mut ordered: Vec<(&String, &NodeStatus)> = nodes.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    let canonical: Vec<&NodeStatus> = ordered.iter().map(|(_, v)| *v).collect();
    let json = serde_json::to_string(&canonical).unwrap_or_default();
    let version = format!("fnv:{}", fnv32a(json.as_bytes()));
    (json, version)
}

/// One offloaded node-status record (`nodesRecord` in Argo).
#[derive(Clone, Debug)]
pub struct OffloadRecord {
    pub uid: Uuid,
    pub version: String,
    pub nodes_json: String,
    pub updated_at: DateTime<Utc>,
}

/// Repository abstraction so a SQL backend (`cave-rdbms`) can swap in.
pub trait NodeStatusRepo {
    fn save(
        &mut self,
        uid: Uuid,
        nodes: &HashMap<String, NodeStatus>,
        now: DateTime<Utc>,
    ) -> String;
    fn get(&self, uid: Uuid, version: &str) -> Option<HashMap<String, NodeStatus>>;
    fn list(&self) -> Vec<(Uuid, String)>;
    fn delete(&mut self, uid: Uuid, version: &str) -> bool;
}

/// In-memory offloaded node-status store with retention GC.
#[derive(Debug)]
pub struct OffloadStore {
    records: HashMap<(Uuid, String), OffloadRecord>,
    /// Superseded versions older than this are GC'd on save.
    ttl: Duration,
}

impl OffloadStore {
    pub fn new() -> Self {
        // Argo default offload TTL is 5 minutes.
        Self {
            records: HashMap::new(),
            ttl: Duration::minutes(5),
        }
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            records: HashMap::new(),
            ttl,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for OffloadStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeStatusRepo for OffloadStore {
    fn save(
        &mut self,
        _uid: Uuid,
        _nodes: &HashMap<String, NodeStatus>,
        _now: DateTime<Utc>,
    ) -> String {
        unimplemented!()
    }

    fn get(&self, _uid: Uuid, _version: &str) -> Option<HashMap<String, NodeStatus>> {
        unimplemented!()
    }

    fn list(&self) -> Vec<(Uuid, String)> {
        unimplemented!()
    }

    fn delete(&mut self, _uid: Uuid, _version: &str) -> bool {
        unimplemented!()
    }
}

/// Archived (terminal) workflow record.
#[derive(Clone, Debug)]
pub struct ArchivedWorkflow {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub phase: WorkflowPhase,
    pub labels: HashMap<String, String>,
    pub workflow: Workflow,
}

/// Workflow archive — terminal workflows copied for long-term query.
#[derive(Debug, Default)]
pub struct WorkflowArchive {
    items: Vec<ArchivedWorkflow>,
}

impl WorkflowArchive {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Archive a terminal workflow with its labels. Only Succeeded / Failed /
    /// Error workflows are archivable; archiving a live one is a no-op that
    /// returns `false`.
    pub fn archive(&mut self, _wf: &Workflow, _labels: HashMap<String, String>) -> bool {
        unimplemented!()
    }

    pub fn get(&self, _uid: Uuid) -> Option<&ArchivedWorkflow> {
        unimplemented!()
    }

    /// List archived workflows in a namespace whose labels match every entry
    /// in `selector` (equality match, AND semantics).
    pub fn list(&self, _namespace: &str, _selector: &HashMap<String, String>) -> Vec<&ArchivedWorkflow> {
        unimplemented!()
    }

    pub fn delete(&mut self, _uid: Uuid) -> bool {
        unimplemented!()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Durable resume: re-hydrate a suspended workflow's offloaded status and
/// transition any `Suspended` nodes (and the workflow phase) back to
/// `Running`. Mirrors `util.ResumeWorkflow` operating on reloaded state.
///
/// Returns the rehydrated [`WorkflowStatus`] with the resume applied, or
/// `None` if no offloaded record exists for `uid`/`version`.
pub fn durable_resume(
    _repo: &dyn NodeStatusRepo,
    _uid: Uuid,
    _version: &str,
    _prior: &WorkflowStatus,
) -> Option<WorkflowStatus> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_crd::{NodeStatus, WorkflowPhase, WorkflowSpec};

    fn node(id: &str, phase: WorkflowPhase) -> NodeStatus {
        NodeStatus {
            id: id.to_string(),
            template_name: "t".to_string(),
            phase,
            message: None,
            started_at: None,
            finished_at: None,
            outputs: None,
            children: vec![],
        }
    }

    fn nodes(spec: &[(&str, WorkflowPhase)]) -> HashMap<String, NodeStatus> {
        spec.iter().map(|(id, p)| (id.to_string(), node(id, *p))).collect()
    }

    #[test]
    fn fnv32a_matches_known_vectors() {
        // FNV-1a 32-bit reference vectors.
        assert_eq!(fnv32a(b""), 2166136261);
        assert_eq!(fnv32a(b"a"), 0xe40c292c);
        assert_eq!(fnv32a(b"foobar"), 0xbf9cf968);
    }

    #[test]
    fn version_is_deterministic_and_order_independent() {
        let a = nodes(&[("n1", WorkflowPhase::Running), ("n2", WorkflowPhase::Pending)]);
        let b = nodes(&[("n2", WorkflowPhase::Pending), ("n1", WorkflowPhase::Running)]);
        let (_, va) = node_status_version(&a);
        let (_, vb) = node_status_version(&b);
        assert_eq!(va, vb, "insertion order must not change the version");
        assert!(va.starts_with("fnv:"));
    }

    #[test]
    fn version_changes_when_a_node_phase_changes() {
        let a = nodes(&[("n1", WorkflowPhase::Running)]);
        let b = nodes(&[("n1", WorkflowPhase::Succeeded)]);
        assert_ne!(node_status_version(&a).1, node_status_version(&b).1);
    }

    #[test]
    fn save_then_get_roundtrips_nodes() {
        let mut store = OffloadStore::new();
        let now = Utc::now();
        let n = nodes(&[("n1", WorkflowPhase::Running), ("n2", WorkflowPhase::Succeeded)]);
        let uid = Uuid::new_v4();
        let version = store.save(uid, &n, now);
        let got = store.get(uid, &version).expect("record present");
        assert_eq!(got.len(), 2);
        assert_eq!(got["n2"].phase, WorkflowPhase::Succeeded);
    }

    #[test]
    fn save_is_idempotent_for_identical_nodes() {
        let mut store = OffloadStore::new();
        let now = Utc::now();
        let n = nodes(&[("n1", WorkflowPhase::Running)]);
        let uid = Uuid::new_v4();
        let v1 = store.save(uid, &n, now);
        let v2 = store.save(uid, &n, now);
        assert_eq!(v1, v2);
        assert_eq!(store.len(), 1, "identical save must not create a 2nd row");
    }

    #[test]
    fn save_gcs_superseded_versions_past_ttl() {
        let mut store = OffloadStore::with_ttl(Duration::minutes(5));
        let uid = Uuid::new_v4();
        let t0 = Utc::now();
        let v1 = store.save(uid, &nodes(&[("n1", WorkflowPhase::Pending)]), t0);
        // A later save with changed nodes, 6 minutes on — v1 is now stale.
        let t1 = t0 + Duration::minutes(6);
        let v2 = store.save(uid, &nodes(&[("n1", WorkflowPhase::Running)]), t1);
        assert_ne!(v1, v2);
        assert!(store.get(uid, &v1).is_none(), "stale version GC'd");
        assert!(store.get(uid, &v2).is_some(), "current version kept");
    }

    #[test]
    fn save_keeps_recent_superseded_version_within_ttl() {
        let mut store = OffloadStore::with_ttl(Duration::minutes(5));
        let uid = Uuid::new_v4();
        let t0 = Utc::now();
        let v1 = store.save(uid, &nodes(&[("n1", WorkflowPhase::Pending)]), t0);
        let v2 = store.save(uid, &nodes(&[("n1", WorkflowPhase::Running)]), t0 + Duration::minutes(1));
        assert!(store.get(uid, &v1).is_some(), "recent prior version retained");
        assert!(store.get(uid, &v2).is_some());
    }

    #[test]
    fn delete_removes_one_version() {
        let mut store = OffloadStore::new();
        let uid = Uuid::new_v4();
        let v = store.save(uid, &nodes(&[("n1", WorkflowPhase::Running)]), Utc::now());
        assert!(store.delete(uid, &v));
        assert!(!store.delete(uid, &v), "second delete is a no-op");
        assert!(store.get(uid, &v).is_none());
    }

    fn terminal_wf(name: &str, phase: WorkflowPhase) -> Workflow {
        let mut wf = Workflow::new(name, "default", WorkflowSpec::default());
        wf.status.phase = phase;
        wf
    }

    #[test]
    fn archive_only_accepts_terminal_workflows() {
        let mut arch = WorkflowArchive::new();
        let running = terminal_wf("live", WorkflowPhase::Running);
        assert!(!arch.archive(&running, HashMap::new()), "running not archivable");
        let done = terminal_wf("done", WorkflowPhase::Succeeded);
        assert!(arch.archive(&done, HashMap::new()));
        assert_eq!(arch.len(), 1);
    }

    #[test]
    fn archive_list_filters_by_namespace_and_label_selector() {
        let mut arch = WorkflowArchive::new();
        let mut wf_a = terminal_wf("a", WorkflowPhase::Succeeded);
        wf_a.namespace = "team-x".to_string();
        let mut wf_b = terminal_wf("b", WorkflowPhase::Failed);
        wf_b.namespace = "team-x".to_string();
        arch.archive(&wf_a, HashMap::from([("app".to_string(), "etl".to_string())]));
        arch.archive(&wf_b, HashMap::from([("app".to_string(), "web".to_string())]));

        let sel = HashMap::from([("app".to_string(), "etl".to_string())]);
        let hits = arch.list("team-x", &sel);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "a");

        // Wrong namespace → nothing.
        assert!(arch.list("other", &sel).is_empty());
        // Empty selector → all in namespace.
        assert_eq!(arch.list("team-x", &HashMap::new()).len(), 2);
    }

    #[test]
    fn durable_resume_transitions_suspended_nodes_and_phase() {
        let mut store = OffloadStore::new();
        let uid = Uuid::new_v4();
        let n = nodes(&[
            ("n0", WorkflowPhase::Succeeded),
            ("n1", WorkflowPhase::Suspended),
        ]);
        let version = store.save(uid, &n, Utc::now());

        let mut prior = WorkflowStatus::default();
        prior.phase = WorkflowPhase::Suspended;

        let resumed = durable_resume(&store, uid, &version, &prior).expect("rehydrated");
        assert_eq!(resumed.phase, WorkflowPhase::Running, "workflow leaves Suspended");
        assert_eq!(resumed.nodes["n1"].phase, WorkflowPhase::Running, "suspended node resumes");
        assert_eq!(resumed.nodes["n0"].phase, WorkflowPhase::Succeeded, "done node untouched");
    }

    #[test]
    fn durable_resume_returns_none_for_missing_record() {
        let store = OffloadStore::new();
        let out = durable_resume(&store, Uuid::new_v4(), "fnv:123", &WorkflowStatus::default());
        assert!(out.is_none());
    }
}
