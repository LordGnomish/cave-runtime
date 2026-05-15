// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/<scanner>_test.go
// Source: gitleaks/gitleaks@9febafb config/gitleaks.toml
//! Integration tests for cave-scan IaC + secrets + license scanners.

use cave_scan::iac::{
    cloudformation::CloudFormationScanner, dockerfile::DockerfileScanner, helm::HelmScanner,
    kubernetes::KubernetesScanner, terraform::TerraformScanner, IacScanner, Severity as IacSev,
};
use cave_scan::license::{spdx::LicenseScanner, License};
use cave_scan::secrets::{entropy::shannon_entropy, patterns::SecretScanner};

// ── IaC: Terraform ──────────────────────────────────────────────────────────

#[test]
fn test_terraform_s3_public_acl() {
    let hcl = r#"
        resource "aws_s3_bucket" "public" {
          bucket = "my-public-bucket"
          acl    = "public-read"
        }
    "#;
    let s = TerraformScanner::new();
    let findings = s.scan_str(hcl, "main.tf").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-AWS-0001"));
}

#[test]
fn test_terraform_unencrypted_s3() {
    let hcl = r#"
        resource "aws_s3_bucket" "raw" {
          bucket = "no-crypto"
        }
    "#;
    let s = TerraformScanner::new();
    let findings = s.scan_str(hcl, "main.tf").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-AWS-0088"));
}

#[test]
fn test_terraform_security_group_open_world() {
    let hcl = r#"
        resource "aws_security_group" "lax" {
          name = "lax"
          ingress {
            from_port   = 22
            to_port     = 22
            protocol    = "tcp"
            cidr_blocks = ["0.0.0.0/0"]
          }
        }
    "#;
    let s = TerraformScanner::new();
    let findings = s.scan_str(hcl, "main.tf").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-AWS-0107"));
}

#[test]
fn test_terraform_iam_wildcard_action() {
    let hcl = r#"
        resource "aws_iam_policy" "broad" {
          name   = "broad"
          policy = "{\"Statement\":[{\"Effect\":\"Allow\",\"Action\":\"*\",\"Resource\":\"*\"}]}"
        }
    "#;
    let s = TerraformScanner::new();
    let findings = s.scan_str(hcl, "iam.tf").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-AWS-0057"));
}

#[test]
fn test_terraform_clean_file_no_findings() {
    let hcl = r#"
        resource "aws_s3_bucket" "ok" {
          bucket = "good-bucket"
          acl    = "private"
          server_side_encryption_configuration {
            rule { apply_server_side_encryption_by_default { sse_algorithm = "AES256" } }
          }
        }
    "#;
    let s = TerraformScanner::new();
    let findings = s.scan_str(hcl, "main.tf").unwrap();
    assert!(findings.iter().all(|f| f.rule_id != "AVD-AWS-0001"));
}

// ── IaC: Kubernetes ─────────────────────────────────────────────────────────

#[test]
fn test_k8s_privileged_container() {
    let y = r#"
apiVersion: v1
kind: Pod
metadata:
  name: nginx
spec:
  containers:
  - name: nginx
    image: nginx:1.25
    securityContext:
      privileged: true
"#;
    let s = KubernetesScanner::new();
    let findings = s.scan_str(y, "pod.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-KSV-0017"));
}

#[test]
fn test_k8s_run_as_root() {
    let y = r#"
apiVersion: v1
kind: Pod
metadata:
  name: nginx
spec:
  containers:
  - name: nginx
    image: nginx:1.25
"#;
    let s = KubernetesScanner::new();
    let findings = s.scan_str(y, "pod.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-KSV-0014"));
}

#[test]
fn test_k8s_latest_tag_warns() {
    let y = r#"
apiVersion: v1
kind: Pod
metadata:
  name: nginx
spec:
  containers:
  - name: nginx
    image: nginx:latest
"#;
    let s = KubernetesScanner::new();
    let findings = s.scan_str(y, "pod.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-KSV-0013"));
}

#[test]
fn test_k8s_host_network_blocked() {
    let y = r#"
apiVersion: v1
kind: Pod
spec:
  hostNetwork: true
  containers:
  - name: c
    image: x:1
"#;
    let s = KubernetesScanner::new();
    let findings = s.scan_str(y, "pod.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-KSV-0009"));
}

#[test]
fn test_k8s_no_resource_limits() {
    let y = r#"
apiVersion: v1
kind: Pod
spec:
  containers:
  - name: c
    image: x:1
"#;
    let s = KubernetesScanner::new();
    let findings = s.scan_str(y, "pod.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-KSV-0011"));
}

