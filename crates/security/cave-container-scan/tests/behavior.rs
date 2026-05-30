// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave-container-scan`, the sovereign
//! heuristic-scanner port compatible with Aqua Trivy v0.70.0
//! (https://github.com/aquasecurity/trivy @ `v0.70.0`, Apache-2.0).
//!
//! These tests close the portable-coverage gaps identified in the TDD audit
//! (`docs/audit/tdd/cave-container-scan-gaps.md`): the `ScanOrchestrator::run`
//! dispatch/dedup/no-scanner branches, the `SecretScanner` SEC-004 Shannon-entropy
//! detector driven through the public `scan` path, and the IaC misconfig rules
//! K8S-003 / K8S-004 / TF-002. Each assertion checks a concrete value derived
//! directly from the scanner source, not mere non-emptiness.

use cave_container_scan::engine::{ScanOrchestrator, Scanner};
use cave_container_scan::models::{
    Confidence, IacKind, ScanKind, ScanRequest, ScanStatus, ScanTarget, Severity,
};
use cave_container_scan::scanners::iac::IacScanner;
use cave_container_scan::scanners::secret::SecretScanner;

// ---------------------------------------------------------------------------
// engine::ScanOrchestrator::run — dispatch, dedup, and no-scanner → Failed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_dispatches_to_matching_scanner_and_completes() {
    // SecretScanner declares kind() == ScanKind::Secret, so a Secret request
    // is dispatched to it. The content carries a single AWS key (SEC-001), so
    // exactly one finding is produced and the status is Completed.
    let orch = ScanOrchestrator::new(vec![Box::new(SecretScanner)]);
    let req = ScanRequest {
        kind: ScanKind::Secret,
        target: ScanTarget::Content(b"export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE".to_vec()),
        options: Default::default(),
    };

    let result = orch.run(&req).await;

    assert_eq!(result.status, ScanStatus::Completed);
    assert_eq!(result.scanner_version, "0.1.0");
    assert!(result.findings.iter().any(|f| f.rule_id == "SEC-001"));
}

#[tokio::test]
async fn run_with_no_matching_scanner_fails_with_empty_findings() {
    // Only a Secret scanner is registered; an Image request matches nothing.
    // The `None` branch of run() yields (vec![], ScanStatus::Failed).
    let orch = ScanOrchestrator::new(vec![Box::new(SecretScanner)]);
    let req = ScanRequest {
        kind: ScanKind::Image,
        target: ScanTarget::ImageRef("docker.io/library/nginx:1.27".to_string()),
        options: Default::default(),
    };

    let result = orch.run(&req).await;

    assert_eq!(result.status, ScanStatus::Failed);
    assert!(result.findings.is_empty());
}

#[tokio::test]
async fn run_dedupes_findings_by_fingerprint() {
    // Two identical private-key blocks in one payload both fire SEC-003.
    // Finding::new derives fingerprint as "rule_id:title:severity", which is
    // identical for both, so dedupe_findings collapses them to a single entry.
    let orch = ScanOrchestrator::new(vec![Box::new(SecretScanner)]);
    let payload = b"-----BEGIN RSA PRIVATE KEY-----\n-----BEGIN RSA PRIVATE KEY-----\n".to_vec();
    let req = ScanRequest {
        kind: ScanKind::Secret,
        target: ScanTarget::Content(payload),
        options: Default::default(),
    };

    let result = orch.run(&req).await;

    assert_eq!(result.status, ScanStatus::Completed);
    let sec003: Vec<_> = result
        .findings
        .iter()
        .filter(|f| f.rule_id == "SEC-003")
        .collect();
    assert_eq!(sec003.len(), 1);
}

// ---------------------------------------------------------------------------
// scanners::secret::SecretScanner::scan — SEC-004 high-entropy branch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn secret_scan_flags_high_entropy_base64_line_as_sec004() {
    // 64-char line of all-distinct base64 chars → Shannon entropy 6.0 (>= 4.5),
    // length >= 40, base64-like → fires SEC-004 with Severity::High / Confidence::Medium.
    let scanner = SecretScanner;
    let line = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let req = ScanRequest {
        kind: ScanKind::Secret,
        target: ScanTarget::Content(line.as_bytes().to_vec()),
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    let sec004 = findings
        .iter()
        .find(|f| f.rule_id == "SEC-004")
        .expect("expected a SEC-004 high-entropy finding");
    assert_eq!(sec004.severity, Severity::High);
    assert_eq!(sec004.confidence, Confidence::Medium);
}

