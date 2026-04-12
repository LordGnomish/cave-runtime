//! Secret detection patterns — regex + entropy based.

use regex::Regex;

#[derive(Clone)]
pub struct SecretDetector {
    pub name: &'static str,
    pub pattern: Regex,
    pub severity: Severity,
    pub verify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub detector: String,
    pub file: String,
    pub line: usize,
    pub matched: String,
    pub severity: Severity,
    pub verified: bool,
}

/// Shannon entropy calculation for detecting high-entropy strings.
pub fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    for b in s.bytes() {
        freq[b as usize] += 1;
    }
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// Scan content for secrets using all detectors.
pub fn scan(content: &str, filename: &str, detectors: &[SecretDetector]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (line_num, line) in content.lines().enumerate() {
        for det in detectors {
            if det.pattern.is_match(line) {
                findings.push(Finding {
                    detector: det.name.to_string(),
                    file: filename.to_string(),
                    line: line_num + 1,
                    matched: redact_match(line),
                    severity: det.severity,
                    verified: false,
                });
            }
        }
        // High entropy check for hex/base64 strings
        if line.len() > 20 && shannon_entropy(line) > 4.5 {
            let has_key_hint = line.contains("key") || line.contains("secret")
                || line.contains("token") || line.contains("password")
                || line.contains("KEY") || line.contains("SECRET");
            if has_key_hint {
                findings.push(Finding {
                    detector: "high-entropy".to_string(),
                    file: filename.to_string(),
                    line: line_num + 1,
                    matched: redact_match(line),
                    severity: Severity::Medium,
                    verified: false,
                });
            }
        }
    }
    findings
}

fn redact_match(line: &str) -> String {
    if line.len() > 20 {
        format!("{}...{}", &line[..8], &line[line.len()-4..])
    } else {
        line.to_string()
    }
}

