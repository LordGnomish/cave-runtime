//! Task catalog — reusable task and pipeline definitions (like Tekton Hub).

use crate::models::{ParamType, ParameterSpec, ResultSpec, Step, Task, TaskType};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Catalog entry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub task: Task,
}

// ---------------------------------------------------------------------------
// Task catalog
// ---------------------------------------------------------------------------

pub struct TaskCatalog {
    entries: HashMap<String, CatalogEntry>,
}

impl Default for TaskCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskCatalog {
    pub fn new() -> Self {
        let mut c = Self { entries: HashMap::new() };
        c.register_builtins();
        c
    }

    fn register_builtins(&mut self) {
        for entry in [
            Self::git_clone_entry(),
            Self::docker_build_entry(),
            Self::run_tests_entry(),
            Self::deploy_k8s_entry(),
            Self::kaniko_build_entry(),
            Self::buildpacks_entry(),
        ] {
            self.register(entry);
        }
    }

    pub fn register(&mut self, entry: CatalogEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    pub fn get(&self, name: &str) -> Option<&CatalogEntry> {
        self.entries.get(name)
    }

    pub fn list(&self) -> Vec<&CatalogEntry> {
        let mut v: Vec<&CatalogEntry> = self.entries.values().collect();
        v.sort_by_key(|e| &e.name);
        v
    }

    // --- Built-in task definitions ---

    fn git_clone_entry() -> CatalogEntry {
        let mut task = Task::new("git-clone", TaskType::GitClone);
        task.params = vec![
            ParameterSpec {
                name: "url".to_string(),
                description: Some("Repository URL".to_string()),
                param_type: ParamType::String,
                default: None,
            },
            ParameterSpec {
                name: "revision".to_string(),
                description: Some("Branch, tag, or commit SHA".to_string()),
                param_type: ParamType::String,
                default: Some("main".to_string()),
            },
        ];
        task.steps = vec![Step {
            name: "clone".to_string(),
            image: Some("alpine/git:latest".to_string()),
            command: vec!["git".to_string(), "clone".to_string(), "--depth=1".to_string()],
            args: vec!["$(params.url)".to_string()],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(300),
            script: None,
        }];
        task.results = vec![ResultSpec {
            name: "commit-sha".to_string(),
            description: Some("The checked-out commit SHA".to_string()),
        }];
        CatalogEntry {
            name: "git-clone".to_string(),
            version: "0.9.0".to_string(),
            description: "Clone a git repository into the source workspace".to_string(),
            task,
        }
    }

    fn docker_build_entry() -> CatalogEntry {
        let mut task = Task::new("docker-build", TaskType::Build);
        task.params = vec![
            ParameterSpec {
                name: "image".to_string(),
                description: Some("Image name:tag to build".to_string()),
                param_type: ParamType::String,
                default: None,
            },
            ParameterSpec {
                name: "dockerfile".to_string(),
                description: Some("Path to Dockerfile".to_string()),
                param_type: ParamType::String,
                default: Some("Dockerfile".to_string()),
            },
        ];
        task.steps = vec![Step {
            name: "build-and-push".to_string(),
            image: Some("docker:24-dind".to_string()),
            command: vec!["docker".to_string(), "build".to_string()],
            args: vec![
                "-t".to_string(),
                "$(params.image)".to_string(),
                "-f".to_string(),
                "$(params.dockerfile)".to_string(),
                ".".to_string(),
            ],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(1800),
            script: None,
        }];
        CatalogEntry {
            name: "docker-build".to_string(),
            version: "2.0.0".to_string(),
            description: "Build and optionally push a Docker/OCI image".to_string(),
            task,
        }
    }

    fn run_tests_entry() -> CatalogEntry {
        let mut task = Task::new("run-tests", TaskType::Test);
        task.params = vec![ParameterSpec {
            name: "test-command".to_string(),
            description: Some("Test command to run".to_string()),
            param_type: ParamType::String,
            default: Some("cargo test".to_string()),
        }];
        task.steps = vec![Step {
            name: "test".to_string(),
            image: Some("rust:1.85".to_string()),
            command: vec![],
            args: vec![],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(600),
            script: Some("$(params.test-command)".to_string()),
        }];
        CatalogEntry {
            name: "run-tests".to_string(),
            version: "0.2.0".to_string(),
            description: "Run the project test suite".to_string(),
            task,
        }
    }

    fn deploy_k8s_entry() -> CatalogEntry {
        let mut task = Task::new("deploy-k8s", TaskType::Deploy);
        task.params = vec![ParameterSpec {
            name: "manifest".to_string(),
            description: Some("Path to k8s manifest or directory".to_string()),
            param_type: ParamType::String,
            default: Some("k8s/".to_string()),
        }];
        task.steps = vec![Step {
            name: "apply".to_string(),
            image: Some("bitnami/kubectl:latest".to_string()),
            command: vec!["kubectl".to_string(), "apply".to_string()],
            args: vec!["-f".to_string(), "$(params.manifest)".to_string()],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(300),
            script: None,
        }];
        CatalogEntry {
            name: "deploy-k8s".to_string(),
            version: "0.1.0".to_string(),
            description: "Apply Kubernetes manifests with kubectl".to_string(),
            task,
        }
    }

    fn kaniko_build_entry() -> CatalogEntry {
        let mut task = Task::new("kaniko-build", TaskType::Build);
        task.params = vec![ParameterSpec {
            name: "image".to_string(),
            description: Some("Destination image reference".to_string()),
            param_type: ParamType::String,
            default: None,
        }];
        task.steps = vec![Step {
            name: "build".to_string(),
            image: Some("gcr.io/kaniko-project/executor:latest".to_string()),
            command: vec!["/kaniko/executor".to_string()],
            args: vec!["--destination=$(params.image)".to_string()],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(1800),
            script: None,
        }];
        CatalogEntry {
            name: "kaniko-build".to_string(),
            version: "0.6.0".to_string(),
            description: "Build OCI images in-cluster with Kaniko (no Docker daemon)".to_string(),
            task,
        }
    }

    fn buildpacks_entry() -> CatalogEntry {
        let mut task = Task::new("buildpacks", TaskType::Build);
        task.params = vec![ParameterSpec {
            name: "image".to_string(),
            description: Some("Output image reference".to_string()),
            param_type: ParamType::String,
            default: None,
        }];
        task.steps = vec![Step {
            name: "build".to_string(),
            image: Some("paketobuildpacks/builder:base".to_string()),
            command: vec!["pack".to_string(), "build".to_string()],
            args: vec!["$(params.image)".to_string()],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(1800),
            script: None,
        }];
        CatalogEntry {
            name: "buildpacks".to_string(),
            version: "0.3.0".to_string(),
            description: "Build OCI images using Cloud Native Buildpacks".to_string(),
            task,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_contains_all_builtins() {
        let c = TaskCatalog::new();
        for name in ["git-clone", "docker-build", "run-tests", "deploy-k8s", "kaniko-build", "buildpacks"] {
            assert!(c.get(name).is_some(), "missing builtin: {name}");
        }
    }

    #[test]
    fn test_catalog_list_sorted() {
        let c = TaskCatalog::new();
        let names: Vec<&str> = c.list().iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn test_catalog_missing_entry() {
        let c = TaskCatalog::new();
        assert!(c.get("does-not-exist").is_none());
    }

    #[test]
    fn test_git_clone_task_type() {
        let c = TaskCatalog::new();
        let e = c.get("git-clone").unwrap();
        assert_eq!(e.task.task_type, TaskType::GitClone);
    }

    #[test]
    fn test_docker_build_has_image_param() {
        let c = TaskCatalog::new();
        let e = c.get("docker-build").unwrap();
        assert!(e.task.params.iter().any(|p| p.name == "image"));
    }

    #[test]
    fn test_git_clone_has_result() {
        let c = TaskCatalog::new();
        let e = c.get("git-clone").unwrap();
        assert!(e.task.results.iter().any(|r| r.name == "commit-sha"));
    }

    #[test]
    fn test_register_custom_task() {
        let mut c = TaskCatalog::new();
        let task = Task::new("my-custom-task", TaskType::Custom);
        c.register(CatalogEntry {
            name: "my-custom-task".to_string(),
            version: "1.0.0".to_string(),
            description: "A custom task".to_string(),
            task,
        });
        assert!(c.get("my-custom-task").is_some());
    }

    #[test]
    fn test_catalog_list_count() {
        let c = TaskCatalog::new();
        assert!(c.list().len() >= 6);
    }
}
