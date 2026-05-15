// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Misconfiguration scanning — Dockerfile, Kubernetes YAML, Terraform.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisconfigFinding {
    pub check_id: String,
    pub title: String,
    pub description: String,
    pub file_path: String,
    pub line_number: Option<usize>,
    pub severity: MisconfigSeverity,
    pub resource_type: String,
    pub resolution: String,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MisconfigSeverity {
    Low,
    Medium,
    High,
    Critical,
}

// ---------------------------------------------------------------------------
// Dockerfile checks
// ---------------------------------------------------------------------------

pub fn scan_dockerfile(content: &str, file_path: &str) -> Vec<MisconfigFinding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    let has_user_directive = lines.iter().any(|l| l.trim_start().starts_with("USER ")
        && !l.contains("USER root")
        && !l.contains("USER 0"));

    if !has_user_directive {
        findings.push(MisconfigFinding {
            check_id: "DS002".into(),
            title: "Image should not be run as root".into(),
            description: "Running containers as root provides full system access. Use a non-root USER directive.".into(),
            file_path: file_path.to_string(),
            line_number: None,
            severity: MisconfigSeverity::High,
            resource_type: "Dockerfile".into(),
            resolution: "Add 'USER nonroot' or create a dedicated user".into(),
            references: vec!["https://docs.docker.com/develop/develop-images/dockerfile_best-practices/#user".into()],
        });
    }

    // Check for USER root explicit
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("USER root") || t == "USER 0" {
            findings.push(MisconfigFinding {
                check_id: "DS002".into(),
                title: "Container runs as root".into(),
                description: "Explicitly setting USER root is a security risk.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::High,
                resource_type: "Dockerfile".into(),
                resolution: "Use a non-root user".into(),
                references: vec![],
            });
        }

        // ADD instead of COPY
        if t.starts_with("ADD ") {
            findings.push(MisconfigFinding {
                check_id: "DS005".into(),
                title: "ADD instead of COPY".into(),
                description: "ADD has extra capabilities (URL fetching, archive extraction) that are often unnecessary. Prefer COPY.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::Low,
                resource_type: "Dockerfile".into(),
                resolution: "Replace ADD with COPY unless you need URL fetch or tar extraction".into(),
                references: vec!["https://docs.docker.com/develop/develop-images/dockerfile_best-practices/#add-or-copy".into()],
            });
        }

        // Latest tag
        if t.starts_with("FROM ") && t.ends_with(":latest") {
            findings.push(MisconfigFinding {
                check_id: "DS013".into(),
                title: "Base image uses latest tag".into(),
                description: "Using 'latest' tag makes builds non-reproducible and can pull vulnerable images.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::Medium,
                resource_type: "Dockerfile".into(),
                resolution: "Pin the base image to a specific version tag or digest".into(),
                references: vec![],
            });
        }

        // FROM with no digest
        if t.starts_with("FROM ") && !t.contains("@sha256:") && !t.ends_with(":latest") {
            // Only warn if it contains a floating tag pattern
            if t.contains(":") {
                // pinned tag — ok
            } else if !t.contains("scratch") && !t.contains("AS ") {
                findings.push(MisconfigFinding {
                    check_id: "DS012".into(),
                    title: "Base image not pinned to digest".into(),
                    description: "Without a digest, the base image can change unexpectedly.".into(),
                    file_path: file_path.to_string(),
                    line_number: Some(i + 1),
                    severity: MisconfigSeverity::Low,
                    resource_type: "Dockerfile".into(),
                    resolution: "Pin the base image using its SHA256 digest".into(),
                    references: vec![],
                });
            }
        }

        // sudo usage
        if t.contains("sudo") {
            findings.push(MisconfigFinding {
                check_id: "DS015".into(),
                title: "sudo used in Dockerfile".into(),
                description: "Using sudo in a Dockerfile is a sign of privilege escalation risk.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::Medium,
                resource_type: "Dockerfile".into(),
                resolution: "Run as root during build, then switch to non-root USER, avoiding sudo".into(),
                references: vec![],
            });
        }

        // curl | bash / wget | sh pattern
        if (t.contains("curl") || t.contains("wget")) && (t.contains("| bash") || t.contains("| sh") || t.contains("|bash") || t.contains("|sh")) {
            findings.push(MisconfigFinding {
                check_id: "DS016".into(),
                title: "Remote script execution via curl/wget pipe".into(),
                description: "Executing remote scripts via curl|bash is a supply-chain risk.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::High,
                resource_type: "Dockerfile".into(),
                resolution: "Download scripts, verify checksums, then execute".into(),
                references: vec![],
            });
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Kubernetes YAML checks
// ---------------------------------------------------------------------------

pub fn scan_k8s_yaml(content: &str, file_path: &str) -> Vec<MisconfigFinding> {
    let mut findings = Vec::new();

    let Ok(docs) = serde_yaml::from_str::<serde_json::Value>(content) else {
        return findings;
    };

    let kind = docs.get("kind").and_then(|v| v.as_str()).unwrap_or_default();

    // Check containers (Pod, Deployment, DaemonSet, StatefulSet, Job, CronJob)
    let containers = find_containers(&docs);

    for (container, path) in &containers {
        let name = container.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");

        // Privileged container
        if container
            .pointer("/securityContext/privileged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            findings.push(MisconfigFinding {
                check_id: "KSV017".into(),
                title: format!("Container '{name}' runs as privileged"),
                description: "Privileged containers have all capabilities and can escape to the host.".into(),
                file_path: file_path.to_string(),
                line_number: None,
                severity: MisconfigSeverity::Critical,
                resource_type: kind.to_string(),
                resolution: "Set securityContext.privileged to false".into(),
                references: vec!["https://kubernetes.io/docs/concepts/security/pod-security-standards/".into()],
            });
        }

        // runAsRoot
        let run_as_user = container
            .pointer("/securityContext/runAsUser")
            .and_then(|v| v.as_u64());
        if run_as_user == Some(0) {
            findings.push(MisconfigFinding {
                check_id: "KSV001".into(),
                title: format!("Container '{name}' runs as root (uid 0)"),
                description: "Running as root increases the risk of container breakout.".into(),
                file_path: file_path.to_string(),
                line_number: None,
                severity: MisconfigSeverity::High,
                resource_type: kind.to_string(),
                resolution: "Set securityContext.runAsNonRoot: true".into(),
                references: vec![],
            });
        }

        // No resource limits
        if container.get("resources").is_none() {
            findings.push(MisconfigFinding {
                check_id: "KSV011".into(),
                title: format!("Container '{name}' has no resource limits"),
                description: "Without resource limits, a container can exhaust node resources.".into(),
                file_path: file_path.to_string(),
                line_number: None,
                severity: MisconfigSeverity::Low,
                resource_type: kind.to_string(),
                resolution: "Add resources.limits.cpu and resources.limits.memory".into(),
                references: vec![],
            });
        }

        // hostNetwork
        let _ = path; // suppress unused
    }

    // Pod-level host namespace checks
    if docs.pointer("/spec/hostNetwork").and_then(|v| v.as_bool()).unwrap_or(false) {
        findings.push(MisconfigFinding {
            check_id: "KSV009".into(),
            title: "Pod uses host network".into(),
            description: "hostNetwork: true grants access to host network interfaces.".into(),
            file_path: file_path.to_string(),
            line_number: None,
            severity: MisconfigSeverity::High,
            resource_type: kind.to_string(),
            resolution: "Set hostNetwork: false unless required".into(),
            references: vec![],
        });
    }

    if docs.pointer("/spec/hostPID").and_then(|v| v.as_bool()).unwrap_or(false) {
        findings.push(MisconfigFinding {
            check_id: "KSV008".into(),
            title: "Pod shares host PID namespace".into(),
            description: "hostPID: true allows the pod to see all processes on the host.".into(),
            file_path: file_path.to_string(),
            line_number: None,
            severity: MisconfigSeverity::High,
            resource_type: kind.to_string(),
            resolution: "Set hostPID: false".into(),
            references: vec![],
        });
    }

    findings
}

