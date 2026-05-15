//! Data models for cave-secrets.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// SecretType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretType {
    ApiKey,
    AwsCredential,
    GithubToken,
    GitlabToken,
    SlackToken,
    PrivateKey,
    Certificate,
    Password,
    DatabaseUrl,
    GenericSecret,
    GoogleApiKey,
    StripeKey,
    SendgridKey,
    JwtSecret,
}

impl SecretType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecretType::ApiKey => "api_key",
            SecretType::AwsCredential => "aws_credential",
            SecretType::GithubToken => "github_token",
            SecretType::GitlabToken => "gitlab_token",
            SecretType::SlackToken => "slack_token",
            SecretType::PrivateKey => "private_key",
            SecretType::Certificate => "certificate",
            SecretType::Password => "password",
            SecretType::DatabaseUrl => "database_url",
            SecretType::GenericSecret => "generic_secret",
            SecretType::GoogleApiKey => "google_api_key",
            SecretType::StripeKey => "stripe_key",
            SecretType::SendgridKey => "sendgrid_key",
            SecretType::JwtSecret => "jwt_secret",
        }
    }
}

impl std::fmt::Display for SecretType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Confidence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => f.write_str("high"),
            Confidence::Medium => f.write_str("medium"),
            Confidence::Low => f.write_str("low"),
        }
    }
}

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Critical => f.write_str("critical"),
            Severity::High => f.write_str("high"),
            Severity::Medium => f.write_str("medium"),
            Severity::Low => f.write_str("low"),
        }
    }
}

// ---------------------------------------------------------------------------
// SecretFinding
// ---------------------------------------------------------------------------

/// A single secret detected in scanned content.
///
/// The `id` field is a deterministic FNV-1a hex digest computed from
/// `rule_id + file_path + line_number` so that identical findings across
/// repeated scans produce the same ID (enabling deduplication and diff).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// Deterministic FNV-1a hash of (rule_id + file_path + line_number).
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub secret_type: SecretType,
    pub file_path: String,
    pub line_number: Option<usize>,
    pub column: Option<usize>,
    /// Redacted value, e.g. `"ghp_****"`.
    pub redacted_value: String,
    pub entropy: f64,
    pub confidence: Confidence,
    /// Surrounding text for human review.
    pub context: String,
    pub commit: Option<String>,
}

// ---------------------------------------------------------------------------
// SecretRule
// ---------------------------------------------------------------------------

/// A detection rule used by the scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRule {
    pub id: String,
    pub name: String,
    pub secret_type: SecretType,
    /// Regex pattern (compiled at runtime).
    pub pattern: String,
    /// Fast pre-filter keywords; rule only runs regex when at least one keyword
    /// is present on the line.
    pub keywords: Vec<String>,
    pub confidence: Confidence,
    /// Minimum Shannon entropy for the matched value to be reported.
    pub entropy_threshold: f64,
    pub severity: Severity,
}

// ---------------------------------------------------------------------------
// AllowlistEntry
// ---------------------------------------------------------------------------

/// An entry that suppresses findings whose file path or redacted value contains
/// `pattern`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistEntry {
    pub id: String,
    pub pattern: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// ScanRequest / ScanResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRequest {
    pub content: String,
    pub file_path: String,
    #[serde(default = "default_true")]
    pub redact: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub file_path: String,
    pub findings: Vec<SecretFinding>,
    pub scanned_lines: usize,
    pub scanned_bytes: usize,
    /// Wall-clock scan duration in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// ScanStats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    pub total_findings: usize,
    /// Number of findings per `SecretType` string key.
    pub by_type: HashMap<String, usize>,
    /// Number of findings per severity level.
    pub by_severity: HashMap<String, usize>,
    pub files_scanned: usize,
    pub high_entropy_count: usize,
    pub high_confidence_count: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_type_display() {
        assert_eq!(SecretType::GithubToken.to_string(), "github_token");
        assert_eq!(SecretType::AwsCredential.to_string(), "aws_credential");
        assert_eq!(SecretType::JwtSecret.to_string(), "jwt_secret");
    }

    #[test]
    fn confidence_ordering() {
        assert!(Confidence::High > Confidence::Medium);
        assert!(Confidence::Medium > Confidence::Low);
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn secret_finding_serialization() {
        let f = SecretFinding {
            id: "abc123".to_string(),
            rule_id: "aws-access-key".to_string(),
            rule_name: "AWS Access Key".to_string(),
            secret_type: SecretType::AwsCredential,
            file_path: "config.env".to_string(),
            line_number: Some(5),
            column: Some(10),
            redacted_value: "AKIA****".to_string(),
            entropy: 3.5,
            confidence: Confidence::High,
            context: "AWS_KEY=AKIAIOSFODNN7EXAMPLE".to_string(),
            commit: None,
        };
        let json = serde_json::to_string(&f).expect("serialize");
        assert!(json.contains("aws_credential"));
        assert!(json.contains("high"));
    }

    #[test]
    fn scan_request_default_redact() {
        let json = r#"{"content":"foo","file_path":"a.txt"}"#;
        let req: ScanRequest = serde_json::from_str(json).expect("deserialize");
        assert!(req.redact);
    }

    #[test]
    fn scan_stats_default() {
        let s = ScanStats {
            total_findings: 0,
            by_type: HashMap::new(),
            by_severity: HashMap::new(),
            files_scanned: 0,
            high_entropy_count: 0,
            high_confidence_count: 0,
        };
        assert_eq!(s.total_findings, 0);
    }

    #[test]
    fn secret_type_as_str_matches_display() {
        for t in [
            SecretType::ApiKey,
            SecretType::AwsCredential,
            SecretType::PrivateKey,
            SecretType::SlackToken,
            SecretType::JwtSecret,
        ] {
            assert_eq!(t.as_str(), t.to_string());
        }
    }

    #[test]
    fn secret_type_serde_roundtrip_all_variants() {
        let variants = [
            SecretType::ApiKey,
            SecretType::AwsCredential,
            SecretType::GithubToken,
            SecretType::GitlabToken,
            SecretType::SlackToken,
            SecretType::PrivateKey,
            SecretType::Certificate,
            SecretType::Password,
            SecretType::DatabaseUrl,
            SecretType::GenericSecret,
            SecretType::GoogleApiKey,
            SecretType::StripeKey,
            SecretType::SendgridKey,
            SecretType::JwtSecret,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let back: SecretType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, &back);
        }
    }

    #[test]
    fn confidence_serde_snake_case() {
        let json = serde_json::to_string(&Confidence::High).unwrap();
        assert_eq!(json, "\"high\"");
    }

    #[test]
    fn severity_serde_snake_case() {
        let json = serde_json::to_string(&Severity::Critical).unwrap();
        assert_eq!(json, "\"critical\"");
    }

    #[test]
    fn scan_request_explicit_redact_false() {
        let req: ScanRequest =
            serde_json::from_str(r#"{"content":"x","file_path":"a","redact":false}"#).unwrap();
        assert!(!req.redact);
    }

    #[test]
    fn scan_result_serializes() {
        let r = ScanResult {
            file_path: "f.txt".to_string(),
            findings: vec![],
            scanned_lines: 10,
            scanned_bytes: 100,
            duration_ms: 5,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("scanned_lines"));
        assert!(json.contains("duration_ms"));
    }

    #[test]
    fn allowlist_entry_serializes() {
        let e = AllowlistEntry {
            id: "a1".to_string(),
            pattern: "test/fixtures/*".to_string(),
            reason: "test data".to_string(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("test/fixtures"));
        assert!(json.contains("test data"));
    }
}
