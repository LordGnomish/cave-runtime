// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::engine::{ScanError, Scanner};
use crate::models::{
    Confidence, Finding, FindingCategory, ScanKind, ScanRequest, ScanTarget, Severity,
};
use async_trait::async_trait;
use regex::Regex;

pub struct SecretScanner;

impl SecretScanner {
    fn detect_secrets(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // AWS Access Key pattern: AKIA[0-9A-Z]{16}
        if let Ok(re) = Regex::new(r"AKIA[0-9A-Z]{16}") {
            if re.is_match(content) {
                let mut f = Finding::new(
                    "SEC-001".to_string(),
                    "AWS Access Key detected".to_string(),
                    FindingCategory::ExposedSecret,
                    Severity::Critical,
                    "AWS Access Key found in content".to_string(),
                    "Exposed AWS credentials can lead to account compromise".to_string(),
                );
                f.remediation = Some("Rotate the AWS access key immediately".to_string());
                f.confidence = Confidence::Confirmed;
                findings.push(f);
            }
        }

        // GitHub Token pattern: ghp_[A-Za-z0-9]{36}
        if let Ok(re) = Regex::new(r"ghp_[A-Za-z0-9]{36}") {
            if re.is_match(content) {
                let mut f = Finding::new(
                    "SEC-002".to_string(),
                    "GitHub Personal Access Token detected".to_string(),
                    FindingCategory::ExposedSecret,
                    Severity::Critical,
                    "GitHub PAT found in content".to_string(),
                    "Exposed GitHub tokens can lead to unauthorized repository access".to_string(),
                );
                f.remediation = Some("Revoke the GitHub token in account settings".to_string());
                f.confidence = Confidence::Confirmed;
                findings.push(f);
            }
        }

        // Private key pattern
        if let Ok(re) = Regex::new(r"-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----") {
            if re.is_match(content) {
                let mut f = Finding::new(
                    "SEC-003".to_string(),
                    "Private key material detected".to_string(),
                    FindingCategory::ExposedSecret,
                    Severity::Critical,
                    "Private key found in content".to_string(),
                    "Exposed private keys can be used for unauthorized access".to_string(),
                );
                f.remediation = Some("Rotate the private key and revoke access".to_string());
                f.confidence = Confidence::Confirmed;
                findings.push(f);
            }
        }

        // High-entropy base64 blobs (Shannon entropy >= 4.5)
        for line in content.lines() {
            if line.len() >= 40 && is_base64_like(line) {
                if shannon_entropy(line) >= 4.5 {
                    let mut f = Finding::new(
                        "SEC-004".to_string(),
                        "High-entropy secret candidate".to_string(),
                        FindingCategory::ExposedSecret,
                        Severity::High,
                        "Potential secret detected via entropy analysis".to_string(),
                        "String exhibits high entropy and may be a secret".to_string(),
                    );
                    f.remediation = Some("Review and remove if this is sensitive data".to_string());
                    f.confidence = Confidence::Medium;
                    findings.push(f);
                    break; // Only report once per content
                }
            }
        }

        findings
    }
}

fn is_base64_like(s: &str) -> bool {
    s.chars().all(|c| {
        (c >= 'A' && c <= 'Z')
            || (c >= 'a' && c <= 'z')
            || (c >= '0' && c <= '9')
            || c == '+'
            || c == '/'
            || c == '='
    })
}

fn shannon_entropy(s: &str) -> f64 {
    let mut freq = [0u32; 256];
    for byte in s.bytes() {
        freq[byte as usize] += 1;
    }

    let len = s.len() as f64;
    let mut entropy = 0.0;

    for count in &freq {
        if *count > 0 {
            let p = *count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

#[async_trait::async_trait]
impl Scanner for SecretScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Secret
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::Content(data) => {
                let content = String::from_utf8_lossy(data);
                let findings = self.detect_secrets(&content);
                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest(
                "Expected Content target".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_secret_aws_key_detection() {
        let scanner = SecretScanner;
        let content = b"export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let req = ScanRequest {
            kind: ScanKind::Secret,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "SEC-001"));
    }

    #[tokio::test]
    async fn test_secret_github_token_detection() {
        let scanner = SecretScanner;
        let content = b"github_token=ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let req = ScanRequest {
            kind: ScanKind::Secret,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "SEC-002"));
    }

    #[tokio::test]
    async fn test_secret_private_key_detection() {
        let scanner = SecretScanner;
        let content = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        let req = ScanRequest {
            kind: ScanKind::Secret,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "SEC-003"));
    }

    #[test]
    fn test_shannon_entropy_calculation() {
        let low_entropy = "aaaaaaaaaaaaaaaaaaa";
        let high_entropy = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcde";

        assert!(shannon_entropy(low_entropy) < 2.0);
        assert!(shannon_entropy(high_entropy) > 4.0);
    }
}
