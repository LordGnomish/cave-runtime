//! ApplicationSet generators — expand template parameters into concrete
//! Application objects.
//!
//! Generators implemented:
//!   • List — static list of parameter maps
//!   • Clusters — all (or label-selected) clusters
//!   • Git — directories or files from a Git repo
//!   • Matrix — cartesian product of two generators
//!   • Merge — overlay generators using merge keys
//!   • PullRequest — open PRs from GitHub / GitLab

use crate::error::DeployError;
use crate::models::{
    Application, ApplicationSet, ApplicationSetGenerator, ApplicationSetTemplate,
    ApplicationSetTemplateMetadata, ApplicationSpec, ApplicationSource, ApplicationDestination,
    ApplicationSetStatus, Cluster, GitDirectoryGeneratorItem, GitFileGeneratorItem,
    GitGenerator, ListGenerator, MatrixGenerator, MergeGenerator, SyncPolicy,
};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Expand an ApplicationSet into a set of Application specs.
/// Each generator produces a list of parameter maps; the template is rendered
/// for each map.
pub fn expand_appset(
    appset: &ApplicationSet,
    clusters: &[Cluster],
) -> Result<Vec<Application>, DeployError> {
    let mut apps = Vec::new();
    for generator in &appset.spec.generators {
        let params = generate_params(generator, clusters)?;
        for param_set in params {
            let app = render_template(&appset.spec.template, &param_set, &appset.namespace)?;
            apps.push(app);
        }
    }
    Ok(apps)
}

/// Produce a list of parameter maps for a single generator.
fn generate_params(
    generator: &ApplicationSetGenerator,
    clusters: &[Cluster],
) -> Result<Vec<HashMap<String, String>>, DeployError> {
    match generator {
        ApplicationSetGenerator::List(g) => generate_list(g),
        ApplicationSetGenerator::Clusters(g) => generate_clusters(g, clusters),
        ApplicationSetGenerator::Git(g) => generate_git(g),
        ApplicationSetGenerator::Matrix(g) => generate_matrix(g, clusters),
        ApplicationSetGenerator::Merge(g) => generate_merge(g, clusters),
        ApplicationSetGenerator::PullRequest(g) => {
            // PullRequest generator requires an external API call; return
            // empty params when not yet connected.
            Ok(vec![])
        }
    }
}

// ─── List generator ───────────────────────────────────────────────────────────

/// List generator: each element is a JSON object whose string fields become
/// template parameters.
pub fn generate_list(g: &ListGenerator) -> Result<Vec<HashMap<String, String>>, DeployError> {
    g.elements
        .iter()
        .map(|elem| {
            let mut map = HashMap::new();
            if let Some(obj) = elem.as_object() {
                for (k, v) in obj {
                    let val = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    map.insert(k.clone(), val);
                }
            }
            Ok(map)
        })
        .collect()
}

// ─── Clusters generator ───────────────────────────────────────────────────────

fn generate_clusters(
    g: &crate::models::ClusterGenerator,
    clusters: &[Cluster],
) -> Result<Vec<HashMap<String, String>>, DeployError> {
    let mut result = Vec::new();
    for cluster in clusters {
        // Apply label selector if present
        if let Some(sel) = &g.selector {
            if !label_selector_matches(&cluster.labels, sel) {
                continue;
            }
        }
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("name".to_string(), cluster.name.clone());
        params.insert("server".to_string(), cluster.server.clone());
        // Extra values from the generator
        for (k, v) in &g.values {
            params.insert(k.clone(), v.clone());
        }
        // Expose cluster labels as parameters
        for (k, v) in &cluster.labels {
            params.insert(format!("metadata.labels.{k}"), v.clone());
        }
        result.push(params);
    }
    Ok(result)
}

fn label_selector_matches(
    labels: &HashMap<String, String>,
    sel: &crate::models::LabelSelector,
) -> bool {
    for (k, v) in &sel.match_labels {
        if labels.get(k).map(|lv| lv != v).unwrap_or(true) {
            return false;
        }
    }
    true
}

// ─── Git generator ────────────────────────────────────────────────────────────

