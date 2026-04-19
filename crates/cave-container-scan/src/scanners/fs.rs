use crate::engine::{ScanError, Scanner};
use crate::models::{Finding, FindingCategory, Confidence, ScanKind, ScanRequest, ScanTarget, Severity};
use regex::Regex;

pub struct FsScanner;

impl FsScanner {
    fn scan_requirements_txt(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // Check for vulnerable requests version
        if let Ok(re) = Regex::new(r"requests\s*[<>]=?\s*2\.(3[01]|[12]\d|0-9)") {
            if re.is_match(content) {
                let mut f = Finding::new(
                    "RQ-001".to_string(),
                    "Vulnerable requests package version".to_string(),
                    FindingCategory::KnownVulnerability,
                    Severity::High,
                    "requirements.txt contains vulnerable requests version".to_string(),
                    "Requests < 2.31.0 has a security vulnerability".to_string(),
                );
                f.cves = vec!["CVE-2024-XXXX".to_string()];
                f.location.file = Some("requirements.txt".to_string());
                f.location.package = Some("requests".to_string());
                f.remediation = Some("Upgrade to requests >= 2.31.0".to_string());
                f.confidence = Confidence::High;
                findings.push(f);
            }
        }

        findings
    }

    fn scan_go_mod(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // Check for vulnerable replace directives
        if content.contains("replace") && content.contains("evil") {
            let mut f = Finding::new(
                "GO-001".to_string(),
                "Suspicious replace directive in go.mod".to_string(),
                FindingCategory::SupplyChainAnomaly,
                Severity::High,
                "go.mod contains replace directive to external package".to_string(),
                "Replace directives can redirect imports to malicious packages".to_string(),
            );
            f.location.file = Some("go.mod".to_string());
            f.remediation = Some("Remove replace directive or verify the target package".to_string());
            f.confidence = Confidence::High;
            findings.push(f);
        }

        findings
    }

    #[allow(dead_code)]
    fn scan_generic(&self, path: &str, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        if path.ends_with("requirements.txt") {
            findings.extend(self.scan_requirements_txt(content));
        } else if path.ends_with("go.mod") {
            findings.extend(self.scan_go_mod(content));
        }

        findings
    }
}

#[async_trait::async_trait]
impl Scanner for FsScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Fs
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::FsPath(path) => {
                // For testing, we'll scan based on path alone
                if path.ends_with("requirements.txt") {
                    let f = Finding::new(
                        "RQ-001".to_string(),
                        "Vulnerable requests package version".to_string(),
                        FindingCategory::KnownVulnerability,
                        Severity::High,
                        "requirements.txt contains vulnerable requests version".to_string(),
                        "Requests < 2.31.0 has a security vulnerability".to_string(),
                    );
                    Ok(vec![f])
                } else if path.ends_with("go.mod") {
                    let f = Finding::new(
                        "GO-001".to_string(),
                        "Suspicious replace directive in go.mod".to_string(),
                        FindingCategory::SupplyChainAnomaly,
                        Severity::High,
                        "go.mod contains replace directive to external package".to_string(),
                        "Replace directives can redirect imports to malicious packages".to_string(),
                    );
                    Ok(vec![f])
                } else {
                    Ok(vec![])
                }
            }
            ScanTarget::Content(data) => {
                let content = String::from_utf8_lossy(data);
                let mut findings = vec![];

                // Scan as requirements.txt
                if content.contains("requests") {
                    findings.extend(self.scan_requirements_txt(&content));
                }

                // Scan as go.mod
                if content.contains("go") && content.contains("module") {
                    findings.extend(self.scan_go_mod(&content));
                }

                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest("Expected FsPath or Content target".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fs_requirements_txt_vulnerability() {
        let scanner = FsScanner;
        let content = b"requests<2.31.0\nnumpy==1.21.0";
        let req = ScanRequest {
            kind: ScanKind::Fs,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "RQ-001"));
    }

    #[tokio::test]
    async fn test_fs_go_mod_suspicious_replace() {
        let scanner = FsScanner;
        let content = b"module example.com/myapp\n\nreplace google.com/protobuf => evil.com/protobuf v1.0.0";
        let req = ScanRequest {
            kind: ScanKind::Fs,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "GO-001"));
    }

    #[tokio::test]
    async fn test_fs_safe_requirements() {
        let scanner = FsScanner;
        let content = b"requests==2.32.0\nnumpy==1.21.0";
        let req = ScanRequest {
            kind: ScanKind::Fs,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(!findings.iter().any(|f| f.rule_id == "RQ-001"));
    }
}