#[tokio::test]
async fn secret_scan_ignores_low_entropy_base64_line() {
    // 40 repeated 'a' chars: length >= 40 and base64-like, so it reaches the
    // entropy gate, but Shannon entropy is 0.0 (< 4.5) → no SEC-004 emitted.
    let scanner = SecretScanner;
    let line = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 40 chars
    assert_eq!(line.len(), 40);
    let req = ScanRequest {
        kind: ScanKind::Secret,
        target: ScanTarget::Content(line.as_bytes().to_vec()),
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    assert!(!findings.iter().any(|f| f.rule_id == "SEC-004"));
}

// ---------------------------------------------------------------------------
// scanners::iac::IacScanner::scan — K8S-003, K8S-004, TF-002 rules
// ---------------------------------------------------------------------------

#[tokio::test]
async fn iac_scan_flags_host_network_as_k8s003() {
    // `hostNetwork: true` substring fires K8S-003 (Severity::High, Confidence::Confirmed).
    let scanner = IacScanner;
    let content = "kind: Pod\nspec:\n  hostNetwork: true\n  securityContext:\n    runAsNonRoot: true\n";
    let req = ScanRequest {
        kind: ScanKind::Iac,
        target: ScanTarget::IacBundle {
            kind: IacKind::Kubernetes,
            content: content.to_string(),
        },
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    let k8s003 = findings
        .iter()
        .find(|f| f.rule_id == "K8S-003")
        .expect("expected K8S-003 host-network finding");
    assert_eq!(k8s003.severity, Severity::High);
    assert_eq!(k8s003.confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn iac_scan_flags_always_pull_latest_as_k8s004() {
    // Requires BOTH `imagePullPolicy: Always` AND `:latest` substrings → K8S-004
    // (Severity::Medium, Confidence::High).
    let scanner = IacScanner;
    let content = "kind: Pod\nspec:\n  securityContext: {}\n  containers:\n  - image: myapp:latest\n    imagePullPolicy: Always\n";
    let req = ScanRequest {
        kind: ScanKind::Iac,
        target: ScanTarget::IacBundle {
            kind: IacKind::Kubernetes,
            content: content.to_string(),
        },
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    let k8s004 = findings
        .iter()
        .find(|f| f.rule_id == "K8S-004")
        .expect("expected K8S-004 insecure-pull-policy finding");
    assert_eq!(k8s004.severity, Severity::Medium);
    assert_eq!(k8s004.confidence, Confidence::High);
}

#[tokio::test]
async fn iac_scan_does_not_flag_k8s004_without_latest_tag() {
    // `imagePullPolicy: Always` alone (pinned tag, no `:latest`) must NOT fire
    // K8S-004 — the rule is gated on the AND of both substrings.
    let scanner = IacScanner;
    let content = "kind: Pod\nspec:\n  securityContext: {}\n  containers:\n  - image: myapp:1.2.3\n    imagePullPolicy: Always\n";
    let req = ScanRequest {
        kind: ScanKind::Iac,
        target: ScanTarget::IacBundle {
            kind: IacKind::Kubernetes,
            content: content.to_string(),
        },
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    assert!(!findings.iter().any(|f| f.rule_id == "K8S-004"));
}

#[tokio::test]
async fn iac_scan_flags_open_ingress_as_tf002() {
    // Requires `0.0.0.0/0` AND (`ingress` OR `from_port`) → TF-002
    // (Severity::High, Confidence::High).
    let scanner = IacScanner;
    let content = r#"
resource "aws_security_group" "open" {
  ingress {
    from_port   = 22
    to_port     = 22
    cidr_blocks = ["0.0.0.0/0"]
  }
}
"#;
    let req = ScanRequest {
        kind: ScanKind::Iac,
        target: ScanTarget::IacBundle {
            kind: IacKind::Terraform,
            content: content.to_string(),
        },
        options: Default::default(),
    };

    let findings = scanner.scan(&req).await.unwrap();

    let tf002 = findings
        .iter()
        .find(|f| f.rule_id == "TF-002")
        .expect("expected TF-002 unrestricted-ingress finding");
    assert_eq!(tf002.severity, Severity::High);
    assert_eq!(tf002.confidence, Confidence::High);
}
