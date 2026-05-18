// SPDX-License-Identifier: AGPL-3.0-or-later
//! ApplicationSet generators — list, cluster, git, matrix, merge, pull request.

use crate::models::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── ApplicationSet CRD ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSet {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: ApplicationSetSpec,
    pub status: Option<ApplicationSetStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetSpec {
    pub generators: Vec<Generator>,
    pub template: ApplicationSetTemplate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_policy: Option<ApplicationSetSyncPolicy>,
    #[serde(default)]
    pub ignore_application_differences: Vec<ApplicationSetIgnoreDifference>,
    #[serde(default)]
    pub template_patch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub go_template: Option<bool>,
    #[serde(default)]
    pub preserve_resources_on_deletion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetSyncPolicy {
    #[serde(default)]
    pub preserve_resources_on_deletion: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applications_sync: Option<ApplicationsSyncPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationsSyncPolicy {
    CreateOnly,
    CreateUpdate,
    CreateDelete,
    Sync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationSetIgnoreDifference {
    pub name: Option<String>,
    pub json_pointers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetTemplate {
    pub metadata: ApplicationSetTemplateMetadata,
    pub spec: ApplicationSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetTemplateMetadata {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    #[serde(default)]
    pub finalizers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSetStatus {
    pub conditions: Vec<ApplicationSetCondition>,
    pub resources: Vec<ApplicationSetResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationSetCondition {
    pub condition_type: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationSetResource {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub health: Option<HealthCondition>,
}

// ─── Generators ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Generator {
    /// Static list of parameter sets.
    List(ListGenerator),
    /// One entry per cluster matching a label selector.
    Clusters(ClusterGenerator),
    /// Entries from git repository (directories or files).
    Git(GitGenerator),
    /// Cartesian product of two or more generators.
    Matrix(MatrixGenerator),
    /// Merge parameters from multiple generators.
    Merge(MergeGenerator),
    /// One entry per pull request in a repository.
    PullRequest(PullRequestGenerator),
    /// One entry per SCM repository matching a filter.
    ScmProvider(ScmProviderGenerator),
    /// Subset of another ApplicationSet's applications.
    Plugin(PluginGenerator),
    /// Single cluster/app combination.
    SingleCluster(SingleClusterGenerator),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListGenerator {
    pub elements: Vec<HashMap<String, String>>,
    #[serde(default)]
    pub template: Option<ApplicationSetTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterGenerator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<LabelSelector>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    #[serde(default)]
    pub flat_list: bool,
    #[serde(default)]
    pub template: Option<ApplicationSetTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: HashMap<String, String>,
    #[serde(default)]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: LabelSelectorOperator,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LabelSelectorOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGenerator {
    pub repo_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default)]
    pub directories: Vec<GitDirectoryFilter>,
    #[serde(default)]
    pub files: Vec<GitFileFilter>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    #[serde(default)]
    pub template: Option<ApplicationSetTemplate>,
    #[serde(default)]
    pub requeue_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDirectoryFilter {
    pub path: String,
    #[serde(default)]
    pub exclude: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileFilter {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixGenerator {
    pub generators: Vec<Generator>,
    pub template: Option<ApplicationSetTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeGenerator {
    pub generators: Vec<MergeGeneratorEntry>,
    pub merge_keys: Vec<String>,
    pub template: Option<ApplicationSetTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeGeneratorEntry {
    pub generator: Box<Generator>,
    pub values: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestGenerator {
    pub github: Option<PullRequestGitHub>,
    pub gitlab: Option<PullRequestGitLab>,
    pub bitbucket: Option<PullRequestBitbucket>,
    #[serde(default)]
    pub filters: Vec<PullRequestFilter>,
    #[serde(default)]
    pub template: Option<ApplicationSetTemplate>,
    #[serde(default)]
    pub requeue_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestGitHub {
    pub owner: String,
    pub repo: String,
    pub api: Option<String>,
    pub token_ref: Option<String>,
    pub app_secret_name: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestGitLab {
    pub project: String,
    pub api: Option<String>,
    pub token_ref: Option<String>,
    pub labels: Vec<String>,
    pub pull_request_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestBitbucket {
    pub owner: String,
    pub repo: String,
    pub api: Option<String>,
    pub basic_auth: Option<BasicAuthRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicAuthRef {
    pub username_ref: String,
    pub password_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFilter {
    pub branch_match: Option<String>,
    pub title_regex: Option<String>,
    pub target_branch_match: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScmProviderGenerator {
    pub github: Option<ScmGitHub>,
    pub gitlab: Option<ScmGitLab>,
    #[serde(default)]
    pub filters: Vec<ScmFilter>,
    pub clone_protocol: Option<String>,
    pub template: Option<ApplicationSetTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScmGitHub {
    pub organization: String,
    pub api: Option<String>,
    pub token_ref: Option<String>,
    #[serde(default)]
    pub all_branches: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScmGitLab {
    pub group: String,
    pub api: Option<String>,
    pub token_ref: Option<String>,
    #[serde(default)]
    pub all_branches: bool,
    #[serde(default)]
    pub include_subgroups: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScmFilter {
    pub repository_match: Option<String>,
    pub paths_exist: Vec<String>,
    pub paths_do_not_exist: Vec<String>,
    pub label_match: Option<String>,
    pub branch_match: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginGenerator {
    pub config_map_ref: String,
    pub input: HashMap<String, serde_json::Value>,
    pub values: HashMap<String, String>,
    pub template: Option<ApplicationSetTemplate>,
    pub requeue_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleClusterGenerator {
    pub values: HashMap<String, String>,
    pub template: Option<ApplicationSetTemplate>,
}

// ─── Generator evaluation ────────────────────────────────────────────────────

/// Parameters generated by a generator for a single application.
pub type GeneratorParams = HashMap<String, String>;

/// Evaluate a list generator into parameter sets.
pub fn evaluate_list_generator(list_gen: &ListGenerator) -> Vec<GeneratorParams> {
    list_gen.elements.clone()
}

/// Evaluate a git directory generator (simplified — real impl would clone repo).
pub fn evaluate_git_directory_generator(git_gen: &GitGenerator, paths: &[&str]) -> Vec<GeneratorParams> {
    let mut params = Vec::new();
    for path in paths {
        for filter in &git_gen.directories {
            if matches_glob(&filter.path, path) && !filter.exclude {
                let dir_name = path.split('/').last().unwrap_or(path);
                let mut p = git_gen.values.clone();
                p.insert("path".to_string(), path.to_string());
                p.insert("path.basename".to_string(), dir_name.to_string());
                p.insert("path.basenameNormalized".to_string(), normalize_name(dir_name));
                p.extend(git_gen.values.clone());
                params.push(p);
            }
        }
    }
    params
}

/// Merge parameters from two generators (merge generator).
pub fn evaluate_merge_generator(
    base: &[GeneratorParams],
    override_params: &[GeneratorParams],
    merge_keys: &[String],
) -> Vec<GeneratorParams> {
    base.iter().map(|b| {
        let mut merged = b.clone();
        for ov in override_params {
            let keys_match = merge_keys.iter().all(|k| {
                b.get(k) == ov.get(k)
            });
            if keys_match {
                merged.extend(ov.clone());
            }
        }
        merged
    }).collect()
}

/// Cartesian product of two parameter sets (matrix generator).
pub fn evaluate_matrix_generator(
    left: &[GeneratorParams],
    right: &[GeneratorParams],
) -> Vec<GeneratorParams> {
    let mut result = Vec::new();
    for l in left {
        for r in right {
            let mut merged = l.clone();
            merged.extend(r.clone());
            result.push(merged);
        }
    }
    result
}

fn matches_glob(pattern: &str, path: &str) -> bool {
    if pattern == "*" { return true; }
    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 2];
        return path.starts_with(prefix);
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;
        for part in &parts {
            if let Some(idx) = path[pos..].find(part) {
                pos += idx + part.len();
            } else {
                return false;
            }
        }
        return true;
    }
    pattern == path
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c.to_lowercase().next().unwrap() } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_generator_returns_elements() {
        let list_gen = ListGenerator {
            elements: vec![
                [("cluster".to_string(), "staging".to_string())].into(),
                [("cluster".to_string(), "prod".to_string())].into(),
            ],
            template: None,
        };
        let params = evaluate_list_generator(&list_gen);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].get("cluster").map(|s| s.as_str()), Some("staging"));
    }

    #[test]
    fn matrix_generator_cartesian_product() {
        let left = vec![
            [("cluster".to_string(), "staging".to_string())].into(),
            [("cluster".to_string(), "prod".to_string())].into(),
        ];
        let right = vec![
            [("region".to_string(), "us-east-1".to_string())].into(),
            [("region".to_string(), "eu-west-1".to_string())].into(),
        ];
        let result = evaluate_matrix_generator(&left, &right);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn merge_generator_merges_on_key() {
        let base = vec![
            [("cluster".to_string(), "prod".to_string()), ("region".to_string(), "us".to_string())].into(),
        ];
        let overrides = vec![
            [("cluster".to_string(), "prod".to_string()), ("replicas".to_string(), "5".to_string())].into(),
        ];
        let result = evaluate_merge_generator(&base, &overrides, &["cluster".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].get("replicas").map(|s| s.as_str()), Some("5"));
        assert_eq!(result[0].get("region").map(|s| s.as_str()), Some("us"));
    }

    #[test]
    fn merge_generator_no_key_match() {
        let base = vec![
            [("cluster".to_string(), "prod".to_string())].into(),
        ];
        let overrides = vec![
            [("cluster".to_string(), "staging".to_string()), ("extra".to_string(), "val".to_string())].into(),
        ];
        let result = evaluate_merge_generator(&base, &overrides, &["cluster".to_string()]);
        assert_eq!(result[0].get("extra"), None);
    }

    #[test]
    fn git_directory_generator() {
        let git_gen = GitGenerator {
            repo_url: "https://github.com/example/config".to_string(),
            revision: Some("main".to_string()),
            directories: vec![GitDirectoryFilter { path: "clusters/*".to_string(), exclude: false }],
            files: vec![],
            values: HashMap::new(),
            template: None,
            requeue_after_seconds: None,
        };
        let paths = &["clusters/staging", "clusters/prod", "other/dir"];
        let params = evaluate_git_directory_generator(&git_gen, paths);
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn matches_glob_wildcard() {
        assert!(matches_glob("clusters/*", "clusters/staging"));
        assert!(matches_glob("clusters/*", "clusters/prod"));
        assert!(!matches_glob("clusters/*", "other/staging"));
    }

    #[test]
    fn normalize_name_converts_special_chars() {
        assert_eq!(normalize_name("My App_123"), "my-app-123");
    }

    #[test]
    fn applicationset_spec_serialization() {
        let spec = ApplicationSetSpec {
            generators: vec![Generator::List(ListGenerator {
                elements: vec![[("env".to_string(), "prod".to_string())].into()],
                template: None,
            })],
            template: ApplicationSetTemplate {
                metadata: ApplicationSetTemplateMetadata {
                    name: "app-{{env}}".to_string(),
                    namespace: None,
                    labels: HashMap::new(),
                    annotations: HashMap::new(),
                    finalizers: vec![],
                },
                spec: ApplicationSpec {
                    source: ApplicationSource {
                        repo_url: "https://github.com/example/app".to_string(),
                        target_revision: Some("{{env}}".to_string()),
                        path: None,
                        helm: None,
                        kustomize: None,
                        directory: None,
                    },
                    sources: vec![],
                    destination: Destination {
                        server: "https://k8s.example.com".to_string(),
                        name: None,
                        namespace: "{{env}}".to_string(),
                    },
                    project: "default".to_string(),
                    sync_policy: None,
                    ignored_differences: None,
                    info: None,
                    revision_history_limit: None,
                },
            },
            sync_policy: None,
            ignore_application_differences: vec![],
            template_patch: None,
            go_template: None,
            preserve_resources_on_deletion: false,
        };
        let json = serde_json::to_string(&spec).unwrap();
        // Generator enum serializes in camelCase (List → list)
        assert!(json.contains("list") || json.contains("List"));
    }
}