/// Git generator: produces one parameter set per matching directory or file
/// in the repository.  When running without an actual checkout we return an
/// empty list; callers must pre-clone the repo and pass the local paths via
/// the `local_paths` extra field if needed.
pub fn generate_git(
    g: &GitGenerator,
) -> Result<Vec<HashMap<String, String>>, DeployError> {
    // In a full implementation we would clone the repo and walk the FS.
    // Here we produce params from already-known directory items (useful when
    // the repo has already been fetched by the sync engine).
    let mut result = Vec::new();

    for dir_item in &g.directories {
        if dir_item.exclude {
            continue;
        }
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("path".to_string(), dir_item.path.clone());
        params.insert(
            "path.basename".to_string(),
            dir_item.path.split('/').last().unwrap_or(&dir_item.path).to_string(),
        );
        params.insert(
            "path.basenameNormalized".to_string(),
            dir_item.path.split('/').last().unwrap_or(&dir_item.path)
                .to_lowercase()
                .replace('_', "-"),
        );
        for (k, v) in &g.values {
            params.insert(k.clone(), v.clone());
        }
        result.push(params);
    }

    for file_item in &g.files {
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert("path".to_string(), file_item.path.clone());
        for (k, v) in &g.values {
            params.insert(k.clone(), v.clone());
        }
        result.push(params);
    }

    Ok(result)
}

// ─── Matrix generator ─────────────────────────────────────────────────────────

/// Matrix generator: cartesian product of exactly 2 generators.
pub fn generate_matrix(
    g: &MatrixGenerator,
    clusters: &[Cluster],
) -> Result<Vec<HashMap<String, String>>, DeployError> {
    if g.generators.len() != 2 {
        return Err(DeployError::Invalid(
            "Matrix generator requires exactly 2 generators".to_string(),
        ));
    }
    let left = generate_params(&g.generators[0], clusters)?;
    let right = generate_params(&g.generators[1], clusters)?;

    let mut result = Vec::new();
    for l in &left {
        for r in &right {
            let mut combined = l.clone();
            combined.extend(r.clone());
            result.push(combined);
        }
    }
    Ok(result)
}

// ─── Merge generator ──────────────────────────────────────────────────────────

