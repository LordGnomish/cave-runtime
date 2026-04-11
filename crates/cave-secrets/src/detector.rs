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
}