fn find_containers(doc: &serde_json::Value) -> Vec<(serde_json::Value, String)> {
    let mut results = Vec::new();
    // Try multiple paths
    let paths = [
        "/spec/containers",
        "/spec/initContainers",
        "/spec/template/spec/containers",
        "/spec/template/spec/initContainers",
        "/spec/jobTemplate/spec/template/spec/containers",
    ];
    for path in &paths {
        if let Some(arr) = doc.pointer(path).and_then(|v| v.as_array()) {
            for c in arr {
                results.push((c.clone(), path.to_string()));
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Terraform checks
// ---------------------------------------------------------------------------

pub fn scan_terraform(content: &str, file_path: &str) -> Vec<MisconfigFinding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();

        // S3 bucket public access
        if t.contains("acl") && (t.contains("\"public-read\"") || t.contains("\"public-read-write\"")) {
            findings.push(MisconfigFinding {
                check_id: "AVD-AWS-0086".into(),
                title: "S3 bucket has public ACL".into(),
                description: "S3 buckets should not be publicly accessible.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::Critical,
                resource_type: "aws_s3_bucket".into(),
                resolution: "Set ACL to private or use bucket policies".into(),
                references: vec!["https://docs.aws.amazon.com/AmazonS3/latest/userguide/acl-overview.html".into()],
            });
        }

        // Security group 0.0.0.0/0
        if t.contains("cidr_blocks") && t.contains("0.0.0.0/0") {
            findings.push(MisconfigFinding {
                check_id: "AVD-AWS-0107".into(),
                title: "Security group allows ingress from 0.0.0.0/0".into(),
                description: "Opening ingress to the entire internet is a security risk.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::High,
                resource_type: "aws_security_group".into(),
                resolution: "Restrict CIDR blocks to specific IP ranges".into(),
                references: vec![],
            });
        }

        // Hard-coded credentials
        if (t.contains("access_key") || t.contains("secret_key")) && t.contains("\"AKIA") {
            findings.push(MisconfigFinding {
                check_id: "AVD-GEN-0001".into(),
                title: "Hard-coded AWS credentials in Terraform".into(),
                description: "Credentials should not be hard-coded.".into(),
                file_path: file_path.to_string(),
                line_number: Some(i + 1),
                severity: MisconfigSeverity::Critical,
                resource_type: "provider".into(),
                resolution: "Use environment variables or IAM roles".into(),
                references: vec![],
            });
        }

        // Unencrypted S3 bucket
        if t.contains("resource \"aws_s3_bucket\"") {
            let block_end = lines[i..]
                .iter()
                .position(|l| l.trim() == "}")
                .map(|p| i + p)
                .unwrap_or(lines.len());
            let block = &lines[i..block_end];
            let has_encryption = block.iter().any(|l| l.contains("server_side_encryption"));
            if !has_encryption {
                findings.push(MisconfigFinding {
                    check_id: "AVD-AWS-0090".into(),
                    title: "S3 bucket not encrypted at rest".into(),
                    description: "S3 buckets should use server-side encryption.".into(),
                    file_path: file_path.to_string(),
                    line_number: Some(i + 1),
                    severity: MisconfigSeverity::High,
                    resource_type: "aws_s3_bucket".into(),
                    resolution: "Add aws_s3_bucket_server_side_encryption_configuration".into(),
                    references: vec![],
                });
            }
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dockerfile_root_user() {
        let content = "FROM ubuntu:22.04\nRUN apt-get install -y curl\n";
        let findings = scan_dockerfile(content, "Dockerfile");
        assert!(findings.iter().any(|f| f.check_id == "DS002"));
    }

    #[test]
    fn dockerfile_add_instruction() {
        let content = "FROM alpine:3.18\nADD . /app\nUSER nonroot\n";
        let findings = scan_dockerfile(content, "Dockerfile");
        assert!(findings.iter().any(|f| f.check_id == "DS005"));
    }

    #[test]
    fn dockerfile_latest_tag() {
        let content = "FROM node:latest\nUSER node\n";
        let findings = scan_dockerfile(content, "Dockerfile");
        assert!(findings.iter().any(|f| f.check_id == "DS013"));
    }

    #[test]
    fn k8s_privileged_container() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: test
spec:
  containers:
  - name: myapp
    image: myapp:1.0
    securityContext:
      privileged: true
"#;
        let findings = scan_k8s_yaml(yaml, "pod.yaml");
        assert!(findings.iter().any(|f| f.check_id == "KSV017"));
    }

    #[test]
    fn terraform_public_s3() {
        let content = r#"resource "aws_s3_bucket" "data" {
  bucket = "my-bucket"
  acl    = "public-read"
}"#;
        let findings = scan_terraform(content, "main.tf");
        assert!(findings.iter().any(|f| f.check_id == "AVD-AWS-0086"));
    }

    #[test]
    fn terraform_open_sg() {
        let content = r#"resource "aws_security_group_rule" "ingress" {
  cidr_blocks = ["0.0.0.0/0"]
}"#;
        let findings = scan_terraform(content, "sg.tf");
        assert!(findings.iter().any(|f| f.check_id == "AVD-AWS-0107"));
    }
}