pub fn builtin_detectors() -> Vec<SecretDetector> {
    vec![
        SecretDetector {
            name: "aws-access-key",
            pattern: Regex::new(r"(?i)AKIA[0-9A-Z]{16}").unwrap(),
            severity: Severity::Critical,
            verify: true,
        },
        SecretDetector {
            name: "github-token",
            pattern: Regex::new(r"gh[ps]_[A-Za-z0-9_]{36,}").unwrap(),
            severity: Severity::Critical,
            verify: true,
        },
        SecretDetector {
            name: "generic-api-key",
            pattern: Regex::new(r#"(?i)(api[_-]?key|apikey)\s*[=:]\s*["']?[A-Za-z0-9_\-]{20,}"#).unwrap(),
            severity: Severity::High,
            verify: false,
        },
        SecretDetector {
            name: "private-key",
            pattern: Regex::new(r"-----BEGIN (RSA |EC |DSA )?PRIVATE KEY-----").unwrap(),
            severity: Severity::Critical,
            verify: false,
        },
        SecretDetector {
            name: "jwt-token",
            pattern: Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+").unwrap(),
            severity: Severity::High,
            verify: false,
        },
        SecretDetector {
            name: "slack-webhook",
            pattern: Regex::new(r"https://hooks\.slack\.com/services/T[A-Z0-9]+/B[A-Z0-9]+/[A-Za-z0-9]+").unwrap(),
            severity: Severity::High,
            verify: true,
        },
        SecretDetector {
            name: "azure-connection-string",
            pattern: Regex::new(r"(?i)DefaultEndpointsProtocol=https;AccountName=[^;]+;AccountKey=[A-Za-z0-9+/=]+").unwrap(),
            severity: Severity::Critical,
            verify: false,
        },
        SecretDetector {
            name: "password-assignment",
            pattern: Regex::new(r#"(?i)(password|passwd|pwd)\s*[=:]\s*["'][^"']{8,}"#).unwrap(),
            severity: Severity::High,
            verify: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy() {
        assert!(shannon_entropy("aaaaaaa") < 1.0);
        assert!(shannon_entropy("aB3$xY9!kL") > 3.0);
    }

    #[test]
    fn test_aws_key_detection() {
        let detectors = builtin_detectors();
        let content = "AWS_KEY=AKIAIOSFODNN7EXAMPLE";
        let findings = scan(content, "test.env", &detectors);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].detector, "aws-access-key");
    }

    #[test]
    fn test_private_key_detection() {
        let detectors = builtin_detectors();
        let content = "-----BEGIN RSA PRIVATE KEY-----";
        let findings = scan(content, "id_rsa", &detectors);
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_entropy_low() {
        // "aaaa" has only one unique character, entropy should be near 0
        assert!(shannon_entropy("aaaa") < 0.01);
    }

    #[test]
    fn test_entropy_high() {
        // random-looking string with many unique chars should have high entropy
        assert!(shannon_entropy("aB3$xY9!kLmN2@pQrS") > 3.0);
    }

    #[test]
    fn test_entropy_binary_string() {
        // "0101010101" has only 2 unique characters, entropy should be ~1.0
        let e = shannon_entropy("0101010101");
        assert!((e - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_scan_aws_key_detected() {
        let detectors = builtin_detectors();
        let content = "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n";
        let findings = scan(content, "test.env", &detectors);
        assert!(!findings.is_empty());
        let aws_finding = findings.iter().find(|f| f.detector == "aws-access-key");
        assert!(aws_finding.is_some(), "Expected aws-access-key finding");
    }

    #[test]
    fn test_scan_github_token() {
        let detectors = builtin_detectors();
        let content = "TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234\n";
        let findings = scan(content, "config.env", &detectors);
        assert!(!findings.is_empty());
        let gh_finding = findings.iter().find(|f| f.detector == "github-token");
        assert!(gh_finding.is_some(), "Expected github-token finding");
    }

    #[test]
    fn test_scan_private_key() {
        let detectors = builtin_detectors();
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\n";
        let findings = scan(content, "id_rsa", &detectors);
        assert!(!findings.is_empty());
        let pk_finding = findings.iter().find(|f| f.detector == "private-key");
        assert!(pk_finding.is_some(), "Expected private-key finding");
    }

    #[test]
    fn test_scan_jwt_token() {
        let detectors = builtin_detectors();
        let content = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c\n";
        let findings = scan(content, "request.txt", &detectors);
        assert!(!findings.is_empty());
        let jwt_finding = findings.iter().find(|f| f.detector == "jwt-token");
        assert!(jwt_finding.is_some(), "Expected jwt-token finding");
    }

    #[test]
    fn test_scan_empty_content() {
        let detectors = builtin_detectors();
        let findings = scan("", "empty.txt", &detectors);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_scan_clean_content() {
        let detectors = builtin_detectors();
        let content = "# This is a normal config file\nHOST=localhost\nPORT=8080\n";
        let findings = scan(content, "config.txt", &detectors);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_builtin_detectors_count() {
        let detectors = builtin_detectors();
        assert!(detectors.len() >= 5, "Expected at least 5 builtin detectors, got {}", detectors.len());
    }

    #[test]
    fn test_builtin_detector_names() {
        let detectors = builtin_detectors();
        for det in &detectors {
            assert!(!det.name.is_empty(), "Detector name should not be empty");
        }
    }

    #[test]
    fn test_scan_multiple_findings() {
        let detectors = builtin_detectors();
        let content = "AWS_KEY=AKIAIOSFODNN7EXAMPLE\n-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\n";
        let findings = scan(content, "secrets.txt", &detectors);
        assert!(findings.len() >= 2, "Expected at least 2 findings, got {}", findings.len());
        let has_aws = findings.iter().any(|f| f.detector == "aws-access-key");
        let has_pk = findings.iter().any(|f| f.detector == "private-key");
        assert!(has_aws, "Expected aws-access-key finding");
        assert!(has_pk, "Expected private-key finding");
    }

    #[test]
    fn test_finding_line_number() {
        let detectors = builtin_detectors();
        let content = "# line 1: nothing\n# line 2: nothing\nAWS_KEY=AKIAIOSFODNN7EXAMPLE\n";
        let findings = scan(content, "test.env", &detectors);
        let aws_finding = findings.iter().find(|f| f.detector == "aws-access-key");
        assert!(aws_finding.is_some(), "Expected aws-access-key finding");
        assert_eq!(aws_finding.unwrap().line, 3, "AWS key should be found on line 3");
    }
}