// ── IaC: Dockerfile ─────────────────────────────────────────────────────────

#[test]
fn test_dockerfile_user_root() {
    let df = "FROM alpine:3.19\nUSER root\nCMD [\"sh\"]\n";
    let s = DockerfileScanner::new();
    let findings = s.scan_str(df, "Dockerfile").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-DS-0002"));
}

#[test]
fn test_dockerfile_missing_user() {
    let df = "FROM alpine:3.19\nCMD [\"sh\"]\n";
    let s = DockerfileScanner::new();
    let findings = s.scan_str(df, "Dockerfile").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-DS-0002"));
}

#[test]
fn test_dockerfile_latest_tag() {
    let df = "FROM alpine:latest\nUSER nobody\n";
    let s = DockerfileScanner::new();
    let findings = s.scan_str(df, "Dockerfile").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-DS-0001"));
}

#[test]
fn test_dockerfile_add_remote_url() {
    let df = "FROM alpine:3.19\nUSER nobody\nADD https://example.com/x.tar /\n";
    let s = DockerfileScanner::new();
    let findings = s.scan_str(df, "Dockerfile").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-DS-0010"));
}

#[test]
fn test_dockerfile_curl_pipe_sh() {
    let df = "FROM alpine:3.19\nUSER nobody\nRUN curl https://x | sh\n";
    let s = DockerfileScanner::new();
    let findings = s.scan_str(df, "Dockerfile").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-DS-0027"));
}

// ── IaC: Helm ───────────────────────────────────────────────────────────────

#[test]
fn test_helm_chart_no_app_version() {
    let chart = r#"
apiVersion: v2
name: my-app
description: a chart
type: application
version: 0.1.0
"#;
    let s = HelmScanner::new();
    let findings = s.scan_str(chart, "Chart.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-HELM-0001"));
}

#[test]
fn test_helm_values_privileged_default() {
    let y = "securityContext:\n  privileged: true\n";
    let s = HelmScanner::new();
    let findings = s.scan_str(y, "values.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-HELM-0002"));
}

#[test]
fn test_helm_chart_clean() {
    let chart = r#"
apiVersion: v2
name: my-app
description: a chart
type: application
version: 0.1.0
appVersion: "1.16.0"
"#;
    let s = HelmScanner::new();
    let findings = s.scan_str(chart, "Chart.yaml").unwrap();
    assert!(findings.iter().all(|f| f.rule_id != "AVD-HELM-0001"));
}

// ── IaC: CloudFormation ─────────────────────────────────────────────────────

#[test]
fn test_cfn_s3_public_acl() {
    let y = r#"
Resources:
  Bucket:
    Type: AWS::S3::Bucket
    Properties:
      AccessControl: PublicRead
"#;
    let s = CloudFormationScanner::new();
    let findings = s.scan_str(y, "stack.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-CFN-0001"));
}

#[test]
fn test_cfn_security_group_open() {
    let y = r#"
Resources:
  SG:
    Type: AWS::EC2::SecurityGroup
    Properties:
      SecurityGroupIngress:
        - IpProtocol: tcp
          FromPort: 22
          ToPort: 22
          CidrIp: 0.0.0.0/0
"#;
    let s = CloudFormationScanner::new();
    let findings = s.scan_str(y, "stack.yaml").unwrap();
    assert!(findings.iter().any(|f| f.rule_id == "AVD-CFN-0002"));
}

#[test]
fn test_cfn_severity_high_only() {
    let y = r#"
Resources:
  Bucket:
    Type: AWS::S3::Bucket
    Properties:
      AccessControl: PublicReadWrite
"#;
    let s = CloudFormationScanner::new();
    let findings = s.scan_str(y, "stack.yaml").unwrap();
    let pub_finding = findings.iter().find(|f| f.rule_id == "AVD-CFN-0001").unwrap();
    assert_eq!(pub_finding.severity, IacSev::High);
}

// ── Secrets: regex patterns ────────────────────────────────────────────────

#[test]
fn test_secret_aws_access_key() {
    let s = SecretScanner::new();
    let content = "aws_access_key_id = AKIAIOSFODNN7EXAMPLE\n";
    let hits = s.scan(content, "creds.txt");
    assert!(hits.iter().any(|h| h.rule_id == "aws-access-key"));
}

