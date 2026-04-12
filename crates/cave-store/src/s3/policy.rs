//! S3 bucket policy evaluation.
//!
//! Parses a subset of the AWS IAM policy language to allow/deny S3 actions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketPolicy {
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "Statement")]
    pub statements: Vec<PolicyStatement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatement {
    #[serde(rename = "Sid", default)]
    pub sid: String,
    #[serde(rename = "Effect")]
    pub effect: Effect,
    #[serde(rename = "Principal")]
    pub principal: PrincipalDef,
    #[serde(rename = "Action")]
    pub action: OneOrMany,
    #[serde(rename = "Resource")]
    pub resource: OneOrMany,
    #[serde(rename = "Condition", default)]
    pub condition: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrincipalDef {
    Wildcard(String),
    Aws { #[serde(rename = "AWS")] aws: OneOrMany },
    Service { #[serde(rename = "Service")] service: OneOrMany },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn as_slice(&self) -> Vec<&str> {
        match self {
            OneOrMany::One(s) => vec![s.as_str()],
            OneOrMany::Many(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

/// Request context for policy evaluation.
pub struct PolicyContext<'a> {
    pub principal: &'a str,   // IAM ARN or "*"
    pub action: &'a str,      // e.g. "s3:GetObject"
    pub resource: &'a str,    // e.g. "arn:aws:s3:::my-bucket/my-key"
}

/// Evaluate if the request is allowed by a bucket policy.
/// Returns None if no matching statement found (implicitly deny).
pub fn evaluate(policy: &BucketPolicy, ctx: &PolicyContext) -> Option<Effect> {
    let mut result: Option<Effect> = None;

    for stmt in &policy.statements {
        if !matches_principal(&stmt.principal, ctx.principal) {
            continue;
        }
        if !matches_pattern_list(stmt.action.as_slice(), ctx.action) {
            continue;
        }
        if !matches_pattern_list(stmt.resource.as_slice(), ctx.resource) {
            continue;
        }
        // A Deny always wins
        if stmt.effect == Effect::Deny {
            return Some(Effect::Deny);
        }
        result = Some(Effect::Allow);
    }
    result
}

fn matches_principal(principal: &PrincipalDef, actor: &str) -> bool {
    match principal {
        PrincipalDef::Wildcard(s) => s == "*" || s == actor,
        PrincipalDef::Aws { aws } => matches_pattern_list(aws.as_slice(), actor),
        PrincipalDef::Service { service } => matches_pattern_list(service.as_slice(), actor),
    }
}

fn matches_pattern_list(patterns: Vec<&str>, value: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, value))
}

/// Simple glob matching supporting `*` and `?`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, &t)
}

fn glob_match_inner(p: &[char], t: &[char]) -> bool {
    match (p.first(), t.first()) {
        (None, None) => true,
        (Some(&'*'), _) => {
            // Match zero or more chars
            glob_match_inner(&p[1..], t)
                || (!t.is_empty() && glob_match_inner(p, &t[1..]))
        }
        (Some(&'?'), Some(_)) => glob_match_inner(&p[1..], &t[1..]),
        (Some(pc), Some(tc)) if pc == tc => glob_match_inner(&p[1..], &t[1..]),
        _ => false,
    }
}

/// Build a standard S3 resource ARN.
pub fn s3_resource_arn(bucket: &str, key: Option<&str>) -> String {
    match key {
        Some(k) => format!("arn:aws:s3:::{bucket}/{k}"),
        None => format!("arn:aws:s3:::{bucket}"),
    }
}
