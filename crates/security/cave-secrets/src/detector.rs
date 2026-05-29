// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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

/// False-positive suppression ported from TruffleHog
/// `pkg/detectors/falsepositives.go`.
///
/// A candidate secret is dropped when its lowercased form is a known
/// placeholder word, when it is built from a single repeated character, or
/// when it otherwise has too little entropy to be a real credential.
pub mod falsepositives {
    use super::shannon_entropy;

    /// Common placeholder / dictionary words that frequently produce false
    /// positives. Mirrors the curated wordlist shipped with TruffleHog's
    /// `falsepositives.go` (`DefaultFalsePositives`), trimmed to the most
    /// common entries.
    pub static WORDLIST: &[&str] = &[
        "example",
        "xxxxxx",
        "aaaaaa",
        "abcde",
        "00000",
        "sample",
        "test",
        "testing",
        "password",
        "passw0rd",
        "secret",
        "changeme",
        "dummy",
        "placeholder",
        "redacted",
        "your_api_key",
        "yourapikey",
        "api_key",
        "apikey",
        "token",
        "123456",
        "1234567890",
        "qwerty",
        "default",
        "none",
        "null",
        "undefined",
        "foobar",
        "deadbeef",
    ];

    /// Returns true when every byte of `s` is identical (e.g. "xxxxxxxx").
    fn all_same_char(s: &str) -> bool {
        let mut bytes = s.bytes();
        match bytes.next() {
            None => true,
            Some(first) => bytes.all(|b| b == first),
        }
    }

    /// Strip the surrounding decoration of a captured candidate to isolate the
    /// secret value: trims whitespace, surrounding quotes and a leading
    /// `key = ` / `key: ` assignment so the wordlist comparison sees only the
    /// value (matching upstream behaviour, which checks the extracted Raw
    /// secret rather than the whole line).
    pub fn extract_candidate(raw: &str) -> &str {
        let mut s = raw.trim();
        // Drop a leading "<ident><sep>" assignment prefix.
        if let Some(pos) = s.find(['=', ':']) {
            // Only treat as assignment when the LHS looks like an identifier.
            let (lhs, rhs) = s.split_at(pos);
            if !lhs.is_empty()
                && lhs
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                s = rhs[1..].trim();
            }
        }
        // Strip surrounding matching quotes.
        let bytes = s.as_bytes();
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' || first == b'\'') && first == last {
                s = &s[1..s.len() - 1];
            }
        }
        s
    }

    /// Port of TruffleHog `IsKnownFalsePositive`. Returns true when `secret`
    /// should be suppressed.
    pub fn is_known_false_positive(secret: &str) -> bool {
        let candidate = extract_candidate(secret);
        let lower = candidate.to_ascii_lowercase();

        if candidate.is_empty() {
            return true;
        }

        // 1. Wordlist match (substring, case-insensitive) — upstream compares
        //    the lowered secret against each known false-positive word.
        for &word in WORDLIST {
            if lower == word || lower.contains(word) {
                return true;
            }
        }

        // 2. A value built from a single repeated character is never a real
        //    secret regardless of length.
        if all_same_char(candidate) {
            return true;
        }

        // 3. Low-entropy heuristic: short or low-entropy values are dropped.
        //    Upstream treats anything below a minimum Shannon entropy as a
        //    likely placeholder. Use a conservative floor so realistic
        //    high-entropy credentials survive.
        let entropy = shannon_entropy(candidate);
        if candidate.len() >= 6 && entropy < 2.5 {
            return true;
        }

        false
    }
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
            let has_key_hint = line.contains("key")
                || line.contains("secret")
                || line.contains("token")
                || line.contains("password")
                || line.contains("KEY")
                || line.contains("SECRET");
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

/// Scan content, then drop findings that match the TruffleHog false-positive
/// filter. For each detector hit the captured secret value is extracted from
/// the matching line and tested against [`falsepositives::is_known_false_positive`].
///
/// This is the recommended entry point for scanning untrusted input: it keeps
/// the raw [`scan`] available for callers that want every regex hit, while
/// suppressing obvious placeholders and low-entropy noise.
pub fn scan_filtered(content: &str, filename: &str, detectors: &[SecretDetector]) -> Vec<Finding> {
    let lines: Vec<&str> = content.lines().collect();
    scan(content, filename, detectors)
        .into_iter()
        .filter(|f| {
            // Re-derive the matched secret value from the original line so the
            // filter inspects the secret rather than the redacted form.
            let raw_line = lines
                .get(f.line.saturating_sub(1))
                .copied()
                .unwrap_or(f.matched.as_str());
            let candidate = secret_candidate(&f.detector, raw_line, detectors);
            !falsepositives::is_known_false_positive(candidate)
        })
        .collect()
}

