// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lint dispatcher: content-type detection and per-type rule filtering.

use crate::models::{BatchLintRequest, BatchLintResult, ContentType, LintRequest, LintResult};
use crate::rules::{Category, LintRule, Violation};

/// Detect content type from filename and/or content.
pub fn detect_content_type(content: &str, filename: Option<&str>) -> ContentType {
    if let Some(name) = filename {
        let base = std::path::Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name);

        // Dockerfile detection
        if base == "Dockerfile"
            || base.starts_with("Dockerfile.")
            || name.ends_with("/Dockerfile")
        {
            return ContentType::Dockerfile;
        }

        // Docker Compose detection
        if base == "docker-compose.yml"
            || base == "docker-compose.yaml"
            || base.starts_with("docker-compose")
        {
            return ContentType::DockerCompose;
        }

        // Terraform
        if name.ends_with(".tf") || name.ends_with(".tfvars") {
            return ContentType::TerraformHcl;
        }

        // Kubernetes / Helm YAML
        if name.ends_with(".yaml") || name.ends_with(".yml") {
            if content.contains("apiVersion:") {
                if content.contains(".Values.") {
                    return ContentType::HelmChart;
                }
                return ContentType::KubernetesManifest;
            }
            if content.contains("services:") {
                return ContentType::DockerCompose;
            }
        }
    }

    // Fall back to content-based detection
    if content.contains("apiVersion:") {
        if content.contains(".Values.") {
            return ContentType::HelmChart;
        }
        return ContentType::KubernetesManifest;
    }
    if content.contains("services:") && content.contains("image:") {
        return ContentType::DockerCompose;
    }
    if content.contains("FROM ") && content.lines().any(|l| l.trim().starts_with("FROM ")) {
        return ContentType::Dockerfile;
    }

    ContentType::Dockerfile // default
}

/// Filter rules relevant to a given content type.
pub fn rules_for_type<'a>(rules: &'a [LintRule], content_type: &ContentType) -> Vec<&'a LintRule> {
    rules
        .iter()
        .filter(|r| match content_type {
            ContentType::Dockerfile => matches!(
                r.category,
                Category::Dockerfile | Category::Security | Category::BestPractice
            ) && !r.id.starts_with("K8S")
                && !r.id.starts_with("DC"),
            ContentType::KubernetesManifest | ContentType::HelmChart => {
                matches!(
                    r.category,
                    Category::Kubernetes | Category::Security | Category::BestPractice
                ) && !r.id.starts_with("DL")
                    && !r.id.starts_with("DC")
            }
            ContentType::DockerCompose => {
                matches!(
                    r.category,
                    Category::Compose | Category::Security | Category::BestPractice
                ) && !r.id.starts_with("DL")
                    && !r.id.starts_with("K8S")
            }
            ContentType::TerraformHcl => {
                // No Terraform-specific rules yet — skip all for now
                false
            }
        })
        .collect()
}

/// Run Dockerfile rules against content.
pub fn lint_dockerfile(content: &str, rules: &[LintRule]) -> Vec<Violation> {
    let applicable = rules_for_type(rules, &ContentType::Dockerfile);
    applicable
        .iter()
        .flat_map(|r| (r.check)(content))
        .collect()
}

/// Run Kubernetes rules against content.
pub fn lint_kubernetes(content: &str, rules: &[LintRule]) -> Vec<Violation> {
    let applicable = rules_for_type(rules, &ContentType::KubernetesManifest);
    applicable
        .iter()
        .flat_map(|r| (r.check)(content))
        .collect()
}

/// Run Docker Compose rules against content.
pub fn lint_compose(content: &str, rules: &[LintRule]) -> Vec<Violation> {
    let applicable = rules_for_type(rules, &ContentType::DockerCompose);
    applicable
        .iter()
        .flat_map(|r| (r.check)(content))
        .collect()
}

