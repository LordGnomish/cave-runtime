//! Reusable task catalog — built-in and user-defined tasks.

use crate::models::*;
use std::collections::HashMap;

pub struct TaskCatalog {
    /// name → TaskSpec
    tasks: HashMap<String, CatalogEntry>,
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub spec: TaskSpec,
    pub tags: Vec<String>,
}

impl TaskCatalog {
    pub fn new() -> Self {
        Self { tasks: HashMap::new() }
    }

    /// Load built-in catalog tasks.
    pub fn builtin() -> Self {
        let mut catalog = Self::new();
        catalog.register_all(builtin_tasks());
        catalog
    }

    pub fn register(&mut self, entry: CatalogEntry) {
        self.tasks.insert(entry.name.clone(), entry);
    }

    pub fn register_all(&mut self, entries: Vec<CatalogEntry>) {
        for e in entries {
            self.register(e);
        }
    }

    pub fn get(&self, name: &str) -> Option<&CatalogEntry> {
        self.tasks.get(name)
    }

    pub fn list(&self) -> Vec<&CatalogEntry> {
        let mut v: Vec<&CatalogEntry> = self.tasks.values().collect();
        v.sort_by_key(|e| &e.name);
        v
    }

    pub fn search(&self, query: &str) -> Vec<&CatalogEntry> {
        let q = query.to_lowercase();
        self.tasks
            .values()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }
}

