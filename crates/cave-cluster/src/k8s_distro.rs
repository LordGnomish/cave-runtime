use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::cluster::KubernetesDistro;

// ── Config & Status ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub distro: KubernetesDistro,
    pub version: String,
    /// `true` = control-plane / server, `false` = agent / worker.
    pub is_server: bool,
    /// URL of an existing server for worker nodes joining a cluster.
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub extra_args: Vec<String>,
    pub node_labels: HashMap<String, String>,
    pub node_taints: Vec<String>,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            distro: KubernetesDistro::K3s,
            version: "v1.29.0+k3s1".to_string(),
            is_server: true,
            server_url: None,
            token: None,
            extra_args: Vec::new(),
            node_labels: HashMap::new(),
            node_taints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallStatus {
    Pending,
    Running,
    Succeeded,
    Failed(String),
}

// ── Job ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallJob {
    pub id: Uuid,
    pub node_id: Uuid,
    pub cluster_id: Uuid,
    pub config: InstallConfig,
    pub status: InstallStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub logs: Vec<String>,
}

impl InstallJob {
    pub fn new(node_id: Uuid, cluster_id: Uuid, config: InstallConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_id,
            cluster_id,
            config,
            status: InstallStatus::Pending,
            started_at: Utc::now(),
            completed_at: None,
            logs: Vec::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.status, InstallStatus::Succeeded | InstallStatus::Failed(_))
    }

    /// Returns the shell command used to install the chosen Kubernetes distribution.
    pub fn install_command(&self) -> String {
        match self.config.distro {
            KubernetesDistro::K3s => {
                format!(
                    "curl -sfL https://get.k3s.io | INSTALL_K3S_VERSION={} sh -",
                    self.config.version
                )
            }
            KubernetesDistro::Rke2 => {
                format!(
                    "curl -sfL https://get.rke2.io | INSTALL_RKE2_VERSION={} sh -",
                    self.config.version
                )
            }
            KubernetesDistro::Kubeadm => {
                format!(
                    "kubeadm init --kubernetes-version={}",
                    self.config.version
                )
            }
        }
    }
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct InstallManager {
    jobs: Arc<RwLock<HashMap<Uuid, InstallJob>>>,
}

impl InstallManager {
    pub fn new() -> Self {
        Self { jobs: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn create_job(
        &self,
        node_id: Uuid,
        cluster_id: Uuid,
        config: InstallConfig,
    ) -> InstallJob {
        let mut job = InstallJob::new(node_id, cluster_id, config);
        job.status = InstallStatus::Running;
        let mut guard = self.jobs.write().await;
        guard.insert(job.id, job.clone());
        tracing::info!(job_id = %job.id, node_id = %node_id, "install job created");
        job
    }

    pub async fn complete_job(
        &self,
        job_id: Uuid,
        success: bool,
        error: Option<String>,
    ) -> Result<(), String> {
        let mut guard = self.jobs.write().await;
        let job = guard.get_mut(&job_id).ok_or_else(|| format!("job not found: {job_id}"))?;
        job.status = if success {
            InstallStatus::Succeeded
        } else {
            InstallStatus::Failed(error.unwrap_or_else(|| "unknown error".to_string()))
        };
        job.completed_at = Some(Utc::now());
        Ok(())
    }

    pub async fn add_log(&self, job_id: Uuid, line: &str) -> Result<(), String> {
        let mut guard = self.jobs.write().await;
        let job = guard.get_mut(&job_id).ok_or_else(|| format!("job not found: {job_id}"))?;
        job.logs.push(line.to_string());
        Ok(())
    }

    pub async fn get_job(&self, job_id: Uuid) -> Option<InstallJob> {
        let guard = self.jobs.read().await;
        guard.get(&job_id).cloned()
    }

    pub async fn list_for_cluster(&self, cluster_id: Uuid) -> Vec<InstallJob> {
        let guard = self.jobs.read().await;
        guard.values().filter(|j| j.cluster_id == cluster_id).cloned().collect()
    }
}

impl Default for InstallManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn config(distro: KubernetesDistro) -> InstallConfig {
        InstallConfig { distro, version: "v1.29.0+k3s1".to_string(), ..Default::default() }
    }

    #[tokio::test]
    async fn test_create_job_running_status() {
        let mgr = InstallManager::new();
        let node_id = Uuid::new_v4();
        let cluster_id = Uuid::new_v4();
        let job = mgr.create_job(node_id, cluster_id, config(KubernetesDistro::K3s)).await;
        assert!(matches!(job.status, InstallStatus::Running));
        assert_eq!(job.node_id, node_id);
        assert_eq!(job.cluster_id, cluster_id);
    }

    #[tokio::test]
    async fn test_complete_job_success() {
        let mgr = InstallManager::new();
        let job = mgr
            .create_job(Uuid::new_v4(), Uuid::new_v4(), config(KubernetesDistro::K3s))
            .await;
        mgr.complete_job(job.id, true, None).await.unwrap();
        let updated = mgr.get_job(job.id).await.unwrap();
        assert!(matches!(updated.status, InstallStatus::Succeeded));
        assert!(updated.is_complete());
        assert!(updated.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_complete_job_failure() {
        let mgr = InstallManager::new();
        let job = mgr
            .create_job(Uuid::new_v4(), Uuid::new_v4(), config(KubernetesDistro::Rke2))
            .await;
        mgr.complete_job(job.id, false, Some("install script failed".to_string()))
            .await
            .unwrap();
        let updated = mgr.get_job(job.id).await.unwrap();
        assert!(matches!(updated.status, InstallStatus::Failed(_)));
    }

    #[test]
    fn test_install_command_k3s() {
        let job = InstallJob::new(Uuid::new_v4(), Uuid::new_v4(), config(KubernetesDistro::K3s));
        let cmd = job.install_command();
        assert!(cmd.contains("get.k3s.io"));
        assert!(cmd.contains("v1.29.0+k3s1"));
    }

    #[test]
    fn test_install_command_rke2() {
        let job = InstallJob::new(Uuid::new_v4(), Uuid::new_v4(), config(KubernetesDistro::Rke2));
        let cmd = job.install_command();
        assert!(cmd.contains("get.rke2.io"));
    }

    #[test]
    fn test_install_command_kubeadm() {
        let job =
            InstallJob::new(Uuid::new_v4(), Uuid::new_v4(), config(KubernetesDistro::Kubeadm));
        let cmd = job.install_command();
        assert!(cmd.contains("kubeadm init"));
    }

    #[tokio::test]
    async fn test_add_log_and_list_for_cluster() {
        let mgr = InstallManager::new();
        let cluster_id = Uuid::new_v4();
        let job = mgr.create_job(Uuid::new_v4(), cluster_id, config(KubernetesDistro::K3s)).await;
        mgr.add_log(job.id, "downloading k3s binary").await.unwrap();
        mgr.add_log(job.id, "install complete").await.unwrap();

        let jobs = mgr.list_for_cluster(cluster_id).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].logs.len(), 2);
    }
}
