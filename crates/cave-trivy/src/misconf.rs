// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Misconfiguration registry + built-in rules.
//!
//! Mirrors trivy's `pkg/iac/rules` + `pkg/iac/rego`. cave-trivy MVP ships
//! a curated set of built-in Rust rules across Terraform AWS/Azure/GCP,
//! Kubernetes manifest, Dockerfile and Helm. Custom Rego policies are a
//! scope cut handled by cave-policy.

use crate::models::Misconfiguration;
use crate::severity::Severity;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MisconfRule {
    pub id: &'static str,
    pub category: &'static str,
    pub r#type: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub severity: Severity,
    pub matcher: MisconfMatcher,
}

#[derive(Debug, Clone)]
pub enum MisconfMatcher {
    /// Substring on raw body.
    Substring(&'static str),
    /// Substring + presence of NOT-pattern.
    SubstringNotContains(&'static str, &'static str),
    /// YAML key present + value substring.
    YamlKeyContains(&'static str, &'static str),
}

#[derive(Debug, Clone, Default)]
pub struct MisconfRegistry {
    rules: Vec<MisconfRule>,
}

impl MisconfRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, r: MisconfRule) {
        self.rules.push(r);
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn rules_by_category(&self) -> HashMap<&'static str, Vec<&MisconfRule>> {
        let mut m: HashMap<&'static str, Vec<&MisconfRule>> = HashMap::new();
        for r in &self.rules {
            m.entry(r.category).or_default().push(r);
        }
        m
    }

    pub fn evaluate(&self, kind: &str, file: &str, body: &str) -> Vec<Misconfiguration> {
        let mut out = Vec::new();
        for r in self.rules_for(kind) {
            if matches(&r.matcher, body) {
                out.push(Misconfiguration {
                    id: r.id.into(),
                    r#type: r.r#type.into(),
                    title: r.title.into(),
                    description: r.description.into(),
                    severity: r.severity,
                    resource: file.into(),
                    references: vec![],
                });
            }
        }
        out
    }

    pub fn rules_for(&self, kind: &str) -> Vec<&MisconfRule> {
        self.rules
            .iter()
            .filter(|r| r.r#type.eq_ignore_ascii_case(kind))
            .collect()
    }

    pub fn builtin() -> Self {
        let mut r = Self::new();
        for x in builtin_rules() {
            r.push(x);
        }
        r
    }
}

fn matches(m: &MisconfMatcher, body: &str) -> bool {
    match m {
        MisconfMatcher::Substring(s) => body.contains(s),
        MisconfMatcher::SubstringNotContains(s, n) => body.contains(s) && !body.contains(n),
        MisconfMatcher::YamlKeyContains(k, v) => {
            for line in body.lines() {
                let l = line.trim();
                if let Some(stripped) = l.strip_prefix(&format!("{}:", k)) {
                    if stripped.trim().contains(v) {
                        return true;
                    }
                }
            }
            false
        }
    }
}

