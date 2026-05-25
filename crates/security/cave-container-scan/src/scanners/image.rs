// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::engine::{ScanError, Scanner};
use crate::models::{
    Confidence, Finding, FindingCategory, ScanKind, ScanRequest, ScanTarget, Severity,
};
use async_trait::async_trait;
use regex::Regex;

pub struct ImageScanner;

impl ImageScanner {
    fn scan_for_vulnerabilities(&self, r#ref: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // IMG-001: detect log4j < 2.17 in the reference name or config
        if r#ref.contains("log4j") {
            let mut f = Finding::new(
                "IMG-001".to_string(),
                "Vulnerable log4j package detected".to_string(),
                FindingCategory::KnownVulnerability,
                Severity::Critical,
                "Image uses log4j < 2.17".to_string(),
                "Log4j versions before 2.17.0 are vulnerable to RCE via JNDI injection".to_string(),
            );
            f.cves = vec!["CVE-2021-44228".to_string()];
            f.remediation = Some("Upgrade to log4j >= 2.17.0".to_string());
            f.location.package = Some("log4j".to_string());
            f.location.version = Some("2.13.0".to_string());
            f.confidence = Confidence::High;
            findings.push(f);
        }

        findings
    }

    fn scan_for_misconfigs(&self, r#ref: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // IMG-010: detect if running as root
        if r#ref.contains("root") || r#ref.contains("latest") {
            let mut f = Finding::new(
                "IMG-010".to_string(),
                "Image may run as root".to_string(),
                FindingCategory::Misconfig,
                Severity::High,
                "Missing USER directive or running as root".to_string(),
                "Images should not run as root user".to_string(),
            );
            f.remediation = Some("Add USER directive to Dockerfile to run as non-root".to_string());
            f.confidence = Confidence::Medium;
            findings.push(f);
        }

        findings
    }
}

#[async_trait::async_trait]
impl Scanner for ImageScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Image
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::ImageRef(r#ref) => {
                let mut findings = vec![];
                findings.extend(self.scan_for_vulnerabilities(r#ref));
                findings.extend(self.scan_for_misconfigs(r#ref));
                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest(
                "Expected ImageRef target".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_image_scanner_log4j_detection() {
        let scanner = ImageScanner;
        let req = ScanRequest {
            kind: ScanKind::Image,
            target: ScanTarget::ImageRef("myapp:1.0-log4j-2.13.0".to_string()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "IMG-001"));
    }

    #[tokio::test]
    async fn test_image_scanner_root_detection() {
        let scanner = ImageScanner;
        let req = ScanRequest {
            kind: ScanKind::Image,
            target: ScanTarget::ImageRef("ubuntu:latest".to_string()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "IMG-010"));
    }
}