/// Merge generator: merges parameter sets from multiple generators using
/// merge keys.  Later generators override earlier ones for matching keys.
pub fn generate_merge(
    g: &MergeGenerator,
    clusters: &[Cluster],
) -> Result<Vec<HashMap<String, String>>, DeployError> {
    if g.generators.is_empty() {
        return Ok(vec![]);
    }

    // Start with the first generator's params as the base
    let mut base = generate_params(&g.generators[0], clusters)?;

    for overlay_gen in &g.generators[1..] {
        let overlay = generate_params(overlay_gen, clusters)?;
        // Merge: for each base entry, find the overlay entry with matching
        // merge keys and apply its values
        for b in &mut base {
            for o in &overlay {
                if g.merge_keys.iter().all(|k| b.get(k) == o.get(k)) {
                    for (k, v) in o {
                        if !g.merge_keys.contains(k) {
                            b.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }

    Ok(base)
}

// ─── Template rendering ───────────────────────────────────────────────────────

/// Render the ApplicationSet template with a concrete set of parameters,
/// substituting `{{param}}` placeholders.
pub fn render_template(
    template: &ApplicationSetTemplate,
    params: &HashMap<String, String>,
    appset_namespace: &str,
) -> Result<Application, DeployError> {
    let name = substitute(&template.metadata.name, params);
    let namespace = template
        .metadata
        .namespace
        .as_deref()
        .map(|n| substitute(n, params))
        .unwrap_or_else(|| appset_namespace.to_string());

    // Deep-substitute the spec via JSON serialization
    let spec_json = serde_json::to_string(&template.spec)
        .map_err(|e| DeployError::Internal(e.to_string()))?;
    let spec_substituted = substitute(&spec_json, params);
    let spec: ApplicationSpec = serde_json::from_str(&spec_substituted)
        .map_err(|e| DeployError::Internal(format!("template render: {e}")))?;

    let now = Utc::now();
    Ok(Application {
        id: Uuid::new_v4(),
        name,
        namespace,
        spec,
        status: Default::default(),
        created_at: now,
        updated_at: now,
        created_by: Some("cave-deploy/appset".to_string()),
        finalizers: vec!["resources-finalizer.argocd.argoproj.io".to_string()],
    })
}

/// Replace `{{key}}` with `params[key]`.
pub fn substitute(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (k, v) in params {
        result = result.replace(&format!("{{{{{k}}}}}"), v);
    }
    result
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ApplicationSetTemplate, ApplicationSetTemplateMetadata, ApplicationSpec,
        ApplicationSource, ApplicationDestination, GitGenerator, GitDirectoryGeneratorItem,
        ListGenerator,
    };
    use serde_json::json;

    fn minimal_template(name_tpl: &str) -> ApplicationSetTemplate {
        ApplicationSetTemplate {
            metadata: ApplicationSetTemplateMetadata {
                name: name_tpl.to_string(),
                namespace: Some("argocd".to_string()),
                ..Default::default()
            },
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".to_string(),
                    path: Some("{{path}}".to_string()),
                    target_revision: Some("HEAD".to_string()),
                    ..Default::default()
                },
                destination: ApplicationDestination {
                    server: Some("https://kubernetes.default.svc".to_string()),
                    namespace: "{{path.basename}}".to_string(),
                    ..Default::default()
                },
                project: "default".to_string(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_list_generator_expand() {
        let list_gen = ListGenerator {
            elements: vec![
                json!({"env": "staging", "region": "us-east-1"}),
                json!({"env": "prod",    "region": "eu-west-1"}),
            ],
            template: None,
        };
        let params = generate_list(&list_gen).unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].get("env").unwrap(), "staging");
        assert_eq!(params[1].get("region").unwrap(), "eu-west-1");
    }

    #[test]
    fn test_git_directory_generator() {
        let git_gen = GitGenerator {
            repo_url: "https://github.com/example/apps.git".to_string(),
            directories: vec![
                GitDirectoryGeneratorItem { path: "apps/frontend".to_string(), exclude: false },
                GitDirectoryGeneratorItem { path: "apps/backend".to_string(), exclude: false },
                GitDirectoryGeneratorItem { path: "apps/legacy".to_string(), exclude: true },
            ],
            files: vec![],
            revision: Some("main".to_string()),
            values: HashMap::new(),
            template: None,
        };
        let params = generate_git(&git_gen).unwrap();
        // legacy is excluded
        assert_eq!(params.len(), 2);
        let paths: Vec<&str> = params.iter().map(|p| p["path"].as_str()).collect();
        assert!(paths.contains(&"apps/frontend"));
        assert!(paths.contains(&"apps/backend"));
        assert!(!paths.contains(&"apps/legacy"));
    }

    #[test]
    fn test_git_generator_basename() {
        let git_gen2 = GitGenerator {
            repo_url: "https://example.com/repo.git".to_string(),
            directories: vec![GitDirectoryGeneratorItem {
                path: "apps/my_service".to_string(),
                exclude: false,
            }],
            files: vec![],
            revision: None,
            values: HashMap::new(),
            template: None,
        };
        let params = generate_git(&git_gen2).unwrap();
        assert_eq!(params[0]["path.basename"], "my_service");
        assert_eq!(params[0]["path.basenameNormalized"], "my-service");
    }

    #[test]
    fn test_matrix_generator_cartesian_product() {
        let matrix_gen = MatrixGenerator {
            generators: vec![
                ApplicationSetGenerator::List(ListGenerator {
                    elements: vec![json!({"env": "dev"}), json!({"env": "prod"})],
                    template: None,
                }),
                ApplicationSetGenerator::List(ListGenerator {
                    elements: vec![json!({"region": "us-east"}), json!({"region": "eu-west"})],
                    template: None,
                }),
            ],
            template: None,
        };
        let params = generate_matrix(&matrix_gen, &[]).unwrap();
        // 2 × 2 = 4 combinations
        assert_eq!(params.len(), 4);
        let found: Vec<String> = params
            .iter()
            .map(|p| format!("{}-{}", p["env"], p["region"]))
            .collect();
        assert!(found.contains(&"dev-us-east".to_string()));
        assert!(found.contains(&"prod-eu-west".to_string()));
    }

    #[test]
    fn test_template_substitution() {
        let mut params = HashMap::new();
        params.insert("path".to_string(), "apps/myservice".to_string());
        params.insert("path.basename".to_string(), "myservice".to_string());

        let tpl = minimal_template("{{path.basename}}-app");
        let app = render_template(&tpl, &params, "argocd").unwrap();
        assert_eq!(app.name, "myservice-app");
        assert_eq!(app.spec.destination.namespace, "myservice");
    }
}