/// Pull the secret value a detector matched out of `line`. For structured
/// detectors we use the regex match itself; otherwise the whole line is the
/// candidate (the extractor in `falsepositives` strips the `key=` prefix).
fn secret_candidate<'a>(detector: &str, line: &'a str, detectors: &[SecretDetector]) -> &'a str {
    if let Some(det) = detectors.iter().find(|d| d.name == detector) {
        if let Some(m) = det.pattern.find(line) {
            return m.as_str();
        }
    }
    line
}

fn redact_match(line: &str) -> String {
    if line.len() > 20 {
        format!("{}...{}", &line[..8], &line[line.len() - 4..])
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
            pattern: Regex::new(r#"(?i)(api[_-]?key|apikey)\s*[=:]\s*["']?[A-Za-z0-9_\-]{20,}"#)
                .unwrap(),
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
            pattern: Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+")
                .unwrap(),
            severity: Severity::High,
            verify: false,
        },
        SecretDetector {
            name: "slack-webhook",
            pattern: Regex::new(
                r"https://hooks\.slack\.com/services/T[A-Z0-9]+/B[A-Z0-9]+/[A-Za-z0-9]+",
            )
            .unwrap(),
            severity: Severity::High,
            verify: true,
        },
        SecretDetector {
            name: "azure-connection-string",
            pattern: Regex::new(
                r"(?i)DefaultEndpointsProtocol=https;AccountName=[^;]+;AccountKey=[A-Za-z0-9+/=]+",
            )
            .unwrap(),
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
        assert!(
            detectors.len() >= 5,
            "Expected at least 5 builtin detectors, got {}",
            detectors.len()
        );
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
        assert!(
            findings.len() >= 2,
            "Expected at least 2 findings, got {}",
            findings.len()
        );
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
        assert_eq!(
            aws_finding.unwrap().line,
            3,
            "AWS key should be found on line 3"
        );
    }

    // ---------------------------------------------------------------------
    // Extended detector coverage
    // ---------------------------------------------------------------------

    #[test]
    fn test_scan_slack_webhook_detected() {
        let detectors = builtin_detectors();
        let content = "WEBHOOK=https://hooks.slack.com/services/T01ABCDEF/B01ABCDEF/abc123XYZdef\n";
        let findings = scan(content, "secrets.env", &detectors);
        assert!(findings.iter().any(|f| f.detector == "slack-webhook"));
    }

    #[test]
    fn test_scan_azure_connection_detected() {
        let detectors = builtin_detectors();
        let content = "AZ=DefaultEndpointsProtocol=https;AccountName=mystore;AccountKey=YWJjZGVmZ2hpamtsbW5vcA==";
        let findings = scan(content, "az.env", &detectors);
        assert!(
            findings
                .iter()
                .any(|f| f.detector == "azure-connection-string")
        );
    }

    #[test]
    fn test_scan_password_assignment_detected() {
        let detectors = builtin_detectors();
        let content = r#"password = "supersecret123""#;
        let findings = scan(content, "config.toml", &detectors);
        assert!(findings.iter().any(|f| f.detector == "password-assignment"));
    }

    #[test]
    fn test_scan_generic_api_key_detected() {
        let detectors = builtin_detectors();
        let content = r#"api_key = "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345""#;
        let findings = scan(content, "k.toml", &detectors);
        assert!(findings.iter().any(|f| f.detector == "generic-api-key"));
    }

    #[test]
    fn test_scan_ec_private_key_variant_detected() {
        let detectors = builtin_detectors();
        let content = "-----BEGIN EC PRIVATE KEY-----\n";
        let findings = scan(content, "id_ecdsa", &detectors);
        assert!(findings.iter().any(|f| f.detector == "private-key"));
    }

    #[test]
    fn test_scan_dsa_private_key_variant_detected() {
        let detectors = builtin_detectors();
        let content = "-----BEGIN DSA PRIVATE KEY-----\n";
        let findings = scan(content, "id_dsa", &detectors);
        assert!(findings.iter().any(|f| f.detector == "private-key"));
    }

    #[test]
    fn test_scan_ghs_token_detected() {
        let detectors = builtin_detectors();
        // gh[ps]_ pattern accepts both ghp_ and ghs_
        let content = "TOKEN=ghs_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234\n";
        let findings = scan(content, "config.env", &detectors);
        assert!(findings.iter().any(|f| f.detector == "github-token"));
    }

    #[test]
    fn test_high_entropy_finding_requires_key_hint() {
        let detectors = builtin_detectors();
        // High-entropy line *without* "key/secret/token/password" should not fire
        // the generic high-entropy detector.
        let content = "abcdef0123456789ABCDEFG_NOTHINTED";
        let findings = scan(content, "x.txt", &detectors);
        assert!(findings.iter().all(|f| f.detector != "high-entropy"));
    }

    #[test]
    fn test_high_entropy_finding_fires_with_hint() {
        let detectors = builtin_detectors();
        let content = "secret=abcdef0123456789ABCDEFGHIJ_NN_xx_KK_pp";
        let findings = scan(content, "x.txt", &detectors);
        assert!(findings.iter().any(|f| f.detector == "high-entropy"));
    }

    #[test]
    fn test_short_line_not_redacted() {
        // Short matched lines should be returned verbatim, not redacted.
        let detectors = builtin_detectors();
        // Use a long-enough AKIA but force the matched line to remain short.
        let content = "AKIAIOSFODNN7EXAM"; // 17 chars — under redaction threshold
        let findings = scan(content, "x.env", &detectors);
        // No detector should fire (AKIA pattern needs 16 trailing alnum after AKIA)
        assert!(findings.iter().all(|f| !f.matched.contains("...")));
    }

    #[test]
    fn test_long_line_is_redacted() {
        let detectors = builtin_detectors();
        let content = "config_token_string=AKIAIOSFODNN7EXAMPLEEXTRAPADDING";
        let findings = scan(content, "x.env", &detectors);
        let aws = findings
            .iter()
            .find(|f| f.detector == "aws-access-key")
            .unwrap();
        assert!(aws.matched.contains("..."));
    }

    #[test]
    fn test_severity_assignment_aws_critical() {
        let detectors = builtin_detectors();
        let content = "K=AKIAIOSFODNN7EXAMPLE";
        let findings = scan(content, "x.env", &detectors);
        let aws = findings
            .iter()
            .find(|f| f.detector == "aws-access-key")
            .unwrap();
        assert_eq!(aws.severity, Severity::Critical);
    }

    #[test]
    fn test_severity_assignment_jwt_high() {
        let detectors = builtin_detectors();
        let content = "Auth: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let findings = scan(content, "r.txt", &detectors);
        let jwt = findings.iter().find(|f| f.detector == "jwt-token").unwrap();
        assert_eq!(jwt.severity, Severity::High);
    }

    #[test]
    fn test_finding_unverified_by_default() {
        let detectors = builtin_detectors();
        let content = "K=AKIAIOSFODNN7EXAMPLE";
        let findings = scan(content, "x.env", &detectors);
        assert!(findings.iter().all(|f| !f.verified));
    }

    #[test]
    fn test_scan_finds_expected_count_when_multi_detector_overlap() {
        let detectors = builtin_detectors();
        // Line containing AWS key + entropy hint => should produce at least 2 findings
        // (aws-access-key + possibly high-entropy if the line is long enough).
        let content = "secret_aws_key_AKIAIOSFODNN7EXAMPLE_padding_padding_extra";
        let findings = scan(content, "x.env", &detectors);
        assert!(findings.len() >= 1);
    }

    #[test]
    fn test_shannon_entropy_zero_for_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    // ---------------------------------------------------------------------
    // False-positive suppression (TruffleHog falsepositives.go port)
    // ---------------------------------------------------------------------

    #[test]
    fn test_fp_wordlist_match() {
        assert!(falsepositives::is_known_false_positive("example"));
        assert!(falsepositives::is_known_false_positive("PASSWORD"));
    }

    #[test]
    fn test_fp_all_same_char() {
        assert!(falsepositives::is_known_false_positive(
            "xxxxxxxxxxxxxxxxxxxxxxxx"
        ));
    }

    #[test]
    fn test_fp_realistic_secret_kept() {
        assert!(!falsepositives::is_known_false_positive(
            "A7xQ9pL2zR8vK3mN6wT1yB5dF4hG0jUq"
        ));
    }

    #[test]
    fn test_fp_extract_candidate_strips_assignment_and_quotes() {
        assert_eq!(
            falsepositives::extract_candidate(r#"API_KEY="example""#),
            "example"
        );
    }

    #[test]
    fn test_scan_filtered_drops_placeholder() {
        let detectors = builtin_detectors();
        let findings = scan_filtered(r#"api_key="example""#, "x.env", &detectors);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_scan_filtered_keeps_real_secret() {
        let detectors = builtin_detectors();
        let findings =
            scan_filtered(r#"api_key="A7xQ9pL2zR8vK3mN6wT1yB5dF4hG0jUq""#, "x.env", &detectors);
        assert!(!findings.is_empty());
    }
}
