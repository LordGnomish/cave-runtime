// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Attestor plug-in dispatch
// line-ported from pkg/agent/workloadattestor + pkg/agent/plugin/nodeattestor.
//
//! Workload + node attestation engine.
//!
//! Each plug-in returns a (possibly empty) set of selectors that the
//! server matches against `[[RegistrationEntry]]` rows. The intersection
//! determines which SVIDs the workload is allowed to receive.

use crate::error::{IdentityError, Result};
use crate::models::{Selector, SpiffeId, WorkloadAttestation};
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

/// Plug-in trait for workload attestors — `pkg/agent/workloadattestor.WorkloadAttestor`.
#[async_trait]
pub trait WorkloadAttestor: Send + Sync {
    fn name(&self) -> &str;
    /// Attest a workload by pid; returns selectors discovered.
    async fn attest(&self, pid: i32) -> Result<Vec<Selector>>;
}

/// Plug-in trait for node attestors — `pkg/agent/plugin/nodeattestor.NodeAttestor`.
#[async_trait]
pub trait NodeAttestor: Send + Sync {
    fn name(&self) -> &str;
    /// Return the SPIFFE ID + selectors for the node.
    async fn attest(&self) -> Result<(SpiffeId, Vec<Selector>)>;
}

/// Engine — round-trips a list of registered attestors.
pub struct AttestorEngine {
    workload: Arc<DashMap<String, Arc<dyn WorkloadAttestor>>>,
    node: Arc<DashMap<String, Arc<dyn NodeAttestor>>>,
}

