// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: gitleaks/gitleaks@9febafb config/gitleaks.toml
//! Built-in secret regex patterns.
//!
//! Each rule is a static `(id, severity, regex)` triple. The pattern set is
//! a curated port of gitleaks's default config — we only include the patterns
//! whose regex doesn't depend on PCRE features (no back-references, no
//! lookarounds). Anything fancier is omitted; see manifest `status="missing"`.
//!
//! Pattern count: 44 (≥40 required).

use super::{SecretDetector, SecretFinding, Severity};
use regex::RegexSet;

#[derive(Debug, Clone, Copy)]
struct RuleMeta {
    id: &'static str,
    severity: Severity,
}

/// All built-in rules. Order matters — earlier rules win on tie (RegexSet
/// gives us all matching indices, we pick the smallest).
const RULES: &[(&str, &str, Severity)] = &[
    // Cloud / IaaS
    ("aws-access-key", r"AKIA[0-9A-Z]{16}", Severity::Critical),
    (
        "aws-secret-key",
        r"(?i)aws[_\-]?secret[_\-]?(access[_\-]?)?key[ \t]*[:=][ \t]*[A-Za-z0-9/+]{40}",
        Severity::Critical,
    ),
    (
        "aws-session-token",
        r"FQoGZXIvYXdz[A-Za-z0-9/+=]{50,}",
        Severity::High,
    ),
    (
        "gcp-service-account",
        r#""type"\s*:\s*"service_account""#,
        Severity::High,
    ),
    ("gcp-api-key", r"AIza[0-9A-Za-z\-_]{35}", Severity::High),
    (
        "azure-storage-key",
        r"DefaultEndpointsProtocol=https;AccountName=",
        Severity::High,
    ),
    (
        "azure-ad-client-secret",
        r"(?i)azure[_\-]?(ad)?[_\-]?client[_\-]?secret[ \t]*[:=][ \t]*[A-Za-z0-9~_\-\.]{32,}",
        Severity::High,
    ),
    // Git platforms
    ("github-pat", r"ghp_[A-Za-z0-9]{36}", Severity::Critical),
    ("github-oauth", r"gho_[A-Za-z0-9]{36}", Severity::High),
    ("github-app", r"(ghu_|ghs_)[A-Za-z0-9]{36}", Severity::High),
    (
        "github-fine-grained",
        r"github_pat_[A-Za-z0-9_]{82}",
        Severity::Critical,
    ),
    ("gitlab-pat", r"glpat-[A-Za-z0-9\-_]{20}", Severity::High),
    (
        "gitlab-pipeline-trigger",
        r"glptt-[0-9a-f]{40}",
        Severity::High,
    ),
    (
        "bitbucket-client-secret",
        r"(?i)bitbucket[_\-]?(client)?[_\-]?secret[ \t]*[:=][ \t]*[A-Za-z0-9]{32,}",
        Severity::High,
    ),
    // Chat / collab
    (
        "slack-bot-token",
        r"xoxb-[0-9]+-[0-9]+-[A-Za-z0-9]+",
        Severity::High,
    ),
    (
        "slack-user-token",
        r"xoxp-[0-9]+-[0-9]+-[0-9]+-[a-f0-9]+",
        Severity::High,
    ),
    (
        "slack-webhook",
        r"https://hooks\.slack\.com/services/T[A-Z0-9]+/B[A-Z0-9]+/[A-Za-z0-9]+",
        Severity::Medium,
    ),
    (
        "discord-bot-token",
        r"[MN][A-Za-z\d]{23}\.[\w-]{6}\.[\w-]{27}",
        Severity::High,
    ),
    (
        "telegram-bot-token",
        r"[0-9]{8,10}:AA[A-Za-z0-9_\-]{33}",
        Severity::High,
    ),
    // Payments
    (
        "stripe-secret-key",
        r"sk_live_[0-9a-zA-Z]{24}",
        Severity::Critical,
    ),
    (
        "stripe-restricted-key",
        r"rk_live_[0-9a-zA-Z]{24}",
        Severity::High,
    ),
    (
        "stripe-publishable-key",
        r"pk_live_[0-9a-zA-Z]{24}",
        Severity::Medium,
    ),
    (
        "square-access-token",
        r"sq0(atp|csp)-[0-9A-Za-z\-_]{22}",
        Severity::High,
    ),
    (
        "paypal-braintree-access-token",
        r"access_token\$production\$[0-9a-z]{16}\$[0-9a-f]{32}",
        Severity::Critical,
    ),
    // Comms / SMS
    ("twilio-account-sid", r"AC[a-f0-9]{32}", Severity::High),
    ("twilio-auth-token", r"SK[a-f0-9]{32}", Severity::High),
    (
        "sendgrid-api-key",
        r"SG\.[A-Za-z0-9\-_]{22}\.[A-Za-z0-9\-_]{43}",
        Severity::High,
    ),
    ("mailgun-api-key", r"key-[a-f0-9]{32}", Severity::High),
    (
        "postmark-token",
        r"[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}",
        Severity::Low,
    ),
    // Generic credentials
    (
        "jwt",
        r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        Severity::High,
    ),
    (
        "private-key",
        r"-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP) )?PRIVATE KEY-----",
        Severity::Critical,
    ),
    (
        "ssh-public-key",
        r"ssh-(rsa|ed25519|dss) [A-Za-z0-9+/=]+",
        Severity::Low,
    ),
    (
        "pkcs8-private",
        r"-----BEGIN ENCRYPTED PRIVATE KEY-----",
        Severity::Critical,
    ),
    (
        "htpasswd-bcrypt",
        r"\$2[ayb]\$[0-9]{2}\$[A-Za-z0-9./]{53}",
        Severity::Medium,
    ),
    // CI/CD
    ("npm-access-token", r"npm_[A-Za-z0-9]{36}", Severity::High),
    (
        "pypi-upload-token",
        r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_\-]{50,}",
        Severity::High,
    ),
    (
        "docker-config-auth",
        r#""auths"\s*:\s*\{[^}]*"auth"\s*:\s*"[A-Za-z0-9+/=]+""#,
        Severity::High,
    ),
    (
        "circleci-personal-token",
        r"CCIPRJ_[A-Za-z0-9_]{40}",
        Severity::High,
    ),
    (
        "travis-ci-token",
        r"travis[\-_]?token[ \t]*[:=][ \t]*[a-zA-Z0-9]{22}",
        Severity::Medium,
    ),
    // Database / messaging
    (
        "postgres-url",
        r"postgres(ql)?://[^:\s]+:[^@\s]+@[^/\s]+/[^\s]+",
        Severity::High,
    ),
    (
        "mongo-srv-url",
        r"mongodb\+srv://[^:\s]+:[^@\s]+@",
        Severity::High,
    ),
    (
        "redis-auth-url",
        r"redis://[^:\s]+:[^@\s]+@",
        Severity::Medium,
    ),
    // Misc
    (
        "generic-api-key",
        r#"(?i)api[_\-]?key[ \t]*[:=][ \t]*["']?[A-Za-z0-9_\-]{32,}["']?"#,
        Severity::Medium,
    ),
    (
        "generic-password",
        r#"(?i)password[ \t]*[:=][ \t]*["'][^"']{12,}["']"#,
        Severity::Low,
    ),
];