fn builtin_rules() -> Vec<MisconfRule> {
    vec![
        // Terraform AWS
        MisconfRule {
            id: "AVD-AWS-0086",
            category: "aws-s3",
            r#type: "terraform",
            title: "S3 bucket has public-read ACL",
            description: "Restrict bucket ACL to private and use bucket policy.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring(r#"acl = "public-read""#),
        },
        MisconfRule {
            id: "AVD-AWS-0017",
            category: "aws-cloudtrail",
            r#type: "terraform",
            title: "CloudTrail logging not encrypted",
            description: "Enable kms_key_id on CloudTrail.",
            severity: Severity::Medium,
            matcher: MisconfMatcher::SubstringNotContains(
                "resource \"aws_cloudtrail\"",
                "kms_key_id",
            ),
        },
        MisconfRule {
            id: "AVD-AWS-0028",
            category: "aws-iam",
            r#type: "terraform",
            title: "IAM policy allows wildcard action *",
            description: "Avoid \"Action: *\" in IAM policies.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring(r#"Action": "*""#),
        },
        // Terraform GCP
        MisconfRule {
            id: "AVD-GCP-0042",
            category: "gcp-iam",
            r#type: "terraform",
            title: "GCP service account has owner role",
            description: "Avoid roles/owner on service accounts.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring(r#"role = "roles/owner""#),
        },
        // Terraform Azure
        MisconfRule {
            id: "AVD-AZU-0042",
            category: "azure-storage",
            r#type: "terraform",
            title: "Azure storage allows public network access",
            description: "Set public_network_access_enabled=false.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring("public_network_access_enabled = true"),
        },
        // Kubernetes
        MisconfRule {
            id: "AVD-KSV-0001",
            category: "k8s-pod",
            r#type: "kubernetes",
            title: "Container runs as root",
            description: "Set runAsNonRoot: true.",
            severity: Severity::High,
            matcher: MisconfMatcher::YamlKeyContains("runAsUser", "0"),
        },
        MisconfRule {
            id: "AVD-KSV-0017",
            category: "k8s-pod",
            r#type: "kubernetes",
            title: "Privileged container",
            description: "Avoid privileged: true.",
            severity: Severity::Critical,
            matcher: MisconfMatcher::YamlKeyContains("privileged", "true"),
        },
        MisconfRule {
            id: "AVD-KSV-0014",
            category: "k8s-pod",
            r#type: "kubernetes",
            title: "Read-only root filesystem disabled",
            description: "Set readOnlyRootFilesystem: true.",
            severity: Severity::Medium,
            matcher: MisconfMatcher::YamlKeyContains("readOnlyRootFilesystem", "false"),
        },
        MisconfRule {
            id: "AVD-KSV-0044",
            category: "k8s-pod",
            r#type: "kubernetes",
            title: "Host network usage",
            description: "Containers should not use hostNetwork.",
            severity: Severity::High,
            matcher: MisconfMatcher::YamlKeyContains("hostNetwork", "true"),
        },
        MisconfRule {
            id: "AVD-KSV-0040",
            category: "k8s-rbac",
            r#type: "kubernetes",
            title: "ClusterRoleBinding to default service account",
            description: "Avoid binding ClusterRoles to default SAs.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring("name: default"),
        },
        // Dockerfile
        MisconfRule {
            id: "AVD-DS-0001",
            category: "docker",
            r#type: "dockerfile",
            title: "Use of ADD without checksum",
            description: "Prefer COPY or ADD with --checksum.",
            severity: Severity::Low,
            matcher: MisconfMatcher::Substring("ADD http"),
        },
        MisconfRule {
            id: "AVD-DS-0002",
            category: "docker",
            r#type: "dockerfile",
            title: "Container runs as root",
            description: "USER directive must drop privileges.",
            severity: Severity::Medium,
            matcher: MisconfMatcher::SubstringNotContains("FROM ", "USER "),
        },
        MisconfRule {
            id: "AVD-DS-0007",
            category: "docker",
            r#type: "dockerfile",
            title: "apt-get without --no-install-recommends",
            description: "Add --no-install-recommends to apt-get install.",
            severity: Severity::Low,
            matcher: MisconfMatcher::SubstringNotContains("apt-get install", "--no-install-recommends"),
        },
        // Helm
        MisconfRule {
            id: "AVD-HELM-0001",
            category: "helm",
            r#type: "helm",
            title: "Helm chart sets allowPrivilegeEscalation: true",
            description: "Set allowPrivilegeEscalation: false in Values.",
            severity: Severity::High,
            matcher: MisconfMatcher::Substring("allowPrivilegeEscalation: true"),
        },
        MisconfRule {
            id: "AVD-HELM-0002",
            category: "helm",
            r#type: "helm",
            title: "Helm values pin :latest image tag",
            description: "Pin imageTag to an immutable version.",
            severity: Severity::Medium,
            matcher: MisconfMatcher::Substring("image: \"latest\""),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_count() {
        let r = MisconfRegistry::builtin();
        assert!(r.len() >= 12);
    }

    #[test]
    fn tf_public_s3() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("terraform", "main.tf", r#"resource "aws_s3_bucket" "x" { acl = "public-read" }"#);
        assert!(f.iter().any(|m| m.id == "AVD-AWS-0086"));
    }

    #[test]
    fn k8s_privileged_pod() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("kubernetes", "pod.yaml", "spec:\n  containers:\n  - name: x\n    securityContext:\n      privileged: true\n");
        assert!(f.iter().any(|m| m.id == "AVD-KSV-0017"));
        assert!(f.iter().any(|m| m.severity == Severity::Critical));
    }

    #[test]
    fn k8s_runs_as_root() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("kubernetes", "p.yaml", "    runAsUser: 0\n");
        assert!(f.iter().any(|m| m.id == "AVD-KSV-0001"));
    }

    #[test]
    fn dockerfile_no_user() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("dockerfile", "Dockerfile", "FROM alpine\nRUN apk add x\n");
        assert!(f.iter().any(|m| m.id == "AVD-DS-0002"));
    }

    #[test]
    fn helm_latest_pin() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("helm", "values.yaml", "image: \"latest\"\n");
        assert!(f.iter().any(|m| m.id == "AVD-HELM-0002"));
    }

    #[test]
    fn substring_not_contains_negates() {
        let r = MisconfRegistry::builtin();
        let f = r.evaluate("dockerfile", "Dockerfile", "FROM alpine\nUSER 1000\nRUN apk add x\n");
        assert!(!f.iter().any(|m| m.id == "AVD-DS-0002"));
    }

    #[test]
    fn yaml_key_contains_matcher() {
        assert!(matches(
            &MisconfMatcher::YamlKeyContains("privileged", "true"),
            "    privileged: true\n"
        ));
        assert!(!matches(
            &MisconfMatcher::YamlKeyContains("privileged", "true"),
            "    privileged: false\n"
        ));
    }

    #[test]
    fn rules_by_category() {
        let r = MisconfRegistry::builtin();
        let map = r.rules_by_category();
        assert!(map.contains_key("aws-s3"));
        assert!(map.contains_key("k8s-pod"));
    }
}
