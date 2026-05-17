// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/models/* (in-memory store backing the ORM layer)
//! In-memory state store for cave-artifacts.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::*;
use crate::pulp::plugin::PluginRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;


/// Central state shared across all route handlers.
pub struct ArtifactsState {
    pub plugins: Arc<PluginRegistry>,
    repositories: Arc<RwLock<HashMap<String, Repository>>>,
    repo_versions: Arc<RwLock<HashMap<String, RepositoryVersion>>>,
    remotes: Arc<RwLock<HashMap<String, Remote>>>,
    content: Arc<RwLock<HashMap<String, ContentUnit>>>,
    artifacts: Arc<RwLock<HashMap<String, Artifact>>>,
    publications: Arc<RwLock<HashMap<String, Publication>>>,
    distributions: Arc<RwLock<HashMap<String, Distribution>>>,
    content_guards: Arc<RwLock<HashMap<String, ContentGuard>>>,
    signing_services: Arc<RwLock<HashMap<String, SigningService>>>,
    exporters: Arc<RwLock<HashMap<String, Exporter>>>,
    tasks: Arc<RwLock<HashMap<String, Task>>>,
}

impl ArtifactsState {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(PluginRegistry::default()),
            repositories: Default::default(),
            repo_versions: Default::default(),
            remotes: Default::default(),
            content: Default::default(),
            artifacts: Default::default(),
            publications: Default::default(),
            distributions: Default::default(),
            content_guards: Default::default(),
            signing_services: Default::default(),
            exporters: Default::default(),
            tasks: Default::default(),
        }
    }

    // -----------------------------------------------------------------------
    // Repositories
    // -----------------------------------------------------------------------

    pub async fn create_repository(
        &self,
        req: CreateRepositoryRequest,
    ) -> Result<Repository, ArtifactsError> {
        let mut repos = self.repositories.write().await;
        if repos.values().any(|r| r.name == req.name) {
            return Err(ArtifactsError::AlreadyExists(format!(
                "repository '{}'",
                req.name
            )));
        }
        let mut repo = Repository::new(req.name, req.plugin_type);
        repo.description = req.description;
        repo.retained_versions = req.retained_versions;
        repos.insert(repo.pulp_href.clone(), repo.clone());
        Ok(repo)
    }

    pub async fn list_repositories(&self) -> Vec<Repository> {
        self.repositories.read().await.values().cloned().collect()
    }

    pub async fn get_repository(&self, href: &str) -> Option<Repository> {
        self.repositories.read().await.get(href).cloned()
    }

    pub async fn get_repository_by_name(&self, name: &str) -> Option<Repository> {
        self.repositories
            .read()
            .await
            .values()
            .find(|r| r.name == name)
            .cloned()
    }

    pub async fn delete_repository(&self, href: &str) -> Result<(), ArtifactsError> {
        let mut repos = self.repositories.write().await;
        if repos.remove(href).is_none() {
            return Err(ArtifactsError::NotFound(format!("repository {href}")));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Repository versions
    // -----------------------------------------------------------------------

    pub async fn create_repo_version(
        &self,
        repo_href: &str,
        content_hrefs: Vec<String>,
    ) -> Result<RepositoryVersion, ArtifactsError> {
        let mut repos = self.repositories.write().await;
        let repo = repos
            .get_mut(repo_href)
            .ok_or_else(|| ArtifactsError::NotFound(format!("repository {repo_href}")))?;

        let versions = self.repo_versions.read().await;
        let next_number = versions
            .values()
            .filter(|v| v.repository == repo_href)
            .map(|v| v.number)
            .max()
            .unwrap_or(0)
            + 1;
        drop(versions);

        let mut ver = RepositoryVersion::new(repo_href, next_number);
        ver.content_hrefs = content_hrefs.clone();

        // Summarise added content by type
        let content_store = self.content.read().await;
        for href in &content_hrefs {
            if let Some(unit) = content_store.get(href) {
                let label = format!("{}.{}", unit.plugin_type.api_segment(), unit.plugin_type.api_segment());
                *ver.content_summary.present.entry(label.clone()).or_insert(0) += 1;
                *ver.content_summary.added.entry(label).or_insert(0) += 1;
            }
        }

        let ver_href = ver.pulp_href.clone();
        repo.latest_version_href = Some(ver_href.clone());
        repo.updated_at = chrono::Utc::now();

        self.repo_versions.write().await.insert(ver_href, ver.clone());
        Ok(ver)
    }

    pub async fn list_repo_versions(&self, repo_href: &str) -> Vec<RepositoryVersion> {
        self.repo_versions
            .read()
            .await
            .values()
            .filter(|v| v.repository == repo_href)
            .cloned()
            .collect()
    }

    pub async fn get_repo_version(&self, href: &str) -> Option<RepositoryVersion> {
        self.repo_versions.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Remotes
    // -----------------------------------------------------------------------

    pub async fn create_remote(
        &self,
        req: CreateRemoteRequest,
    ) -> Result<Remote, ArtifactsError> {
        let mut remotes = self.remotes.write().await;
        if remotes.values().any(|r| r.name == req.name) {
            return Err(ArtifactsError::AlreadyExists(format!(
                "remote '{}'",
                req.name
            )));
        }
        let mut remote = Remote::new(req.name, req.plugin_type, req.url);
        if let Some(p) = req.download_policy {
            remote.download_policy = p;
        }
        remote.username = req.username;
        remote.password = req.password;
        if let Some(v) = req.tls_validation {
            remote.tls_validation = v;
        }
        remote.proxy_url = req.proxy_url;
        remotes.insert(remote.pulp_href.clone(), remote.clone());
        Ok(remote)
    }

    pub async fn list_remotes(&self) -> Vec<Remote> {
        self.remotes.read().await.values().cloned().collect()
    }

    pub async fn get_remote(&self, href: &str) -> Option<Remote> {
        self.remotes.read().await.get(href).cloned()
    }

    pub async fn delete_remote(&self, href: &str) -> Result<(), ArtifactsError> {
        let mut remotes = self.remotes.write().await;
        if remotes.remove(href).is_none() {
            return Err(ArtifactsError::NotFound(format!("remote {href}")));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Content units
    // -----------------------------------------------------------------------

    pub async fn store_content(&self, unit: ContentUnit) -> ContentUnit {
        let mut content = self.content.write().await;
        content.insert(unit.pulp_href.clone(), unit.clone());
        unit
    }

    pub async fn list_content(&self, plugin_type: Option<&PluginType>) -> Vec<ContentUnit> {
        self.content
            .read()
            .await
            .values()
            .filter(|u| plugin_type.map(|pt| &u.plugin_type == pt).unwrap_or(true))
            .cloned()
            .collect()
    }

    pub async fn get_content(&self, href: &str) -> Option<ContentUnit> {
        self.content.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Artifacts
    // -----------------------------------------------------------------------

    pub async fn store_artifact(&self, artifact: Artifact) -> Artifact {
        let mut artifacts = self.artifacts.write().await;
        artifacts.insert(artifact.pulp_href.clone(), artifact.clone());
        artifact
    }

    pub async fn get_artifact_data(&self, href: &str) -> Option<Vec<u8>> {
        self.artifacts
            .read()
            .await
            .get(href)
            .map(|a| a.data.clone())
    }

    // -----------------------------------------------------------------------
    // Publications
    // -----------------------------------------------------------------------

    pub async fn create_publication(
        &self,
        req: CreatePublicationRequest,
    ) -> Result<Publication, ArtifactsError> {
        let repo_ver_href = match (&req.repository_version, &req.repository) {
            (Some(v), _) => v.clone(),
            (None, Some(r)) => {
                let repos = self.repositories.read().await;
                repos
                    .get(r.as_str())
                    .and_then(|repo| repo.latest_version_href.clone())
                    .ok_or_else(|| {
                        ArtifactsError::NotFound(format!("no version for repository {r}"))
                    })?
            }
            (None, None) => {
                return Err(ArtifactsError::InvalidRequest(
                    "repository_version or repository is required".into(),
                ));
            }
        };

        let ver = self
            .get_repo_version(&repo_ver_href)
            .await
            .ok_or_else(|| ArtifactsError::NotFound(format!("version {repo_ver_href}")))?;

        let repo_opt = self.get_repository(&ver.repository).await;
        let plugin_type = repo_opt
            .map(|r| r.plugin_type)
            .ok_or_else(|| ArtifactsError::NotFound("parent repository".into()))?;

        let mut pub_ = Publication::new(plugin_type, repo_ver_href);
        pub_.repository = Some(ver.repository.clone());
        pub_.signing_service = req.signing_service;

        self.publications.write().await.insert(pub_.pulp_href.clone(), pub_.clone());
        Ok(pub_)
    }

    pub async fn list_publications(&self) -> Vec<Publication> {
        self.publications.read().await.values().cloned().collect()
    }

    pub async fn get_publication(&self, href: &str) -> Option<Publication> {
        self.publications.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Distributions
    // -----------------------------------------------------------------------

    pub async fn create_distribution(
        &self,
        req: CreateDistributionRequest,
    ) -> Result<Distribution, ArtifactsError> {
        let mut dists = self.distributions.write().await;
        if dists.values().any(|d| d.base_path == req.base_path) {
            return Err(ArtifactsError::AlreadyExists(format!(
                "base_path '{}' already in use",
                req.base_path
            )));
        }
        let mut dist = Distribution::new(req.name, req.plugin_type, req.base_path);
        dist.publication = req.publication;
        dist.repository = req.repository;
        dist.content_guard = req.content_guard;
        dists.insert(dist.pulp_href.clone(), dist.clone());
        Ok(dist)
    }

    pub async fn list_distributions(&self) -> Vec<Distribution> {
        self.distributions.read().await.values().cloned().collect()
    }

    pub async fn get_distribution(&self, href: &str) -> Option<Distribution> {
        self.distributions.read().await.get(href).cloned()
    }

    pub async fn get_distribution_by_base_path(&self, base_path: &str) -> Option<Distribution> {
        self.distributions
            .read()
            .await
            .values()
            .find(|d| d.base_path == base_path)
            .cloned()
    }

    pub async fn delete_distribution(&self, href: &str) -> Result<(), ArtifactsError> {
        let mut dists = self.distributions.write().await;
        if dists.remove(href).is_none() {
            return Err(ArtifactsError::NotFound(format!("distribution {href}")));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Content guards
    // -----------------------------------------------------------------------

    pub async fn create_content_guard(&self, guard: ContentGuard) -> ContentGuard {
        self.content_guards
            .write()
            .await
            .insert(guard.pulp_href.clone(), guard.clone());
        guard
    }

    pub async fn list_content_guards(&self) -> Vec<ContentGuard> {
        self.content_guards.read().await.values().cloned().collect()
    }

    pub async fn get_content_guard(&self, href: &str) -> Option<ContentGuard> {
        self.content_guards.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Signing services
    // -----------------------------------------------------------------------

    pub async fn create_signing_service(&self, svc: SigningService) -> SigningService {
        self.signing_services
            .write()
            .await
            .insert(svc.pulp_href.clone(), svc.clone());
        svc
    }

    pub async fn list_signing_services(&self) -> Vec<SigningService> {
        self.signing_services.read().await.values().cloned().collect()
    }

    pub async fn get_signing_service(&self, href: &str) -> Option<SigningService> {
        self.signing_services.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Tasks
    // -----------------------------------------------------------------------

    pub async fn enqueue_task(&self, task: Task) -> Task {
        self.tasks.write().await.insert(task.pulp_href.clone(), task.clone());
        task
    }

    pub async fn complete_task(
        &self,
        href: &str,
        created_resources: Vec<String>,
    ) -> Result<(), ArtifactsError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(href)
            .ok_or_else(|| ArtifactsError::NotFound(format!("task {href}")))?;
        task.state = TaskState::Completed;
        task.finished_at = Some(chrono::Utc::now());
        task.created_resources = created_resources;
        Ok(())
    }

    pub async fn fail_task(&self, href: &str, description: impl Into<String>) -> Result<(), ArtifactsError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(href)
            .ok_or_else(|| ArtifactsError::NotFound(format!("task {href}")))?;
        task.state = TaskState::Failed;
        task.finished_at = Some(chrono::Utc::now());
        task.error = Some(TaskError {
            description: description.into(),
            traceback: None,
        });
        Ok(())
    }

    pub async fn cancel_task(&self, href: &str) -> Result<Task, ArtifactsError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(href)
            .ok_or_else(|| ArtifactsError::NotFound(format!("task {href}")))?;
        if task.state == TaskState::Completed || task.state == TaskState::Failed {
            return Err(ArtifactsError::InvalidRequest(
                "cannot cancel a finished task".into(),
            ));
        }
        task.state = TaskState::Canceled;
        task.finished_at = Some(chrono::Utc::now());
        Ok(task.clone())
    }

    pub async fn list_tasks(&self) -> Vec<Task> {
        self.tasks.read().await.values().cloned().collect()
    }

    pub async fn get_task(&self, href: &str) -> Option<Task> {
        self.tasks.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Exporters
    // -----------------------------------------------------------------------

    pub async fn create_exporter(&self, exporter: Exporter) -> Exporter {
        self.exporters
            .write()
            .await
            .insert(exporter.pulp_href.clone(), exporter.clone());
        exporter
    }

    pub async fn list_exporters(&self) -> Vec<Exporter> {
        self.exporters.read().await.values().cloned().collect()
    }

    pub async fn get_exporter(&self, href: &str) -> Option<Exporter> {
        self.exporters.read().await.get(href).cloned()
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    pub async fn search_content(
        &self,
        name: Option<&str>,
        plugin_type: Option<&PluginType>,
        version: Option<&str>,
    ) -> Vec<ContentUnit> {
        let content = self.content.read().await;
        content
            .values()
            .filter(|u| {
                plugin_type.map(|pt| &u.plugin_type == pt).unwrap_or(true)
            })
            .filter(|u| {
                name.map(|n| {
                    u.metadata
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.contains(n))
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            })
            .filter(|u| {
                version.map(|ver| {
                    u.metadata
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s == ver)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Helpers for task-based operations
    // -----------------------------------------------------------------------

    /// Spawn a background task that immediately executes `f` then marks itself done.
    pub async fn run_as_task<F, Fut>(
        self: &Arc<Self>,
        name: impl Into<String>,
        reserved: Vec<String>,
        f: F,
    ) -> Task
    where
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Vec<String>, ArtifactsError>> + Send + 'static,
    {
        let task = Task::new(name, reserved);
        let task_href = task.pulp_href.clone();
        self.enqueue_task(task.clone()).await;

        let state = Arc::clone(self);
        tokio::spawn(async move {
            match f(Arc::clone(&state)).await {
                Ok(created) => {
                    let _ = state.complete_task(&task_href, created).await;
                }
                Err(e) => {
                    let _ = state.fail_task(&task_href, e.to_string()).await;
                }
            }
        });

        task
    }
}

impl Default for ArtifactsState {
    fn default() -> Self {
        Self::new()
    }
}
