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
