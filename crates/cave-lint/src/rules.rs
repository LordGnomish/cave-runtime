//! Linting rules for Dockerfiles and Kubernetes manifests.

use serde::Serialize;

#[derive(Clone)]
pub struct LintRule {
    pub id: &'static str,
    pub description: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub check: fn(&str) -> Vec<Violation>,
}

#[derive(Debug, Clone, Serialize)]
pub enum Category { Dockerfile, Kubernetes, Security, BestPractice }

#[derive(Debug, Clone, Serialize)]
pub enum Severity { Error, Warning, Info }

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule_id: String,
    pub message: String,
    pub line: Option<usize>,
    pub severity: Severity,
}

fn check_dockerfile_root(content: &str) -> Vec<Violation> {
    let mut v = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim().to_uppercase();
        if trimmed.starts_with("USER ROOT") || (trimmed.starts_with("USER") && trimmed.contains("0")) {
            v.push(Violation {
                rule_id: "DL3002".into(),
                message: "Do not run as root user".into(),
                line: Some(i + 1),
                severity: Severity::Warning,
            });
        }
    }
    v
}

fn check_dockerfile_latest(content: &str) -> Vec<Violation> {
    let mut v = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("FROM") && (trimmed.ends_with(":latest") || !trimmed.contains(':')) {
            v.push(Violation {
                rule_id: "DL3007".into(),
                message: "Pin image version instead of using latest".into(),
                line: Some(i + 1),
                severity: Severity::Warning,
            });
        }
    }
    v
}

fn check_k8s_no_limits(content: &str) -> Vec<Violation> {
    if content.contains("kind:") && !content.contains("limits:") {
        vec![Violation {
            rule_id: "K8S001".into(),
            message: "Container has no resource limits defined".into(),
            line: None,
            severity: Severity::Warning,
        }]
    } else {
        vec![]
    }
}

fn check_k8s_privileged(content: &str) -> Vec<Violation> {
    let mut v = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.contains("privileged: true") {
            v.push(Violation {
                rule_id: "K8S002".into(),
                message: "Container running in privileged mode".into(),
                line: Some(i + 1),
                severity: Severity::Error,
            });
        }
    }
    v
}

fn check_deprecated_apis(content: &str) -> Vec<Violation> {
    let deprecated = [
        ("extensions/v1beta1", "Use apps/v1 instead"),
        ("networking.k8s.io/v1beta1", "Use networking.k8s.io/v1"),
        ("policy/v1beta1", "Use policy/v1"),
        ("rbac.authorization.k8s.io/v1beta1", "Use rbac.authorization.k8s.io/v1"),
    ];
    let mut v = Vec::new();
    for (i, line) in content.lines().enumerate() {
        for (api, msg) in &deprecated {
            if line.contains(api) {
                v.push(Violation {
                    rule_id: "DEP001".into(),
                    message: format!("Deprecated API {api}: {msg}"),
                    line: Some(i + 1),
                    severity: Severity::Error,
                });
            }
        }
    }
    v
}

pub fn builtin_rules() -> Vec<LintRule> {
    vec![
        LintRule { id: "DL3002", description: "Do not run as root", category: Category::Dockerfile, severity: Severity::Warning, check: check_dockerfile_root },
        LintRule { id: "DL3007", description: "Pin image versions", category: Category::Dockerfile, severity: Severity::Warning, check: check_dockerfile_latest },
        LintRule { id: "K8S001", description: "Resource limits required", category: Category::Kubernetes, severity: Severity::Warning, check: check_k8s_no_limits },
        LintRule { id: "K8S002", description: "No privileged containers", category: Category::Security, severity: Severity::Error, check: check_k8s_privileged },
        LintRule { id: "DEP001", description: "Deprecated K8s APIs", category: Category::Kubernetes, severity: Severity::Error, check: check_deprecated_apis },
    ]
}

pub fn lint(content: &str, rules: &[LintRule]) -> Vec<Violation> {
    rules.iter().flat_map(|r| (r.check)(content)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dockerfile_root_detected() {
        let violations = check_dockerfile_root("USER root\n");
        assert!(!violations.is_empty(), "Expected violation for USER root");
    }

    #[test]
    fn test_dockerfile_root_not_detected() {
        let violations = check_dockerfile_root("USER appuser\n");
        assert!(violations.is_empty(), "Expected no violation for USER appuser");
    }

    #[test]
    fn test_dockerfile_latest_detected() {
        let violations = check_dockerfile_latest("FROM nginx:latest\n");
        assert!(!violations.is_empty(), "Expected violation for FROM nginx:latest");
    }

    #[test]
    fn test_dockerfile_latest_no_tag_detected() {
        let violations = check_dockerfile_latest("FROM nginx\n");
        assert!(!violations.is_empty(), "Expected violation for FROM nginx (no tag)");
    }

    #[test]
    fn test_dockerfile_latest_specific_version_ok() {
        let violations = check_dockerfile_latest("FROM nginx:1.21\n");
        assert!(violations.is_empty(), "Expected no violation for pinned version");
    }

    #[test]
    fn test_k8s_no_limits_detected() {
        let content = "kind: Deployment\nspec:\n  containers:\n  - name: app\n    image: nginx\n";
        let violations = check_k8s_no_limits(content);
        assert!(!violations.is_empty(), "Expected violation for missing resource limits");
    }

    #[test]
    fn test_k8s_no_limits_ok() {
        let content = "kind: Deployment\nspec:\n  containers:\n  - resources:\n      limits:\n        cpu: \"500m\"\n";
        let violations = check_k8s_no_limits(content);
        assert!(violations.is_empty(), "Expected no violation when limits are defined");
    }

    #[test]
    fn test_k8s_privileged_detected() {
        let violations = check_k8s_privileged("privileged: true\n");
        assert!(!violations.is_empty(), "Expected violation for privileged: true");
    }

    #[test]
    fn test_k8s_privileged_false_ok() {
        let violations = check_k8s_privileged("privileged: false\n");
        assert!(violations.is_empty(), "Expected no violation for privileged: false");
    }

    #[test]
    fn test_deprecated_api_detected() {
        let violations = check_deprecated_apis("apiVersion: extensions/v1beta1\n");
        assert!(!violations.is_empty(), "Expected violation for deprecated extensions/v1beta1");
    }

    #[test]
    fn test_deprecated_api_new_version_ok() {
        let violations = check_deprecated_apis("apiVersion: apps/v1\n");
        assert!(violations.is_empty(), "Expected no violation for apps/v1");
    }

    #[test]
    fn test_lint_applies_all_rules() {
        // Content that triggers all 5 builtin rules
        let content = "FROM nginx:latest\nUSER root\napiVersion: extensions/v1beta1\nkind: Deployment\nprivileged: true\n";
        let rules = builtin_rules();
        let violations = lint(content, &rules);
        // Should have violations from multiple rules
        assert!(!violations.is_empty(), "Expected violations from lint");
        // Check we got violations from at least 3 different rules
        let unique_rule_ids: std::collections::HashSet<&str> = violations.iter().map(|v| v.rule_id.as_str()).collect();
        assert!(unique_rule_ids.len() >= 3, "Expected violations from at least 3 rules");
    }

    #[test]
    fn test_builtin_rules_count() {
        let rules = builtin_rules();
        assert!(rules.len() >= 5, "Expected at least 5 builtin rules, got {}", rules.len());
    }

    #[test]
    fn test_violation_has_rule_id() {
        let violations = check_dockerfile_root("USER root\n");
        assert!(!violations.is_empty());
        for v in &violations {
            assert!(!v.rule_id.is_empty(), "Violation rule_id should not be empty");
        }
    }
}