/// Lint a single request, dispatching to the right rule set.
pub fn lint(req: &LintRequest, rules: &[LintRule]) -> LintResult {
    let violations = match &req.content_type {
        ContentType::Dockerfile => lint_dockerfile(&req.content, rules),
        ContentType::KubernetesManifest | ContentType::HelmChart => {
            lint_kubernetes(&req.content, rules)
        }
        ContentType::DockerCompose => lint_compose(&req.content, rules),
        ContentType::TerraformHcl => vec![], // not yet implemented
    };
    LintResult::from_violations(violations, req.content_type.clone())
}

/// Lint multiple files, returning a combined result.
pub fn lint_batch(req: &BatchLintRequest, rules: &[LintRule]) -> BatchLintResult {
    let results: Vec<(String, LintResult)> = req
        .files
        .iter()
        .map(|file_req| {
            let name = file_req
                .filename
                .clone()
                .unwrap_or_else(|| "unnamed".to_string());
            let result = lint(file_req, rules);
            (name, result)
        })
        .collect();

    let total_errors: usize = results.iter().map(|(_, r)| r.total_errors).sum();
    let total_warnings: usize = results.iter().map(|(_, r)| r.total_warnings).sum();
    let passed = results.iter().all(|(_, r)| r.passed);

    BatchLintResult {
        results,
        total_errors,
        total_warnings,
        passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::builtin_rules;

    #[test]
    fn test_detect_dockerfile_by_name() {
        assert!(matches!(
            detect_content_type("FROM ubuntu\n", Some("Dockerfile")),
            ContentType::Dockerfile
        ));
    }

    #[test]
    fn test_detect_k8s_by_content() {
        assert!(matches!(
            detect_content_type("apiVersion: apps/v1\nkind: Deployment\n", None),
            ContentType::KubernetesManifest
        ));
    }

    #[test]
    fn test_detect_compose_by_name() {
        assert!(matches!(
            detect_content_type(
                "version: \"3\"\nservices:\n  web:\n    image: nginx\n",
                Some("docker-compose.yml")
            ),
            ContentType::DockerCompose
        ));
    }

    #[test]
    fn test_lint_dockerfile_applies_dockerfile_rules() {
        let rules = builtin_rules();
        let content = "FROM nginx:latest\nUSER root\n";
        let violations = lint_dockerfile(content, &rules);
        assert!(!violations.is_empty(), "Expected Dockerfile violations");
        // Should not include K8S or DC rules
        for v in &violations {
            assert!(
                !v.rule_id.starts_with("K8S") && !v.rule_id.starts_with("DC"),
                "Unexpected rule {} in Dockerfile lint",
                v.rule_id
            );
        }
    }

    #[test]
    fn test_lint_kubernetes_applies_k8s_rules() {
        let rules = builtin_rules();
        let content = "apiVersion: apps/v1\nkind: Deployment\nprivileged: true\n";
        let violations = lint_kubernetes(content, &rules);
        assert!(!violations.is_empty(), "Expected K8S violations");
    }

    #[test]
    fn test_lint_compose_applies_compose_rules() {
        let rules = builtin_rules();
        let content = "services:\n  web:\n    image: nginx\n";
        let violations = lint_compose(content, &rules);
        assert!(!violations.is_empty(), "Expected Compose violations");
    }

    #[test]
    fn test_lint_result_score_100_for_clean() {
        let result = LintResult::from_violations(vec![], ContentType::Dockerfile);
        assert_eq!(result.score, 100);
        assert!(result.passed);
    }

    #[test]
    fn test_lint_batch_aggregates() {
        let rules = builtin_rules();
        let req = BatchLintRequest {
            files: vec![
                LintRequest {
                    content: "FROM nginx:latest\n".into(),
                    content_type: ContentType::Dockerfile,
                    filename: Some("Dockerfile".into()),
                },
                LintRequest {
                    content: "apiVersion: apps/v1\nkind: Deployment\nprivileged: true\n".into(),
                    content_type: ContentType::KubernetesManifest,
                    filename: Some("deploy.yaml".into()),
                },
            ],
        };
        let batch = lint_batch(&req, &rules);
        assert_eq!(batch.results.len(), 2);
        assert!(batch.total_errors + batch.total_warnings > 0);
    }
}
