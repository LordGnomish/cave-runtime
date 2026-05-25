// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge token API — parity with `src/lib/edge/*` (Unleash v5.0.0 / Unleash Edge).
//!
//! Edge tokens are scoped SDK tokens consumed by the standalone
//! Unleash Edge proxy. They carry a project scope, an environment, and
//! a token type (CLIENT / FRONTEND / ADMIN). The format mirrors
//! Unleash's wire format:
//!
//!   `<project>:<env>.<32-hex-secret>` for non-admin tokens
//!   `*:*.<32-hex-secret>`             for wildcard tokens
//!   `*:*.<32-hex-secret>` (with type=ADMIN) for admin tokens

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EdgeTokenType {
    Client,
    Frontend,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeToken {
    pub token: String,
    pub project: String,
    pub environment: String,
    #[serde(rename = "type")]
    pub token_type: EdgeTokenType,
    pub alias: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EdgeTokenError {
    #[error("invalid token format")]
    InvalidFormat,
    #[error("invalid secret length (expected 32 hex chars)")]
    InvalidSecretLength,
    #[error("admin token must be wildcard scoped (*:*)")]
    AdminMustBeWildcard,
}

/// Issue a new token. Secret is derived from `seed_bytes` so callers in
/// tests can produce deterministic tokens; in production the runtime
/// passes a CSPRNG draw.
pub fn issue_token(
    project: &str,
    environment: &str,
    token_type: EdgeTokenType,
    seed_bytes: &[u8],
) -> Result<EdgeToken, EdgeTokenError> {
    if token_type == EdgeTokenType::Admin && (project != "*" || environment != "*") {
        return Err(EdgeTokenError::AdminMustBeWildcard);
    }
    let mut h = Sha256::new();
    h.update(project.as_bytes());
    h.update(b":");
    h.update(environment.as_bytes());
    h.update(b"|");
    h.update(seed_bytes);
    let secret_full: String = h.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    let secret = &secret_full[..32]; // 128-bit body, hex-encoded
    let token = format!("{}:{}.{}", project, environment, secret);
    Ok(EdgeToken {
        token,
        project: project.to_string(),
        environment: environment.to_string(),
        token_type,
        alias: None,
    })
}

/// Parse `<project>:<env>.<32-hex>` back into structured form. Used by
/// admin surfaces to validate manual token entries.
pub fn parse_token(raw: &str, declared_type: EdgeTokenType) -> Result<EdgeToken, EdgeTokenError> {
    let (scope, secret) = raw.split_once('.').ok_or(EdgeTokenError::InvalidFormat)?;
    let (project, environment) = scope.split_once(':').ok_or(EdgeTokenError::InvalidFormat)?;
    if project.is_empty() || environment.is_empty() {
        return Err(EdgeTokenError::InvalidFormat);
    }
    if secret.len() != 32 || !secret.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(EdgeTokenError::InvalidSecretLength);
    }
    if declared_type == EdgeTokenType::Admin && (project != "*" || environment != "*") {
        return Err(EdgeTokenError::AdminMustBeWildcard);
    }
    Ok(EdgeToken {
        token: raw.to_string(),
        project: project.to_string(),
        environment: environment.to_string(),
        token_type: declared_type,
        alias: None,
    })
}

/// Wildcard-aware token-scope match — used by Edge to decide whether a
/// stored token grants access to a requested (project, environment) pair.
pub fn token_matches(token: &EdgeToken, project: &str, environment: &str) -> bool {
    let project_ok = token.project == "*" || token.project == project;
    let env_ok = token.environment == "*" || token.environment == environment;
    project_ok && env_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_client_token_well_formed() {
        let t = issue_token("default", "production", EdgeTokenType::Client, b"seed1").unwrap();
        assert!(t.token.starts_with("default:production."));
        let secret = t.token.split('.').nth(1).unwrap();
        assert_eq!(secret.len(), 32);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(t.token_type, EdgeTokenType::Client);
    }

    #[test]
    fn issue_admin_requires_wildcard() {
        let err = issue_token("default", "prod", EdgeTokenType::Admin, b"x").unwrap_err();
        assert_eq!(err, EdgeTokenError::AdminMustBeWildcard);
    }

    #[test]
    fn admin_wildcard_token_ok() {
        let t = issue_token("*", "*", EdgeTokenType::Admin, b"x").unwrap();
        assert!(t.token.starts_with("*:*."));
    }

    #[test]
    fn parse_token_round_trip() {
        let issued = issue_token("p1", "stg", EdgeTokenType::Frontend, b"k").unwrap();
        let parsed = parse_token(&issued.token, EdgeTokenType::Frontend).unwrap();
        assert_eq!(parsed.project, "p1");
        assert_eq!(parsed.environment, "stg");
        assert_eq!(parsed.token_type, EdgeTokenType::Frontend);
    }

    #[test]
    fn parse_rejects_missing_dot() {
        assert_eq!(
            parse_token("default:prod-nosecret", EdgeTokenType::Client),
            Err(EdgeTokenError::InvalidFormat)
        );
    }

    #[test]
    fn parse_rejects_bad_secret_length() {
        assert_eq!(
            parse_token("default:prod.abcd", EdgeTokenType::Client),
            Err(EdgeTokenError::InvalidSecretLength)
        );
    }

    #[test]
    fn parse_rejects_non_hex_secret() {
        let raw = format!("default:prod.{}", "z".repeat(32));
        assert_eq!(
            parse_token(&raw, EdgeTokenType::Client),
            Err(EdgeTokenError::InvalidSecretLength)
        );
    }

    #[test]
    fn parse_rejects_empty_scope() {
        let raw = format!(":prod.{}", "a".repeat(32));
        assert_eq!(
            parse_token(&raw, EdgeTokenType::Client),
            Err(EdgeTokenError::InvalidFormat)
        );
    }

    #[test]
    fn token_match_exact() {
        let t = issue_token("p1", "prod", EdgeTokenType::Client, b"s").unwrap();
        assert!(token_matches(&t, "p1", "prod"));
        assert!(!token_matches(&t, "p2", "prod"));
        assert!(!token_matches(&t, "p1", "stg"));
    }

    #[test]
    fn token_match_wildcard() {
        let t = issue_token("*", "*", EdgeTokenType::Admin, b"s").unwrap();
        assert!(token_matches(&t, "anything", "anywhere"));
    }

    #[test]
    fn token_match_project_wildcard_only() {
        let t = issue_token("*", "prod", EdgeTokenType::Client, b"s").unwrap();
        assert!(token_matches(&t, "p1", "prod"));
        assert!(!token_matches(&t, "p1", "stg"));
    }
}