#[test]
fn test_secret_github_pat() {
    let s = SecretScanner::new();
    let content = "token: ghp_abcdefghijklmnopqrstuvwxyzABCDEF0123\n";
    let hits = s.scan(content, "ci.yaml");
    assert!(hits.iter().any(|h| h.rule_id == "github-pat"));
}

#[test]
fn test_secret_slack_token() {
    let s = SecretScanner::new();
    let content = "SLACK=xoxb-12345-67890-AbCdEfGhIjKlMnOp\n";
    let hits = s.scan(content, "env");
    assert!(hits.iter().any(|h| h.rule_id == "slack-bot-token"));
}

#[test]
fn test_secret_stripe_secret() {
    let s = SecretScanner::new();
    let content = "STRIPE_KEY=sk_live_abcdefghijklmnopqrstuvwx\n";
    let hits = s.scan(content, "env");
    assert!(hits.iter().any(|h| h.rule_id == "stripe-secret-key"));
}

#[test]
fn test_secret_pem_private_key() {
    let s = SecretScanner::new();
    let content = "-----BEGIN RSA PRIVATE KEY-----\nABCDEFG\n-----END RSA PRIVATE KEY-----\n";
    let hits = s.scan(content, "id_rsa");
    assert!(hits.iter().any(|h| h.rule_id == "private-key"));
}

#[test]
fn test_secret_jwt() {
    let s = SecretScanner::new();
    let content = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyIn0.abc";
    let hits = s.scan(content, "log.txt");
    assert!(hits.iter().any(|h| h.rule_id == "jwt"));
}

#[test]
fn test_secret_clean_text_no_false_positive() {
    let s = SecretScanner::new();
    let hits = s.scan("hello world this is fine\n", "readme.md");
    assert!(hits.is_empty());
}

#[test]
fn test_entropy_high_for_random_base64() {
    let e = shannon_entropy("YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXowMTIz");
    assert!(e > 4.0);
}

#[test]
fn test_entropy_low_for_repeated_chars() {
    let e = shannon_entropy("aaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(e < 1.0);
}

#[test]
fn test_secret_pattern_count_at_least_40() {
    let s = SecretScanner::new();
    assert!(
        s.pattern_count() >= 40,
        "got {} patterns, expected >= 40",
        s.pattern_count()
    );
}

// ── License: SPDX detection ─────────────────────────────────────────────────

#[test]
fn test_license_mit_detection() {
    let s = LicenseScanner::new();
    let body = "MIT License\nCopyright (c) 2026\nPermission is hereby granted, free of charge...";
    let r = s.detect_from_text(body);
    assert!(r.iter().any(|l| l.spdx_id == "MIT"));
}

#[test]
fn test_license_apache_detection() {
    let s = LicenseScanner::new();
    let body = "Apache License\nVersion 2.0, January 2004\nhttp://www.apache.org/licenses/";
    let r = s.detect_from_text(body);
    assert!(r.iter().any(|l| l.spdx_id == "Apache-2.0"));
}

#[test]
fn test_license_gpl_flagged_copyleft() {
    let s = LicenseScanner::new();
    let body = "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007";
    let r = s.detect_from_text(body);
    let gpl: &License = r.iter().find(|l| l.spdx_id == "GPL-3.0").unwrap();
    assert!(gpl.is_copyleft);
}

#[test]
fn test_license_agpl_flagged_copyleft() {
    let s = LicenseScanner::new();
    let body = "GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3, 19 November 2007";
    let r = s.detect_from_text(body);
    let agpl: &License = r.iter().find(|l| l.spdx_id == "AGPL-3.0").unwrap();
    assert!(agpl.is_copyleft);
}

#[test]
fn test_license_spdx_id_in_source_header() {
    let s = LicenseScanner::new();
    let r = s.detect_from_text("// SPDX-License-Identifier: BSD-3-Clause\n");
    assert!(r.iter().any(|l| l.spdx_id == "BSD-3-Clause"));
}

#[test]
fn test_license_path_heuristic_license_file() {
    let s = LicenseScanner::new();
    assert!(s.is_license_path("LICENSE"));
    assert!(s.is_license_path("COPYING"));
    assert!(s.is_license_path("LICENSE.md"));
    assert!(s.is_license_path("license.txt"));
    assert!(!s.is_license_path("src/main.rs"));
}

#[test]
fn test_license_no_match_returns_empty() {
    let s = LicenseScanner::new();
    let r = s.detect_from_text("this is not a license document at all\njust random text\n");
    assert!(r.is_empty());
}