impl Default for AttestorEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AttestorEngine {
    pub fn new() -> Self {
        Self {
            workload: Arc::new(DashMap::new()),
            node: Arc::new(DashMap::new()),
        }
    }
    pub fn register_workload(&self, a: Arc<dyn WorkloadAttestor>) {
        self.workload.insert(a.name().to_string(), a);
    }
    pub fn register_node(&self, a: Arc<dyn NodeAttestor>) {
        self.node.insert(a.name().to_string(), a);
    }
    pub fn workload_attestor(&self, name: &str) -> Option<Arc<dyn WorkloadAttestor>> {
        self.workload.get(name).map(|v| v.value().clone())
    }
    pub fn node_attestor(&self, name: &str) -> Option<Arc<dyn NodeAttestor>> {
        self.node.get(name).map(|v| v.value().clone())
    }
    /// Run all registered workload attestors against a pid and union the
    /// selectors they produce (`SelectorMatch.MERGE`).
    pub async fn attest_workload(&self, pid: i32) -> Result<WorkloadAttestation> {
        let mut selectors: Vec<Selector> = Vec::new();
        for kv in self.workload.iter() {
            match kv.value().attest(pid).await {
                Ok(s) => selectors.extend(s),
                Err(IdentityError::AttestationFailed(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(WorkloadAttestation { pid, selectors })
    }

    /// Run a single node attestor by name.
    pub async fn attest_node(&self, name: &str) -> Result<(SpiffeId, Vec<Selector>)> {
        let a = self
            .node_attestor(name)
            .ok_or_else(|| IdentityError::AttestorNotFound(name.to_string()))?;
        a.attest().await
    }
}

/// Unix workload attestor — returns `unix:uid:<u>` + `unix:gid:<g>` etc. from
/// a synthetic process table. Real implementations read `/proc/<pid>/status`
/// or use the equivalent BSD/macOS APIs.
pub struct UnixWorkloadAttestor {
    /// Synthetic table keyed by pid; production uses kernel APIs.
    pub table: DashMap<i32, UnixProcessInfo>,
}

#[derive(Debug, Clone)]
pub struct UnixProcessInfo {
    pub uid: u32,
    pub gid: u32,
    pub path: String,
    pub sha256: Option<String>,
}

impl Default for UnixWorkloadAttestor {
    fn default() -> Self {
        Self {
            table: DashMap::new(),
        }
    }
}

#[async_trait]
impl WorkloadAttestor for UnixWorkloadAttestor {
    fn name(&self) -> &str {
        "unix"
    }
    async fn attest(&self, pid: i32) -> Result<Vec<Selector>> {
        let info = self
            .table
            .get(&pid)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed(format!("pid not found: {}", pid)))?;
        let mut out = vec![
            Selector::new("unix", format!("uid:{}", info.uid)),
            Selector::new("unix", format!("gid:{}", info.gid)),
            Selector::new("unix", format!("path:{}", info.path)),
        ];
        if let Some(sha) = info.sha256 {
            out.push(Selector::new("unix", format!("sha256:{}", sha)));
        }
        Ok(out)
    }
}

/// Docker workload attestor — emits `docker:label:<k>=<v>` selectors based on
/// a static lookup table indexed by container id.
pub struct DockerWorkloadAttestor {
    pub by_pid: DashMap<i32, String>,
    pub by_container: DashMap<String, Vec<(String, String)>>,
}

impl Default for DockerWorkloadAttestor {
    fn default() -> Self {
        Self {
            by_pid: DashMap::new(),
            by_container: DashMap::new(),
        }
    }
}

#[async_trait]
impl WorkloadAttestor for DockerWorkloadAttestor {
    fn name(&self) -> &str {
        "docker"
    }
    async fn attest(&self, pid: i32) -> Result<Vec<Selector>> {
        let container_id = self
            .by_pid
            .get(&pid)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed("docker: pid unknown".into()))?;
        let labels = self
            .by_container
            .get(&container_id)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed("docker: cid unknown".into()))?;
        let mut out = vec![Selector::new("docker", format!("id:{}", container_id))];
        for (k, v) in labels {
            out.push(Selector::new("docker", format!("label:{}={}", k, v)));
        }
        Ok(out)
    }
}

/// X.509 proof-of-possession attestor — emits `x509_pop:fingerprint:<hex>`
/// when the workload presents a cert whose key is the same as one bound to
/// the registration entry.
pub struct X509PopAttestor {
    pub by_pid: DashMap<i32, String>,
}

impl Default for X509PopAttestor {
    fn default() -> Self {
        Self {
            by_pid: DashMap::new(),
        }
    }
}

#[async_trait]
impl WorkloadAttestor for X509PopAttestor {
    fn name(&self) -> &str {
        "x509_pop"
    }
    async fn attest(&self, pid: i32) -> Result<Vec<Selector>> {
        let fp = self
            .by_pid
            .get(&pid)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::AttestationFailed("x509_pop: pid unknown".into()))?;
        Ok(vec![Selector::new("x509_pop", format!("fingerprint:{}", fp))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unix_attestor_yields_selectors() {
        let a = UnixWorkloadAttestor::default();
        a.table.insert(
            42,
            UnixProcessInfo {
                uid: 1000,
                gid: 1000,
                path: "/usr/bin/svc".into(),
                sha256: Some("deadbeef".into()),
            },
        );
        let s = a.attest(42).await.unwrap();
        assert_eq!(s.len(), 4);
        assert!(s
            .iter()
            .any(|sel| sel.kind == "unix" && sel.value == "uid:1000"));
    }

    #[tokio::test]
    async fn docker_attestor_yields_labels() {
        let a = DockerWorkloadAttestor::default();
        a.by_pid.insert(7, "cidA".into());
        a.by_container
            .insert("cidA".into(), vec![("env".into(), "prod".into())]);
        let s = a.attest(7).await.unwrap();
        assert!(s
            .iter()
            .any(|sel| sel.value == "label:env=prod" && sel.kind == "docker"));
        assert!(s.iter().any(|sel| sel.value == "id:cidA"));
    }

    #[tokio::test]
    async fn x509_pop_attestor_emits_fp() {
        let a = X509PopAttestor::default();
        a.by_pid.insert(1, "aabb".into());
        let s = a.attest(1).await.unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].value.starts_with("fingerprint:"));
    }

    #[tokio::test]
    async fn engine_merges_selectors() {
        let eng = AttestorEngine::new();
        let unix = Arc::new(UnixWorkloadAttestor::default());
        unix.table.insert(
            1,
            UnixProcessInfo {
                uid: 0,
                gid: 0,
                path: "/u".into(),
                sha256: None,
            },
        );
        let dock = Arc::new(DockerWorkloadAttestor::default());
        dock.by_pid.insert(1, "c".into());
        dock.by_container.insert("c".into(), vec![]);
        eng.register_workload(unix);
        eng.register_workload(dock);
        let r = eng.attest_workload(1).await.unwrap();
        // unix produces 3 (no sha), docker produces 1 (id only)
        assert!(r.selectors.len() >= 4);
    }

    #[tokio::test]
    async fn engine_missing_node_attestor() {
        let eng = AttestorEngine::new();
        assert!(matches!(
            eng.attest_node("missing").await,
            Err(IdentityError::AttestorNotFound(_))
        ));
    }
}
