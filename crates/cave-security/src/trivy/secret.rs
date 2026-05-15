// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Secret scanning — regex + entropy patterns matching Trivy's ruleset.

use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    pub rule_id: String,
    pub title: String,
    pub file_path: String,
    pub line_number: usize,
    pub match_preview: String,
    pub severity: SecretSeverity,
    pub category: SecretCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SecretSeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCategory {
    AwsCredentials,
    GcpCredentials,
    AzureCredentials,
    GitHubToken,
    GitLabToken,
    PrivateKey,
    GenericApiKey,
    Password,
    DatabaseUrl,
    SlackWebhook,
    StripeKey,
    TwilioKey,
    SendgridKey,
    JwtToken,
    DockerAuth,
    Other,
}

// ---------------------------------------------------------------------------
// Pattern
// ---------------------------------------------------------------------------

pub struct SecretPattern {
    pub id: &'static str,
    pub title: &'static str,
    pub pattern: Regex,
    pub severity: SecretSeverity,
    pub category: SecretCategory,
}

// ---------------------------------------------------------------------------
// Builtin patterns (Trivy-parity ruleset)
// ---------------------------------------------------------------------------

pub fn builtin_patterns() -> Vec<SecretPattern> {
    let rules: Vec<(&str, &str, &str, SecretSeverity, SecretCategory)> = vec![
        // AWS
        ("aws-access-key-id",
         "AWS Access Key ID",
         r"(?i)(AKIA|ABIA|ACCA|ASIA)[0-9A-Z]{16}",
         SecretSeverity::Critical,
         SecretCategory::AwsCredentials),
        ("aws-secret-access-key",
         "AWS Secret Access Key",
         r#"(?i)(aws_secret_access_key|secret_access_key)\s*[=:]\s*["']?[A-Za-z0-9/+]{40}"#,
         SecretSeverity::Critical,
         SecretCategory::AwsCredentials),
        ("aws-session-token",
         "AWS Session Token",
         r"(?i)(aws_session_token|session_token)\s*[=:]\s*[A-Za-z0-9/+=]{100,}",
         SecretSeverity::Critical,
         SecretCategory::AwsCredentials),
        // GCP
        ("gcp-service-account",
         "GCP Service Account Key",
         r#""type"\s*:\s*"service_account""#,
         SecretSeverity::Critical,
         SecretCategory::GcpCredentials),
        ("gcp-api-key",
         "GCP API Key",
         r"AIza[0-9A-Za-z\-_]{35}",
         SecretSeverity::High,
         SecretCategory::GcpCredentials),
        // Azure
        ("azure-client-secret",
         "Azure Client Secret",
         r#"(?i)(client_secret|azure_client_secret)\s*[=:]\s*["']?[A-Za-z0-9_\-.~]{34,}"#,
         SecretSeverity::Critical,
         SecretCategory::AzureCredentials),
        ("azure-connection-string",
         "Azure Storage Connection String",
         r"DefaultEndpointsProtocol=https;AccountName=[^;]+;AccountKey=[A-Za-z0-9+/=]+;",
         SecretSeverity::Critical,
         SecretCategory::AzureCredentials),
        // GitHub
        ("github-pat",
         "GitHub Personal Access Token",
         r"ghp_[A-Za-z0-9]{36,}",
         SecretSeverity::Critical,
         SecretCategory::GitHubToken),
        ("github-oauth",
         "GitHub OAuth Token",
         r"gho_[A-Za-z0-9]{36,}",
         SecretSeverity::Critical,
         SecretCategory::GitHubToken),
        ("github-server-to-server",
         "GitHub Server-to-Server Token",
         r"ghs_[A-Za-z0-9]{36,}",
         SecretSeverity::Critical,
         SecretCategory::GitHubToken),
        ("github-refresh-token",
         "GitHub Refresh Token",
         r"ghr_[A-Za-z0-9]{76}",
         SecretSeverity::High,
         SecretCategory::GitHubToken),
        // GitLab
        ("gitlab-pat",
         "GitLab Personal Access Token",
         r"glpat-[A-Za-z0-9\-_]{20}",
         SecretSeverity::Critical,
         SecretCategory::GitLabToken),
        // Private keys
        ("rsa-private-key",
         "RSA Private Key",
         r"-----BEGIN RSA PRIVATE KEY-----",
         SecretSeverity::Critical,
         SecretCategory::PrivateKey),
        ("ec-private-key",
         "EC Private Key",
         r"-----BEGIN EC PRIVATE KEY-----",
         SecretSeverity::Critical,
         SecretCategory::PrivateKey),
        ("openssh-private-key",
         "OpenSSH Private Key",
         r"-----BEGIN OPENSSH PRIVATE KEY-----",
         SecretSeverity::Critical,
         SecretCategory::PrivateKey),
        ("pgp-private-key",
         "PGP Private Key",
         r"-----BEGIN PGP PRIVATE KEY BLOCK-----",
         SecretSeverity::Critical,
         SecretCategory::PrivateKey),
        // JWT
        ("jwt-token",
         "JSON Web Token",
         r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+",
         SecretSeverity::High,
         SecretCategory::JwtToken),
        // Slack
        ("slack-webhook",
         "Slack Webhook URL",
         r"https://hooks\.slack\.com/services/T[A-Z0-9]{8,}/B[A-Z0-9]{8,}/[A-Za-z0-9]{24,}",
         SecretSeverity::High,
         SecretCategory::SlackWebhook),
        ("slack-token",
         "Slack Token",
         r"xox[baprs]-[0-9]{12}-[0-9]{12}-[A-Za-z0-9]{24}",
         SecretSeverity::High,
         SecretCategory::SlackWebhook),
        // Stripe
        ("stripe-sk",
         "Stripe Secret Key",
         r"sk_live_[A-Za-z0-9]{24,}",
         SecretSeverity::Critical,
         SecretCategory::StripeKey),
        ("stripe-pk",
         "Stripe Publishable Key",
         r"pk_live_[A-Za-z0-9]{24,}",
         SecretSeverity::Medium,
         SecretCategory::StripeKey),
        // Twilio
        ("twilio-sid",
         "Twilio Account SID",
         r"AC[a-z0-9]{32}",
         SecretSeverity::High,
         SecretCategory::TwilioKey),
        // SendGrid
        ("sendgrid-api-key",
         "SendGrid API Key",
         r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}",
         SecretSeverity::High,
         SecretCategory::SendgridKey),
        // Generic
        ("generic-api-key",
         "Generic API Key",
         r#"(?i)(api[_-]?key|apikey|x-api-key)\s*[=:]\s*["']?[A-Za-z0-9_\-]{20,}"#,
         SecretSeverity::Medium,
         SecretCategory::GenericApiKey),
        ("generic-password",
         "Generic Password",
         r#"(?i)(password|passwd|pwd|secret)\s*[=:]\s*["'][^"']{8,}["']"#,
         SecretSeverity::Medium,
         SecretCategory::Password),
        ("database-url",
         "Database URL with credentials",
         r"(?i)(postgres|mysql|mongodb|redis|sqlserver)://[^:]+:[^@]+@[^\s]+",
         SecretSeverity::Critical,
         SecretCategory::DatabaseUrl),
        // Docker
        ("docker-auth",
         "Docker Registry Auth",
         r#""auth"\s*:\s*"[A-Za-z0-9+/=]{20,}""#,
         SecretSeverity::High,
         SecretCategory::DockerAuth),
    ];

    rules
        .into_iter()
        .filter_map(|(id, title, pattern, severity, category)| {
            Regex::new(pattern).ok().map(|re| SecretPattern {
                id,
                title,
                pattern: re,
                severity,
                category,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

/// Shannon entropy for high-entropy string detection.
pub fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 { return 0.0; }
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

/// Redact a matched string for safe display.
fn redact(s: &str) -> String {
    if s.len() <= 12 {
        "*".repeat(s.len())
    } else {
        format!("{}...{}", &s[..4], &s[s.len() - 4..])
    }
}

/// Scan a file's content for secrets using builtin patterns.
pub fn scan_file_for_secrets(
    content: &str,
    file_path: &str,
    patterns: &[SecretPattern],
) -> Vec<SecretFinding> {
    let mut findings = Vec::new();

    for (line_number, line) in content.lines().enumerate() {
        for pat in patterns {
            if let Some(m) = pat.pattern.find(line) {
                findings.push(SecretFinding {
                    rule_id: pat.id.to_string(),
                    title: pat.title.to_string(),
                    file_path: file_path.to_string(),
                    line_number: line_number + 1,
                    match_preview: redact(m.as_str()),
                    severity: pat.severity.clone(),
                    category: pat.category.clone(),
                });
            }
        }

        // High-entropy string detection (Trivy: entropy > 3.5, len > 20)
        for token in line.split_whitespace() {
            let token = token.trim_matches(|c| matches!(c, '"' | '\'' | ',' | ';' | '='));
            if token.len() >= 20 && shannon_entropy(token) > 3.5 {
                let has_secret_hint = line.to_lowercase().contains("key")
                    || line.to_lowercase().contains("secret")
                    || line.to_lowercase().contains("token")
                    || line.to_lowercase().contains("password")
                    || line.to_lowercase().contains("credential");
                if has_secret_hint {
                    // Don't double-report if already found by regex
                    let already_reported = findings
                        .iter()
                        .any(|f| f.file_path == file_path && f.line_number == line_number + 1);
                    if !already_reported {
                        findings.push(SecretFinding {
                            rule_id: "high-entropy-string".to_string(),
                            title: "High-entropy secret candidate".to_string(),
                            file_path: file_path.to_string(),
                            line_number: line_number + 1,
                            match_preview: redact(token),
                            severity: SecretSeverity::Medium,
                            category: SecretCategory::Other,
                        });
                    }
                }
            }
        }
    }
    findings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_aws_key() {
        let patterns = builtin_patterns();
        let findings = scan_file_for_secrets(
            "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
            ".env",
            &patterns,
        );
        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.rule_id == "aws-access-key-id"));
    }

    #[test]
    fn detects_private_key() {
        let patterns = builtin_patterns();
        let findings = scan_file_for_secrets(
            "-----BEGIN RSA PRIVATE KEY-----",
            "id_rsa",
            &patterns,
        );
        assert!(findings.iter().any(|f| f.rule_id == "rsa-private-key"));
    }

    #[test]
    fn detects_github_token() {
        let patterns = builtin_patterns();
        // GitHub PAT = "ghp_" + 36 alphanumeric chars (40 total)
        let findings = scan_file_for_secrets(
            "token: ghp_abcdefghijklmnopqrstuvwxyz1234567890",
            "config.yaml",
            &patterns,
        );
        assert!(findings.iter().any(|f| f.rule_id == "github-pat"));
    }

    #[test]
    fn detects_database_url() {
        let patterns = builtin_patterns();
        let findings = scan_file_for_secrets(
            "DATABASE_URL=postgres://admin:s3cr3t@db.example.com/mydb",
            ".env",
            &patterns,
        );
        assert!(findings.iter().any(|f| f.rule_id == "database-url"));
    }

    #[test]
    fn entropy_calculation() {
        let low = shannon_entropy("aaaaaaaaaa");
        let high = shannon_entropy("aB3$xY9!kLmN2pQr");
        assert!(low < 1.0);
        assert!(high > 3.0);
    }

    #[test]
    fn no_false_positive_on_comment() {
        let patterns = builtin_patterns();
        let findings = scan_file_for_secrets(
            "# This is just a comment about API keys",
            "README.md",
            &patterns,
        );
        assert!(findings.is_empty());
    }
}
