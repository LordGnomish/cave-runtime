//! CAVE Artifacts — Pulp-compatible artifact repository.
//!
//! Upstream: [Pulp Project](https://pulpproject.org/)
//!
//! Architecture mirrors Pulp's plugin system:
//! - **Plugins**: pulp_file, pulp_python, pulp_rpm, pulp_deb, pulp_container, pulp_ansible, pulp_maven
//! - **Workflow**: Repository → Content → Publication → Distribution
//! - **Remotes**: sync from upstream with immediate / on-demand / streamed download policies
//! - **Tasks**: async task queue for sync, publish, modify operations
//! - **Content Guards**: restrict access to distributions
//! - **Signing Services**: GPG sign repository metadata
//! - **Import/Export**: air-gapped environment support

pub mod error;
pub mod export;
pub mod models;
pub mod plugin;
pub mod plugins;
pub mod publication;
pub mod routes;
pub mod signing;
pub mod store;
pub mod sync;

pub use error::ArtifactsError;
pub use models::*;
pub use store::ArtifactsState;

use axum::Router;
use std::sync::Arc;

pub const MODULE_NAME: &str = "artifacts";

pub fn router(state: Arc<ArtifactsState>) -> Router {
    routes::create_router(state)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn state() -> Arc<ArtifactsState> {
        Arc::new(ArtifactsState::new())
    }

    // ------------------------------------------------------------------
    // 1. Create a repository
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_create_repository() {
        let s = state();
        let repo = s
            .create_repository(CreateRepositoryRequest {
                name: "my-python-repo".into(),
                plugin_type: PluginType::Python,
                description: Some("Test repo".into()),
                retained_versions: Some(10),
            })
            .await
            .unwrap();
        assert_eq!(repo.name, "my-python-repo");
        assert_eq!(repo.plugin_type, PluginType::Python);
        assert!(repo.pulp_href.contains("/pulp/api/v3/repositories/python/"));
        assert_eq!(repo.retained_versions, Some(10));
    }

    // ------------------------------------------------------------------
    // 2. Duplicate repository name returns AlreadyExists
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_duplicate_repository_rejected() {
        let s = state();
        let req = || CreateRepositoryRequest {
            name: "dup-repo".into(),
            plugin_type: PluginType::File,
            description: None,
            retained_versions: None,
        };
        s.create_repository(req()).await.unwrap();
        let err = s.create_repository(req()).await.unwrap_err();
        assert!(matches!(err, ArtifactsError::AlreadyExists(_)));
    }

    // ------------------------------------------------------------------
    // 3. List repositories
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_list_repositories() {
        let s = state();
        for i in 0..3 {
            s.create_repository(CreateRepositoryRequest {
                name: format!("repo-{i}"),
                plugin_type: PluginType::File,
                description: None,
                retained_versions: None,
            })
            .await
            .unwrap();
        }
        assert_eq!(s.list_repositories().await.len(), 3);
    }

    // ------------------------------------------------------------------
    // 4. Delete repository
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_delete_repository() {
        let s = state();
        let repo = s
            .create_repository(CreateRepositoryRequest {
                name: "to-delete".into(),
                plugin_type: PluginType::Rpm,
                description: None,
                retained_versions: None,
            })
            .await
            .unwrap();
        s.delete_repository(&repo.pulp_href).await.unwrap();
        assert!(s.get_repository(&repo.pulp_href).await.is_none());
    }

    // ------------------------------------------------------------------
    // 5. Create a remote
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_create_remote() {
        let s = state();
        let remote = s
            .create_remote(CreateRemoteRequest {
                name: "pypi-upstream".into(),
                plugin_type: PluginType::Python,
                url: "https://pypi.org/simple/".into(),
                download_policy: Some(DownloadPolicy::OnDemand),
                username: None,
                password: Some("secret".into()),
                tls_validation: Some(true),
                proxy_url: None,
            })
            .await
            .unwrap();
        assert_eq!(remote.name, "pypi-upstream");
        assert_eq!(remote.download_policy, DownloadPolicy::OnDemand);
        assert!(remote.pulp_href.contains("/pulp/api/v3/remotes/python/"));
    }

    // ------------------------------------------------------------------
    // 6. Store and retrieve a content unit
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_store_and_retrieve_content() {
        let s = state();
        let unit = ContentUnit::new(
            PluginType::File,
            serde_json::json!({ "name": "readme", "relative_path": "README.md" }),
        );
        let href = unit.pulp_href.clone();
        s.store_content(unit).await;
        let retrieved = s.get_content(&href).await.unwrap();
        assert_eq!(retrieved.plugin_type, PluginType::File);
    }

    // ------------------------------------------------------------------
    // 7. Create a repository version
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_create_repo_version() {
        let s = state();
        let repo = s
            .create_repository(CreateRepositoryRequest {
                name: "versioned-repo".into(),
                plugin_type: PluginType::File,
                description: None,
                retained_versions: None,
            })
            .await
            .unwrap();

        let unit = s
            .store_content(ContentUnit::new(
                PluginType::File,
                serde_json::json!({}),
            ))
            .await;

        let ver = s
            .create_repo_version(&repo.pulp_href, vec![unit.pulp_href.clone()])
            .await
            .unwrap();

        assert_eq!(ver.number, 1);
        assert_eq!(ver.content_hrefs.len(), 1);

        // Second version increments
        let ver2 = s
            .create_repo_version(&repo.pulp_href, vec![])
            .await
            .unwrap();
        assert_eq!(ver2.number, 2);
    }

    // ------------------------------------------------------------------
    // 8. Create publication
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_create_publication() {
        let s = state();
        let repo = s
            .create_repository(CreateRepositoryRequest {
                name: "pub-repo".into(),
                plugin_type: PluginType::Python,
                description: None,
                retained_versions: None,
            })
            .await
            .unwrap();
        let _ver = s.create_repo_version(&repo.pulp_href, vec![]).await.unwrap();

        let pub_ = s
            .create_publication(CreatePublicationRequest {
                repository: Some(repo.pulp_href.clone()),
                repository_version: None,
                signing_service: None,
            })
            .await
            .unwrap();

        assert_eq!(pub_.plugin_type, PluginType::Python);
        assert!(pub_.pulp_href.contains("/pulp/api/v3/publications/python/"));
    }

    // ------------------------------------------------------------------
    // 9. Create distribution
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_create_distribution() {
        let s = state();
        let dist = s
            .create_distribution(CreateDistributionRequest {
                name: "my-dist".into(),
                plugin_type: PluginType::Python,
                base_path: "internal/python".into(),
                publication: None,
                repository: None,
                content_guard: None,
            })
            .await
            .unwrap();
        assert_eq!(dist.base_path, "internal/python");
        assert!(dist.base_url.contains("internal/python"));
    }

    // ------------------------------------------------------------------
    // 10. Duplicate base_path rejected
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_duplicate_base_path_rejected() {
        let s = state();
        let make = || CreateDistributionRequest {
            name: "d".into(),
            plugin_type: PluginType::File,
            base_path: "shared/path".into(),
            publication: None,
            repository: None,
            content_guard: None,
        };
        s.create_distribution(make()).await.unwrap();
        assert!(matches!(
            s.create_distribution(make()).await.unwrap_err(),
            ArtifactsError::AlreadyExists(_)
        ));
    }

    // ------------------------------------------------------------------
    // 11. Search content by name
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_search_content_by_name() {
        let s = state();
        for name in ["requests", "flask", "django"] {
            s.store_content(ContentUnit::new(
                PluginType::Python,
                serde_json::json!({ "name": name, "version": "1.0.0" }),
            ))
            .await;
        }
        let results = s.search_content(Some("flask"), None, None).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata["name"], "flask");
    }

    // ------------------------------------------------------------------
    // 12. Search content by plugin type
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_search_content_by_plugin_type() {
        let s = state();
        s.store_content(ContentUnit::new(PluginType::Rpm, serde_json::json!({ "name": "bash" }))).await;
        s.store_content(ContentUnit::new(PluginType::Deb, serde_json::json!({ "name": "bash" }))).await;
        s.store_content(ContentUnit::new(PluginType::Python, serde_json::json!({ "name": "bash" }))).await;

        let rpms = s.search_content(None, Some(&PluginType::Rpm), None).await;
        assert_eq!(rpms.len(), 1);
        assert_eq!(rpms[0].plugin_type, PluginType::Rpm);
    }

    // ------------------------------------------------------------------
    // 13. Task lifecycle
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_task_lifecycle() {
        let s = state();
        let task = Task::new("test.task", vec![]);
        let href = task.pulp_href.clone();
        s.enqueue_task(task).await;

        assert_eq!(s.get_task(&href).await.unwrap().state, TaskState::Waiting);

        s.complete_task(&href, vec!["/pulp/api/v3/repositories/file/file/x/".into()])
            .await
            .unwrap();

        let done = s.get_task(&href).await.unwrap();
        assert_eq!(done.state, TaskState::Completed);
        assert_eq!(done.created_resources.len(), 1);
    }

    // ------------------------------------------------------------------
    // 14. Cancel a task
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_cancel_task() {
        let s = state();
        let task = Task::new("long.task", vec![]);
        let href = task.pulp_href.clone();
        s.enqueue_task(task).await;
        let canceled = s.cancel_task(&href).await.unwrap();
        assert_eq!(canceled.state, TaskState::Canceled);
    }

    // ------------------------------------------------------------------
    // 15. Content guard header check
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_content_guard_header() {
        let mut guard = ContentGuard::new("my-guard", ContentGuardType::Header);
        guard.header_name = Some("X-Token".into());
        guard.header_value = Some("secret123".into());

        let mut good_headers = std::collections::HashMap::new();
        good_headers.insert("X-Token".into(), "secret123".into());

        let mut bad_headers = std::collections::HashMap::new();
        bad_headers.insert("X-Token".into(), "wrong".into());

        assert!(guard.allows(&good_headers));
        assert!(!guard.allows(&bad_headers));
        assert!(!guard.allows(&std::collections::HashMap::new()));
    }

    // ------------------------------------------------------------------
    // 16. RBAC content guard always allows (delegated to cave-auth)
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_content_guard_rbac_always_passes() {
        let guard = ContentGuard::new("rbac-guard", ContentGuardType::Rbac);
        assert!(guard.allows(&std::collections::HashMap::new()));
    }

    // ------------------------------------------------------------------
    // 17. Plugin registry contains all 7 plugins
    // ------------------------------------------------------------------
    #[test]
    fn test_plugin_registry_default() {
        let registry = plugin::PluginRegistry::default();
        let mut types = registry.list();
        types.sort_by_key(|t| t.api_segment());
        assert_eq!(types.len(), 7);
        assert!(registry.get(&PluginType::File).is_some());
        assert!(registry.get(&PluginType::Maven).is_some());
        assert!(registry.get(&PluginType::Python).is_some());
        assert!(registry.get(&PluginType::Rpm).is_some());
        assert!(registry.get(&PluginType::Deb).is_some());
        assert!(registry.get(&PluginType::Container).is_some());
        assert!(registry.get(&PluginType::Ansible).is_some());
    }

    // ------------------------------------------------------------------
    // 18. run_as_task spawns and completes a task
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_run_as_task() {
        let s = state();
        let task = s
            .run_as_task("test.immediate", vec![], |_state| async {
                Ok(vec!["/pulp/api/v3/repositories/file/file/x/".to_string()])
            })
            .await;

        // Give the spawned task a moment to complete.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let done = s.get_task(&task.pulp_href).await.unwrap();
        assert_eq!(done.state, TaskState::Completed);
        assert_eq!(done.created_resources.len(), 1);
    }

    // ------------------------------------------------------------------
    // 19. Export: validate_import rejects empty path
    // ------------------------------------------------------------------
    #[test]
    fn test_export_validate_import_empty_path() {
        assert!(export::validate_import("").is_err());
        assert!(export::validate_import("/tmp/export").is_ok());
    }

    // ------------------------------------------------------------------
    // 20. Signing: stub returns error when script is empty
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_signing_empty_script_errors() {
        let svc = SigningService::new("test-svc", "pubkey", "");
        let result = signing::sign_metadata(&svc, b"data").await;
        assert!(result.is_err());
    }
}