/// Compiled scanner.
pub struct SecretScanner {
    set: RegexSet,
    rules: Vec<RuleMeta>,
    individual: Vec<regex::Regex>,
}

impl Default for SecretScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretScanner {
    pub fn new() -> Self {
        let patterns: Vec<&str> = RULES.iter().map(|(_, p, _)| *p).collect();
        let set = RegexSet::new(&patterns).expect("built-in regex set must compile");
        let individual: Vec<regex::Regex> = patterns
            .iter()
            .map(|p| regex::Regex::new(p).expect("built-in regex must compile"))
            .collect();
        let rules: Vec<RuleMeta> = RULES
            .iter()
            .map(|(id, _, sev)| RuleMeta { id, severity: *sev })
            .collect();
        Self {
            set,
            rules,
            individual,
        }
    }

    pub fn pattern_count(&self) -> usize {
        self.rules.len()
    }

    /// Scan a buffer, returning every secret hit (deduped per rule × line).
    pub fn scan(&self, content: &str, path: &str) -> Vec<SecretFinding> {
        let mut out = Vec::new();
        for (line_idx, line) in content.lines().enumerate() {
            let matches = self.set.matches(line);
            if !matches.matched_any() {
                continue;
            }
            for ri in matches.iter() {
                let re = &self.individual[ri];
                if let Some(m) = re.find(line) {
                    let sample_end = m.as_str().len().min(6);
                    let masked = format!("{}…", &m.as_str()[..sample_end]);
                    out.push(SecretFinding {
                        rule_id: self.rules[ri].id.to_string(),
                        severity: self.rules[ri].severity,
                        file: path.to_string(),
                        line: line_idx + 1,
                        sample: masked,
                    });
                }
            }
        }
        out
    }
}

impl SecretDetector for SecretScanner {
    fn scan(&self, content: &str, path: &str) -> Vec<SecretFinding> {
        Self::scan(self, content, path)
    }
}