fn builtin_tasks() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            name: "git-clone".to_string(),
            version: "0.9.0".to_string(),
            description: "Clone a git repository into a workspace.".to_string(),
            tags: vec!["git".to_string(), "source".to_string()],
            spec: TaskSpec {
                description: Some("Clones a git repository into the output workspace.".to_string()),
                params: vec![
                    ParamSpec { name: "url".to_string(), param_type: ParamType::String, description: Some("Repository URL".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "revision".to_string(), param_type: ParamType::String, description: Some("Branch/tag/SHA".to_string()), default: Some(ParamValue::String("main".to_string())), enum_values: None },
                    ParamSpec { name: "depth".to_string(), param_type: ParamType::String, description: Some("Clone depth (0=full)".to_string()), default: Some(ParamValue::String("1".to_string())), enum_values: None },
                    ParamSpec { name: "submodules".to_string(), param_type: ParamType::String, description: Some("Fetch submodules".to_string()), default: Some(ParamValue::String("true".to_string())), enum_values: None },
                ],
                workspaces: vec![
                    WorkspaceDeclaration { name: "output".to_string(), description: Some("Workspace for the cloned repo".to_string()), optional: false, mount_path: None, read_only: false },
                    WorkspaceDeclaration { name: "ssh-directory".to_string(), description: Some("SSH credentials".to_string()), optional: true, mount_path: Some("/root/.ssh".to_string()), read_only: true },
                    WorkspaceDeclaration { name: "basic-auth".to_string(), description: Some("HTTP basic auth credentials".to_string()), optional: true, mount_path: None, read_only: true },
                ],
                results: vec![
                    ResultSpec { name: "commit".to_string(), description: Some("The SHA of the cloned commit".to_string()), result_type: ParamType::String },
                    ResultSpec { name: "url".to_string(), description: Some("The URL of the cloned repo".to_string()), result_type: ParamType::String },
                    ResultSpec { name: "committer-date".to_string(), description: Some("Date of the HEAD commit".to_string()), result_type: ParamType::String },
                ],
                steps: vec![Step {
                    name: "clone".to_string(),
                    image: "cgr.dev/chainguard/git:root-2.39".to_string(),
                    command: None,
                    args: vec![],
                    env: vec![
                        EnvVar { name: "PARAM_URL".to_string(), value: Some("$(params.url)".to_string()), value_from: None },
                        EnvVar { name: "PARAM_REVISION".to_string(), value: Some("$(params.revision)".to_string()), value_from: None },
                        EnvVar { name: "PARAM_DEPTH".to_string(), value: Some("$(params.depth)".to_string()), value_from: None },
                    ],
                    volume_mounts: vec![
                        VolumeMount { name: "output".to_string(), mount_path: "/workspace/output".to_string(), sub_path: None, read_only: false },
                    ],
                    script: Some(r#"#!/usr/bin/env sh
set -eu
git clone --depth="${PARAM_DEPTH}" "${PARAM_URL}" /workspace/output
cd /workspace/output
git checkout "${PARAM_REVISION}"
RESULT_SHA="$(git rev-parse HEAD)"
printf '%s' "${RESULT_SHA}" > /tekton/results/commit
printf '%s' "${PARAM_URL}" > /tekton/results/url
"#.to_string()),
                    working_dir: None,
                    resources: None,
                    security_context: None,
                    timeout: None,
                    ref_: None,
                    results: vec![],
                }],
                sidecars: vec![],
                step_template: None,
            },
        },
        CatalogEntry {
            name: "buildah".to_string(),
            version: "0.6.0".to_string(),
            description: "Build and push a container image with buildah.".to_string(),
            tags: vec!["docker".to_string(), "image".to_string(), "oci".to_string()],
            spec: TaskSpec {
                description: Some("Builds a container image using buildah.".to_string()),
                params: vec![
                    ParamSpec { name: "IMAGE".to_string(), param_type: ParamType::String, description: Some("Reference of the image buildah will produce.".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "DOCKERFILE".to_string(), param_type: ParamType::String, description: Some("Path to the Dockerfile.".to_string()), default: Some(ParamValue::String("./Dockerfile".to_string())), enum_values: None },
                    ParamSpec { name: "CONTEXT".to_string(), param_type: ParamType::String, description: Some("Path to the directory to use as context.".to_string()), default: Some(ParamValue::String(".".to_string())), enum_values: None },
                    ParamSpec { name: "TLSVERIFY".to_string(), param_type: ParamType::String, description: Some("Verify the TLS on the registry endpoint.".to_string()), default: Some(ParamValue::String("true".to_string())), enum_values: None },
                    ParamSpec { name: "BUILD_EXTRA_ARGS".to_string(), param_type: ParamType::String, description: Some("Extra parameters passed for the build command.".to_string()), default: Some(ParamValue::String("".to_string())), enum_values: None },
                ],
                workspaces: vec![
                    WorkspaceDeclaration { name: "source".to_string(), description: Some("Workspace containing the source code.".to_string()), optional: false, mount_path: None, read_only: false },
                    WorkspaceDeclaration { name: "dockerconfig".to_string(), description: Some("Docker credentials.".to_string()), optional: true, mount_path: Some("/root/.docker".to_string()), read_only: true },
                ],
                results: vec![
                    ResultSpec { name: "IMAGE_DIGEST".to_string(), description: Some("Digest of the image just built.".to_string()), result_type: ParamType::String },
                    ResultSpec { name: "IMAGE_URL".to_string(), description: Some("URL of the image just built.".to_string()), result_type: ParamType::String },
                ],
                steps: vec![
                    Step {
                        name: "build-and-push".to_string(),
                        image: "quay.io/buildah/stable:v1.33".to_string(),
                        command: None,
                        args: vec![],
                        env: vec![],
                        volume_mounts: vec![
                            VolumeMount { name: "source".to_string(), mount_path: "/workspace/source".to_string(), sub_path: None, read_only: false },
                        ],
                        script: Some(r#"#!/usr/bin/env bash
set -eu
buildah bud \
  $(params.BUILD_EXTRA_ARGS) \
  --tls-verify=$(params.TLSVERIFY) \
  -f $(params.DOCKERFILE) \
  -t $(params.IMAGE) \
  $(params.CONTEXT)
buildah push \
  --tls-verify=$(params.TLSVERIFY) \
  --digestfile=/tmp/image-digest \
  $(params.IMAGE) \
  docker://$(params.IMAGE)
cat /tmp/image-digest | tee /tekton/results/IMAGE_DIGEST
echo -n "$(params.IMAGE)" | tee /tekton/results/IMAGE_URL
"#.to_string()),
                        working_dir: Some("/workspace/source".to_string()),
                        resources: None,
                        security_context: Some(SecurityContext {
                            run_as_user: Some(0),
                            run_as_group: None,
                            run_as_non_root: Some(false),
                            allow_privilege_escalation: Some(true),
                            read_only_root_filesystem: false,
                        }),
                        timeout: None,
                        ref_: None,
                        results: vec![],
                    },
                ],
                sidecars: vec![],
                step_template: None,
            },
        },
        CatalogEntry {
            name: "helm-upgrade-from-source".to_string(),
            version: "0.3.0".to_string(),
            description: "Deploy a Helm chart from source directory.".to_string(),
            tags: vec!["helm".to_string(), "deploy".to_string(), "kubernetes".to_string()],
            spec: TaskSpec {
                description: Some("Deploys a Helm chart from a source directory.".to_string()),
                params: vec![
                    ParamSpec { name: "release_name".to_string(), param_type: ParamType::String, description: Some("Helm release name.".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "release_namespace".to_string(), param_type: ParamType::String, description: Some("Namespace to deploy to.".to_string()), default: Some(ParamValue::String("default".to_string())), enum_values: None },
                    ParamSpec { name: "charts_dir".to_string(), param_type: ParamType::String, description: Some("Path to the chart directory.".to_string()), default: Some(ParamValue::String("./helm".to_string())), enum_values: None },
                    ParamSpec { name: "values_file".to_string(), param_type: ParamType::String, description: Some("Path to values file.".to_string()), default: Some(ParamValue::String("values.yaml".to_string())), enum_values: None },
                    ParamSpec { name: "wait".to_string(), param_type: ParamType::String, description: Some("Wait for rollout.".to_string()), default: Some(ParamValue::String("true".to_string())), enum_values: None },
                ],
                workspaces: vec![
                    WorkspaceDeclaration { name: "source".to_string(), description: None, optional: false, mount_path: None, read_only: true },
                    WorkspaceDeclaration { name: "kubeconfig".to_string(), description: Some("Kubeconfig for cluster access.".to_string()), optional: true, mount_path: Some("/root/.kube".to_string()), read_only: true },
                ],
                results: vec![],
                steps: vec![Step {
                    name: "helm-upgrade".to_string(),
                    image: "alpine/helm:3.14".to_string(),
                    command: None,
                    args: vec![],
                    env: vec![],
                    volume_mounts: vec![],
                    script: Some(r#"#!/usr/bin/env sh
set -eu
helm upgrade --install \
  $(params.release_name) \
  $(params.charts_dir) \
  --namespace $(params.release_namespace) \
  --create-namespace \
  -f $(params.values_file) \
  $([ "$(params.wait)" = "true" ] && echo "--wait" || echo "")
"#.to_string()),
                    working_dir: Some("/workspace/source".to_string()),
                    resources: None,
                    security_context: None,
                    timeout: None,
                    ref_: None,
                    results: vec![],
                }],
                sidecars: vec![],
                step_template: None,
            },
        },
        CatalogEntry {
            name: "trivy-scanner".to_string(),
            version: "0.2.0".to_string(),
            description: "Scan a container image for vulnerabilities using Trivy.".to_string(),
            tags: vec!["security".to_string(), "scan".to_string(), "vulnerabilities".to_string()],
            spec: TaskSpec {
                description: Some("Scans a container image using Trivy.".to_string()),
                params: vec![
                    ParamSpec { name: "IMAGE".to_string(), param_type: ParamType::String, description: Some("Image to scan.".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "SEVERITY".to_string(), param_type: ParamType::String, description: Some("Severity levels to report.".to_string()), default: Some(ParamValue::String("HIGH,CRITICAL".to_string())), enum_values: None },
                    ParamSpec { name: "EXIT_CODE".to_string(), param_type: ParamType::String, description: Some("Exit code on vuln found.".to_string()), default: Some(ParamValue::String("0".to_string())), enum_values: None },
                ],
                workspaces: vec![],
                results: vec![
                    ResultSpec { name: "SCAN_OUTPUT".to_string(), description: Some("Trivy scan output summary.".to_string()), result_type: ParamType::String },
                ],
                steps: vec![Step {
                    name: "trivy-scan".to_string(),
                    image: "aquasec/trivy:0.50".to_string(),
                    command: None,
                    args: vec![
                        "image".to_string(),
                        "--exit-code".to_string(), "$(params.EXIT_CODE)".to_string(),
                        "--severity".to_string(), "$(params.SEVERITY)".to_string(),
                        "--format".to_string(), "json".to_string(),
                        "$(params.IMAGE)".to_string(),
                    ],
                    env: vec![],
                    volume_mounts: vec![],
                    script: None,
                    working_dir: None,
                    resources: None,
                    security_context: None,
                    timeout: None,
                    ref_: None,
                    results: vec![],
                }],
                sidecars: vec![],
                step_template: None,
            },
        },
        CatalogEntry {
            name: "sonarqube-scanner".to_string(),
            version: "0.2.0".to_string(),
            description: "Static code analysis with SonarQube.".to_string(),
            tags: vec!["sonar".to_string(), "quality".to_string(), "sast".to_string()],
            spec: TaskSpec {
                description: Some("Runs SonarQube scanner for code quality analysis.".to_string()),
                params: vec![
                    ParamSpec { name: "SONAR_HOST_URL".to_string(), param_type: ParamType::String, description: Some("SonarQube server URL.".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "SONAR_PROJECT_KEY".to_string(), param_type: ParamType::String, description: Some("Project key.".to_string()), default: None, enum_values: None },
                    ParamSpec { name: "SOURCE_TO_SCAN".to_string(), param_type: ParamType::String, description: Some("Source path to scan.".to_string()), default: Some(ParamValue::String(".".to_string())), enum_values: None },
                ],
                workspaces: vec![
                    WorkspaceDeclaration { name: "source".to_string(), description: None, optional: false, mount_path: None, read_only: true },
                ],
                results: vec![],
                steps: vec![Step {
                    name: "sonar-scan".to_string(),
                    image: "sonarsource/sonar-scanner-cli:5".to_string(),
                    command: None,
                    args: vec![
                        "-Dsonar.host.url=$(params.SONAR_HOST_URL)".to_string(),
                        "-Dsonar.projectKey=$(params.SONAR_PROJECT_KEY)".to_string(),
                        "-Dsonar.sources=$(params.SOURCE_TO_SCAN)".to_string(),
                    ],
                    env: vec![
                        EnvVar { name: "SONAR_TOKEN".to_string(), value: None, value_from: Some(EnvVarSource::SecretKeyRef { name: "sonar-secret".to_string(), key: "token".to_string() }) },
                    ],
                    volume_mounts: vec![],
                    script: None,
                    working_dir: Some("/workspace/source".to_string()),
                    resources: None,
                    security_context: None,
                    timeout: None,
                    ref_: None,
                    results: vec![],
                }],
                sidecars: vec![],
                step_template: None,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_has_expected_tasks() {
        let catalog = TaskCatalog::builtin();
        assert!(catalog.get("git-clone").is_some());
        assert!(catalog.get("buildah").is_some());
        assert!(catalog.get("helm-upgrade-from-source").is_some());
        assert!(catalog.get("trivy-scanner").is_some());
        assert!(catalog.get("sonarqube-scanner").is_some());
    }

    #[test]
    fn catalog_search() {
        let catalog = TaskCatalog::builtin();
        let results = catalog.search("docker");
        assert!(!results.is_empty());
        // buildah has "docker" in tags
        assert!(results.iter().any(|e| e.name == "buildah"));
    }

    #[test]
    fn catalog_search_by_description() {
        let catalog = TaskCatalog::builtin();
        let results = catalog.search("vulnerabilit"); // matches both "vulnerability" and "vulnerabilities"
        assert!(results.iter().any(|e| e.name == "trivy-scanner"));
    }

    #[test]
    fn catalog_list_sorted() {
        let catalog = TaskCatalog::builtin();
        let list = catalog.list();
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn catalog_get_nonexistent() {
        let catalog = TaskCatalog::builtin();
        assert!(catalog.get("nonexistent-task").is_none());
    }

    #[test]
    fn git_clone_has_results() {
        let catalog = TaskCatalog::builtin();
        let entry = catalog.get("git-clone").unwrap();
        assert!(entry.spec.results.iter().any(|r| r.name == "commit"));
        assert!(entry.spec.results.iter().any(|r| r.name == "url"));
    }

    #[test]
    fn buildah_has_image_result() {
        let catalog = TaskCatalog::builtin();
        let entry = catalog.get("buildah").unwrap();
        assert!(entry.spec.results.iter().any(|r| r.name == "IMAGE_DIGEST"));
    }
}
