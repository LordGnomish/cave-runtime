//! WAF — OWASP CRS-style rule engine for the cave-waf-rules plugin slot.
//!
//! Implements the slice of ModSecurity / Coraza semantics needed to evaluate
//! the OWASP Core Rule Set: variables, transformations, operators, actions,
//! anomaly scoring, paranoia levels. Plus a curated bundled rule set that
//! covers the high-value categories — SQLi, XSS, command injection, path
//! traversal, scanner detection — without dragging in the full CRS YAML.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Variables ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variable {
    Method,
    RequestUri,
    RequestPath,
    RequestQuery,
    RequestHeader(String),
    RequestHeadersAll,
    Arg(String),
    ArgsAll,
    RequestBody,
    SourceIp,
    UserAgent,
    /// Server name from SNI / Host header.
    ServerName,
}

#[derive(Debug, Clone)]
pub struct WafContext<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub headers: &'a HashMap<String, String>,
    pub args: &'a HashMap<String, String>,
    pub body: Option<&'a [u8]>,
    pub source_ip: &'a str,
    pub server_name: &'a str,
}

impl<'a> WafContext<'a> {
    pub fn extract(&self, var: &Variable) -> Vec<String> {
        match var {
            Variable::Method => vec![self.method.to_string()],
            Variable::RequestUri => vec![self.uri.to_string()],
            Variable::RequestPath => vec![self.path.to_string()],
            Variable::RequestQuery => vec![self.query.to_string()],
            Variable::RequestHeader(name) => self
                .headers
                .get(&name.to_ascii_lowercase())
                .map(|v| vec![v.clone()])
                .unwrap_or_default(),
            Variable::RequestHeadersAll => self.headers.values().cloned().collect(),
            Variable::Arg(name) => self
                .args
                .get(name)
                .map(|v| vec![v.clone()])
                .unwrap_or_default(),
            Variable::ArgsAll => self.args.values().cloned().collect(),
            Variable::RequestBody => self
                .body
                .map(|b| vec![String::from_utf8_lossy(b).to_string()])
                .unwrap_or_default(),
            Variable::SourceIp => vec![self.source_ip.to_string()],
            Variable::UserAgent => vec![self
                .headers
                .get("user-agent")
                .cloned()
                .unwrap_or_default()],
            Variable::ServerName => vec![self.server_name.to_string()],
        }
    }
}

// ── Transformations ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transformation {
    Lowercase,
    Uppercase,
    UrlDecode,
    HtmlEntityDecode,
    Base64Decode,
    /// Compress consecutive whitespace into one space, trim ends.
    CompressWhitespace,
    /// Strip non-printable ASCII.
    RemoveNulls,
    /// Trim leading/trailing whitespace only.
    Trim,
    None,
}

pub fn apply_transformation(t: Transformation, input: &str) -> String {
    match t {
        Transformation::Lowercase => input.to_ascii_lowercase(),
        Transformation::Uppercase => input.to_ascii_uppercase(),
        Transformation::UrlDecode => url_decode(input),
        Transformation::HtmlEntityDecode => html_entity_decode(input),
        Transformation::Base64Decode => base64_decode_lossy(input),
        Transformation::CompressWhitespace => {
            let trimmed = input.trim();
            let mut out = String::with_capacity(trimmed.len());
            let mut prev_ws = false;
            for c in trimmed.chars() {
                if c.is_whitespace() {
                    if !prev_ws {
                        out.push(' ');
                    }
                    prev_ws = true;
                } else {
                    out.push(c);
                    prev_ws = false;
                }
            }
            out
        }
        Transformation::RemoveNulls => input.replace('\0', ""),
        Transformation::Trim => input.trim().to_string(),
        Transformation::None => input.to_string(),
    }
}

fn url_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                hex_digit(bytes[i + 1]),
                hex_digit(bytes[i + 2]),
            ) {
                out.push(((h << 4) | l) as char);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn html_entity_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(end) = bytes[i..].iter().position(|c| *c == b';') {
                let entity = &input[i + 1..i + end];
                if let Some(replacement) = match entity {
                    "lt" => Some("<"),
                    "gt" => Some(">"),
                    "amp" => Some("&"),
                    "quot" => Some("\""),
                    "apos" => Some("'"),
                    "nbsp" => Some(" "),
                    _ => None,
                } {
                    out.push_str(replacement);
                    i += end + 1;
                    continue;
                }
                if let Some(num) = entity.strip_prefix('#') {
                    let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    if let Some(c) = code.and_then(char::from_u32) {
                        out.push(c);
                        i += end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn base64_decode_lossy(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_else(|_| input.to_string())
}

// ── Operators ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Operator {
    /// Regex match (compiled per-rule at load).
    Rx { pattern: String },
    Eq { value: String },
    Contains { value: String },
    BeginsWith { value: String },
    EndsWith { value: String },
    /// Numeric comparisons treat the input as decimal int.
    Gt { value: i64 },
    Lt { value: i64 },
    /// Membership in a static set, lowercase.
    PmFromSet { values: Vec<String> },
    /// Length comparison.
    LenGt { value: usize },
}

impl Operator {
    pub fn matches(&self, input: &str) -> bool {
        match self {
            Operator::Rx { pattern } => {
                regex::Regex::new(pattern).map(|re| re.is_match(input)).unwrap_or(false)
            }
            Operator::Eq { value } => input == value,
            Operator::Contains { value } => input.contains(value),
            Operator::BeginsWith { value } => input.starts_with(value),
            Operator::EndsWith { value } => input.ends_with(value),
            Operator::Gt { value } => input.parse::<i64>().map(|v| v > *value).unwrap_or(false),
            Operator::Lt { value } => input.parse::<i64>().map(|v| v < *value).unwrap_or(false),
            Operator::PmFromSet { values } => {
                let lower = input.to_ascii_lowercase();
                values.iter().any(|v| lower.contains(&v.to_ascii_lowercase()))
            }
            Operator::LenGt { value } => input.len() > *value,
        }
    }
}

// ── Severity / Action ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Notice,
    Warning,
    Error,
    Critical,
    Emergency,
}

impl Severity {
    pub fn anomaly_score(self) -> u32 {
        match self {
            Severity::Notice => 2,
            Severity::Warning => 3,
            Severity::Error => 4,
            Severity::Critical => 5,
            Severity::Emergency => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Add to anomaly score; final decision deferred.
    Score,
    /// Immediately deny with the given status.
    Deny { status: u16 },
    /// Drop the connection (no response).
    Drop,
    /// Allow request through, bypassing later rules.
    Allow,
    /// Log only.
    Log,
}

// ── Phases ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    RequestHeaders,
    RequestBody,
    ResponseHeaders,
    ResponseBody,
}

// ── Rule ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WafRule {
    pub id: u32,
    pub message: String,
    pub severity: Severity,
    pub phase: Phase,
    pub paranoia_level: u8,
    pub variables: Vec<Variable>,
    pub transformations: Vec<Transformation>,
    pub operator: Operator,
    pub action: Action,
    /// CRS-style tags ("attack-sqli", "language-multi", …).
    pub tags: Vec<String>,
}

impl WafRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 {
            return Err("rule id 0 reserved".into());
        }
        if !(1..=4).contains(&self.paranoia_level) {
            return Err("paranoia_level must be in 1..=4".into());
        }
        if self.variables.is_empty() {
            return Err("at least one variable required".into());
        }
        if let Operator::Rx { pattern } = &self.operator {
            regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
        }
        Ok(())
    }
}

// ── Engine config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WafEngineConfig {
    pub paranoia_level: u8,
    /// Total anomaly score that triggers a deny.
    pub inbound_anomaly_threshold: u32,
    /// Status code to use when threshold is crossed (default 403).
    pub deny_status: u16,
    /// Detection-only mode logs everything but never blocks.
    pub detection_only: bool,
    /// Rule IDs to skip.
    pub excluded_rule_ids: Vec<u32>,
}

impl Default for WafEngineConfig {
    fn default() -> Self {
        Self {
            paranoia_level: 1,
            inbound_anomaly_threshold: 5,
            deny_status: 403,
            detection_only: false,
            excluded_rule_ids: vec![],
        }
    }
}

impl WafEngineConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=4).contains(&self.paranoia_level) {
            return Err("paranoia_level must be in 1..=4".into());
        }
        if !(400..=599).contains(&self.deny_status) {
            return Err("deny_status must be 4xx/5xx".into());
        }
        if self.inbound_anomaly_threshold == 0 {
            return Err("inbound_anomaly_threshold must be > 0".into());
        }
        Ok(())
    }
}

// ── Engine ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct WafEngine {
    rules: Vec<WafRule>,
}

impl WafEngine {
    pub fn new() -> Self {
        Self { rules: vec![] }
    }

    pub fn with_bundled_crs() -> Self {
        Self { rules: bundled_crs_rules() }
    }

    pub fn add_rule(&mut self, rule: WafRule) -> Result<(), String> {
        rule.validate()?;
        self.rules.push(rule);
        Ok(())
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub fn evaluate(&self, cfg: &WafEngineConfig, ctx: &WafContext<'_>) -> WafDecision {
        let mut hits: Vec<RuleHit> = Vec::new();
        let mut score: u32 = 0;
        let mut allowed_by_allow = false;
        let mut deny_status: Option<u16> = None;
        let mut drop_connection = false;

        for rule in &self.rules {
            if rule.paranoia_level > cfg.paranoia_level {
                continue;
            }
            if cfg.excluded_rule_ids.contains(&rule.id) {
                continue;
            }
            let matched = rule_matches(rule, ctx);
            if !matched {
                continue;
            }
            hits.push(RuleHit { id: rule.id, message: rule.message.clone(), severity: rule.severity });
            match &rule.action {
                Action::Score => {
                    score = score.saturating_add(rule.severity.anomaly_score());
                }
                Action::Deny { status } => {
                    deny_status = Some(*status);
                }
                Action::Drop => {
                    drop_connection = true;
                }
                Action::Allow => {
                    allowed_by_allow = true;
                    break;
                }
                Action::Log => {}
            }
        }

        if cfg.detection_only {
            return WafDecision::Allow { hits, score };
        }
        if allowed_by_allow {
            return WafDecision::Allow { hits, score };
        }
        if drop_connection {
            return WafDecision::Drop { hits, score };
        }
        if let Some(status) = deny_status {
            return WafDecision::Deny { status, hits, score };
        }
        if score >= cfg.inbound_anomaly_threshold {
            return WafDecision::Deny { status: cfg.deny_status, hits, score };
        }
        WafDecision::Allow { hits, score }
    }
}

fn rule_matches(rule: &WafRule, ctx: &WafContext<'_>) -> bool {
    for var in &rule.variables {
        let values = ctx.extract(var);
        for raw in values {
            let mut transformed = raw;
            for t in &rule.transformations {
                transformed = apply_transformation(*t, &transformed);
            }
            if rule.operator.matches(&transformed) {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleHit {
    pub id: u32,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WafDecision {
    Allow { hits: Vec<RuleHit>, score: u32 },
    Deny { status: u16, hits: Vec<RuleHit>, score: u32 },
    Drop { hits: Vec<RuleHit>, score: u32 },
}

// ── Bundled CRS-style rules (curated, paranoia 1..=2) ──────────────────────

pub fn bundled_crs_rules() -> Vec<WafRule> {
    vec![
        // 941100 — XSS basic
        WafRule {
            id: 941100,
            message: "XSS attack: <script> tag detected".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::RequestUri, Variable::ArgsAll, Variable::RequestBody],
            transformations: vec![Transformation::UrlDecode, Transformation::Lowercase],
            operator: Operator::Rx { pattern: r"<\s*script\b".into() },
            action: Action::Score,
            tags: vec!["attack-xss".into(), "paranoia-level/1".into()],
        },
        // 941200 — XSS event handler
        WafRule {
            id: 941200,
            message: "XSS attack: HTML event handler".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::ArgsAll, Variable::RequestBody, Variable::RequestUri],
            transformations: vec![Transformation::UrlDecode, Transformation::Lowercase],
            operator: Operator::Rx { pattern: r"\bon[a-z]+\s*=".into() },
            action: Action::Score,
            tags: vec!["attack-xss".into()],
        },
        // 942100 — SQLi with single quote + UNION/SELECT
        WafRule {
            id: 942100,
            message: "SQL injection attempt".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::ArgsAll, Variable::RequestBody, Variable::RequestUri],
            transformations: vec![Transformation::UrlDecode, Transformation::Lowercase],
            operator: Operator::Rx {
                pattern: r"(?:'\s*(?:or|and)\s+|\bunion\s+select\b|\bselect\s+.+\s+from\b|\bdrop\s+table\b)".into(),
            },
            action: Action::Score,
            tags: vec!["attack-sqli".into()],
        },
        // 932100 — RCE: shell metacharacters
        WafRule {
            id: 932100,
            message: "Remote command execution: shell metacharacters".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::ArgsAll, Variable::RequestBody, Variable::RequestUri],
            transformations: vec![Transformation::UrlDecode],
            operator: Operator::Rx { pattern: r"(?:[;|&`$]|\$\(|\$\{)".into() },
            action: Action::Score,
            tags: vec!["attack-rce".into()],
        },
        // 930100 — Path traversal
        WafRule {
            id: 930100,
            message: "Path traversal attack: ..".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::RequestUri, Variable::ArgsAll],
            transformations: vec![Transformation::UrlDecode, Transformation::HtmlEntityDecode],
            operator: Operator::Contains { value: "../".into() },
            action: Action::Score,
            tags: vec!["attack-lfi".into()],
        },
        // 913100 — Scanner UA detection
        WafRule {
            id: 913100,
            message: "Found user agent associated with security scanner".into(),
            severity: Severity::Warning,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::UserAgent],
            transformations: vec![Transformation::Lowercase],
            operator: Operator::PmFromSet {
                values: vec![
                    "sqlmap".into(),
                    "nikto".into(),
                    "nmap".into(),
                    "acunetix".into(),
                    "havij".into(),
                    "wpscan".into(),
                ],
            },
            action: Action::Deny { status: 403 },
            tags: vec!["attack-scanner".into()],
        },
        // 920100 — Method allowlist
        WafRule {
            id: 920100,
            message: "Disallowed method".into(),
            severity: Severity::Notice,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::Method],
            transformations: vec![],
            operator: Operator::Rx {
                pattern: r"^(?:TRACE|TRACK|CONNECT|DEBUG)$".into(),
            },
            action: Action::Deny { status: 405 },
            tags: vec!["protocol-violation".into()],
        },
        // 920310 — Excessive header length
        WafRule {
            id: 920310,
            message: "Header length unreasonable".into(),
            severity: Severity::Warning,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::RequestHeadersAll],
            transformations: vec![],
            operator: Operator::LenGt { value: 8192 },
            action: Action::Score,
            tags: vec!["protocol-violation".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for<'a>(
        method: &'a str,
        uri: &'a str,
        path: &'a str,
        query: &'a str,
        headers: &'a HashMap<String, String>,
        args: &'a HashMap<String, String>,
        body: Option<&'a [u8]>,
        ip: &'a str,
    ) -> WafContext<'a> {
        WafContext {
            method,
            uri,
            path,
            query,
            headers,
            args,
            body,
            source_ip: ip,
            server_name: "edge.local",
        }
    }

    fn h(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_string()))
            .collect()
    }

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // ── Transformations ──

    #[test]
    fn url_decode_percent() {
        assert_eq!(url_decode("a%20b"), "a b");
    }

    #[test]
    fn url_decode_plus_space() {
        assert_eq!(url_decode("a+b"), "a b");
    }

    #[test]
    fn url_decode_invalid_passes_through() {
        assert_eq!(url_decode("a%ZZb"), "a%ZZb");
    }

    #[test]
    fn html_decode_named() {
        assert_eq!(
            html_entity_decode("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"),
            "<script>alert(\"x\")</script>"
        );
    }

    #[test]
    fn html_decode_numeric_dec_and_hex() {
        assert_eq!(html_entity_decode("&#65;&#x42;"), "AB");
    }

    #[test]
    fn html_decode_unknown_passes_through() {
        assert_eq!(html_entity_decode("&unknown;"), "&unknown;");
    }

    #[test]
    fn compress_whitespace_collapses() {
        assert_eq!(
            apply_transformation(Transformation::CompressWhitespace, "  hello   world  "),
            "hello world"
        );
    }

    #[test]
    fn lowercase_transformation() {
        assert_eq!(apply_transformation(Transformation::Lowercase, "ABC"), "abc");
    }

    #[test]
    fn uppercase_transformation() {
        assert_eq!(apply_transformation(Transformation::Uppercase, "abc"), "ABC");
    }

    #[test]
    fn remove_nulls_strips() {
        assert_eq!(
            apply_transformation(Transformation::RemoveNulls, "a\0b\0c"),
            "abc"
        );
    }

    #[test]
    fn base64_decode_round_trip() {
        // "hello" → base64 → "aGVsbG8="
        assert_eq!(
            apply_transformation(Transformation::Base64Decode, "aGVsbG8="),
            "hello"
        );
    }

    #[test]
    fn base64_decode_invalid_passthrough() {
        assert_eq!(
            apply_transformation(Transformation::Base64Decode, "not-base64!"),
            "not-base64!"
        );
    }

    // ── Operators ──

    #[test]
    fn op_rx_matches() {
        assert!(Operator::Rx { pattern: r"^abc".into() }.matches("abcdef"));
        assert!(!Operator::Rx { pattern: r"^abc".into() }.matches("xyz"));
    }

    #[test]
    fn op_eq() {
        assert!(Operator::Eq { value: "abc".into() }.matches("abc"));
        assert!(!Operator::Eq { value: "abc".into() }.matches("abcd"));
    }

    #[test]
    fn op_contains() {
        assert!(Operator::Contains { value: "needle".into() }.matches("haystack with needle in it"));
    }

    #[test]
    fn op_begins_ends() {
        assert!(Operator::BeginsWith { value: "/api/".into() }.matches("/api/users"));
        assert!(Operator::EndsWith { value: ".json".into() }.matches("file.json"));
    }

    #[test]
    fn op_gt_lt_numeric() {
        assert!(Operator::Gt { value: 10 }.matches("11"));
        assert!(!Operator::Gt { value: 10 }.matches("not-a-number"));
        assert!(Operator::Lt { value: 5 }.matches("3"));
    }

    #[test]
    fn op_pm_from_set_case_insensitive() {
        let op = Operator::PmFromSet {
            values: vec!["sqlmap".into(), "nikto".into()],
        };
        assert!(op.matches("Mozilla SQLMap/1.5"));
        assert!(!op.matches("Mozilla/5.0"));
    }

    #[test]
    fn op_len_gt() {
        assert!(Operator::LenGt { value: 10 }.matches("more than ten characters"));
        assert!(!Operator::LenGt { value: 10 }.matches("short"));
    }

    // ── Variable extraction ──

    #[test]
    fn var_request_header_lowercase_key() {
        let h = h(&[("X-Tenant", "acme")]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        let vals = ctx.extract(&Variable::RequestHeader("X-Tenant".into()));
        assert_eq!(vals, vec!["acme".to_string()]);
    }

    #[test]
    fn var_args_individual() {
        let h = h(&[]);
        let a = args(&[("q", "test")]);
        let ctx = ctx_for("GET", "/?q=test", "/", "q=test", &h, &a, None, "1.2.3.4");
        assert_eq!(ctx.extract(&Variable::Arg("q".into())), vec!["test"]);
    }

    #[test]
    fn var_request_body_utf8_lossy() {
        let h = h(&[]);
        let a = args(&[]);
        let body = b"hello".as_ref();
        let ctx = ctx_for("POST", "/", "/", "", &h, &a, Some(body), "1.2.3.4");
        assert_eq!(ctx.extract(&Variable::RequestBody), vec!["hello"]);
    }

    // ── Severity ──

    #[test]
    fn severity_anomaly_scores() {
        assert_eq!(Severity::Critical.anomaly_score(), 5);
        assert!(Severity::Emergency.anomaly_score() > Severity::Notice.anomaly_score());
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Notice < Severity::Critical);
    }

    // ── Engine config ──

    #[test]
    fn engine_config_default_validates() {
        WafEngineConfig::default().validate().unwrap();
    }

    #[test]
    fn engine_config_paranoia_out_of_range_rejects() {
        let mut c = WafEngineConfig::default();
        c.paranoia_level = 5;
        assert!(c.validate().is_err());
    }

    #[test]
    fn engine_config_invalid_status_rejects() {
        let mut c = WafEngineConfig::default();
        c.deny_status = 200;
        assert!(c.validate().is_err());
    }

    // ── Rule validation ──

    #[test]
    fn rule_id_zero_rejects() {
        let mut r = WafRule {
            id: 0,
            message: "x".into(),
            severity: Severity::Notice,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::Method],
            transformations: vec![],
            operator: Operator::Eq { value: "GET".into() },
            action: Action::Log,
            tags: vec![],
        };
        assert!(r.validate().is_err());
        r.id = 1;
        r.validate().unwrap();
    }

    #[test]
    fn rule_invalid_paranoia_rejects() {
        let mut r = bundled_crs_rules()[0].clone();
        r.paranoia_level = 0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn rule_invalid_regex_rejects() {
        let mut r = bundled_crs_rules()[0].clone();
        r.operator = Operator::Rx { pattern: "[".into() };
        assert!(r.validate().is_err());
    }

    #[test]
    fn rule_no_variables_rejects() {
        let mut r = bundled_crs_rules()[0].clone();
        r.variables = vec![];
        assert!(r.validate().is_err());
    }

    // ── Engine evaluation ──

    fn engine() -> WafEngine {
        WafEngine::with_bundled_crs()
    }

    #[test]
    fn engine_clean_request_allows() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[("user-agent", "Mozilla/5.0")]);
        let a = args(&[("q", "hello")]);
        let ctx = ctx_for("GET", "/api/users", "/api/users", "q=hello", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Allow { hits, .. } => assert!(hits.is_empty()),
            other => panic!("expected allow, got {:?}", other),
        }
    }

    #[test]
    fn engine_xss_script_tag_scores_critical() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[]);
        let a = args(&[("comment", "<script>alert(1)</script>")]);
        let ctx = ctx_for("POST", "/comment", "/comment", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Deny { hits, score, status } => {
                assert!(hits.iter().any(|h| h.id == 941100));
                assert!(score >= cfg.inbound_anomaly_threshold);
                assert_eq!(status, 403);
            }
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn engine_sqli_union_select_denies() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[]);
        let a = args(&[("id", "1 UNION SELECT * FROM users")]);
        let ctx = ctx_for("GET", "/q", "/q", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Deny { hits, .. } => assert!(hits.iter().any(|h| h.id == 942100)),
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn engine_path_traversal_detected() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[]);
        let a = args(&[("file", "../../etc/passwd")]);
        let ctx = ctx_for(
            "GET",
            "/files/../../etc/passwd",
            "/files/../../etc/passwd",
            "",
            &h,
            &a,
            None,
            "1.2.3.4",
        );
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Deny { hits, .. } => assert!(hits.iter().any(|h| h.id == 930100)),
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn engine_scanner_ua_blocks_immediately() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[("user-agent", "sqlmap/1.0")]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Deny { status, hits, .. } => {
                assert_eq!(status, 403);
                assert!(hits.iter().any(|h| h.id == 913100));
            }
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn engine_disallowed_method_blocks_405() {
        let e = engine();
        let cfg = WafEngineConfig::default();
        let h = h(&[]);
        let a = args(&[]);
        let ctx = ctx_for("TRACE", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Deny { status, .. } => assert_eq!(status, 405),
            other => panic!("expected 405, got {:?}", other),
        }
    }

    #[test]
    fn engine_detection_only_does_not_deny() {
        let e = engine();
        let mut cfg = WafEngineConfig::default();
        cfg.detection_only = true;
        let h = h(&[("user-agent", "sqlmap/1.0")]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Allow { hits, .. } => {
                assert!(hits.iter().any(|h| h.id == 913100));
            }
            other => panic!("expected allow with hits, got {:?}", other),
        }
    }

    #[test]
    fn engine_excluded_rule_skipped() {
        let e = engine();
        let mut cfg = WafEngineConfig::default();
        cfg.excluded_rule_ids = vec![913100];
        let h = h(&[("user-agent", "sqlmap/1.0")]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Allow { hits, .. } => {
                assert!(!hits.iter().any(|h| h.id == 913100));
            }
            other => panic!("expected allow, got {:?}", other),
        }
    }

    #[test]
    fn engine_paranoia_filters_out_higher_levels() {
        let mut e = engine();
        // Inject a paranoia=4 rule that would otherwise hit.
        e.add_rule(WafRule {
            id: 999999,
            message: "paranoia 4 always-hit".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 4,
            variables: vec![Variable::Method],
            transformations: vec![],
            operator: Operator::Eq { value: "GET".into() },
            action: Action::Score,
            tags: vec![],
        })
        .unwrap();
        let cfg = WafEngineConfig::default(); // PL 1
        let h = h(&[]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Allow { hits, .. } => {
                assert!(!hits.iter().any(|h| h.id == 999999));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn engine_score_below_threshold_allows() {
        let mut e = WafEngine::new();
        e.add_rule(WafRule {
            id: 1001,
            message: "tiny".into(),
            severity: Severity::Notice, // score 2
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::Method],
            transformations: vec![],
            operator: Operator::Eq { value: "GET".into() },
            action: Action::Score,
            tags: vec![],
        })
        .unwrap();
        let cfg = WafEngineConfig::default(); // threshold 5
        let h = h(&[]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        match e.evaluate(&cfg, &ctx) {
            WafDecision::Allow { score, .. } => assert_eq!(score, 2),
            _ => panic!(),
        }
    }

    #[test]
    fn engine_drop_action_marks_drop() {
        let mut e = WafEngine::new();
        e.add_rule(WafRule {
            id: 2001,
            message: "drop".into(),
            severity: Severity::Critical,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::Method],
            transformations: vec![],
            operator: Operator::Eq { value: "GET".into() },
            action: Action::Drop,
            tags: vec![],
        })
        .unwrap();
        let cfg = WafEngineConfig::default();
        let h = h(&[]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "1.2.3.4");
        assert!(matches!(
            e.evaluate(&cfg, &ctx),
            WafDecision::Drop { .. }
        ));
    }

    #[test]
    fn engine_allow_action_short_circuits() {
        let mut e = WafEngine::with_bundled_crs();
        e.add_rule(WafRule {
            id: 3001,
            message: "allowlist".into(),
            severity: Severity::Notice,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::SourceIp],
            transformations: vec![],
            operator: Operator::Eq { value: "10.0.0.1".into() },
            action: Action::Allow,
            tags: vec![],
        })
        .unwrap();
        // Allow rule before bundled scanner-UA rule? Order matters; we put it
        // *after* bundled rules since add_rule appends.
        // To make it short-circuit before the scanner-UA rule, swap order.
        let mut e2 = WafEngine::new();
        e2.add_rule(WafRule {
            id: 3002,
            message: "allowlist".into(),
            severity: Severity::Notice,
            phase: Phase::RequestHeaders,
            paranoia_level: 1,
            variables: vec![Variable::SourceIp],
            transformations: vec![],
            operator: Operator::Eq { value: "10.0.0.1".into() },
            action: Action::Allow,
            tags: vec![],
        })
        .unwrap();
        for r in bundled_crs_rules() {
            e2.add_rule(r).unwrap();
        }

        let cfg = WafEngineConfig::default();
        let h = h(&[("user-agent", "sqlmap/1.0")]);
        let a = args(&[]);
        let ctx = ctx_for("GET", "/", "/", "", &h, &a, None, "10.0.0.1");
        match e2.evaluate(&cfg, &ctx) {
            WafDecision::Allow { hits, .. } => {
                // Only the allowlist rule fires; subsequent rules are skipped.
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].id, 3002);
            }
            _ => panic!("expected allow"),
        }
    }

    #[test]
    fn bundled_rules_validate_individually() {
        for r in bundled_crs_rules() {
            r.validate().unwrap_or_else(|e| panic!("{} invalid: {}", r.id, e));
        }
    }

    #[test]
    fn bundled_rule_count_is_reasonable() {
        let count = bundled_crs_rules().len();
        assert!(count >= 6 && count <= 20);
    }

    // ── JSON round-trips ──

    #[test]
    fn rule_json_roundtrip() {
        let r = bundled_crs_rules()[0].clone();
        let s = serde_json::to_string(&r).unwrap();
        let r2: WafRule = serde_json::from_str(&s).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn engine_config_json_roundtrip() {
        let c = WafEngineConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let c2: WafEngineConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn severity_json_uppercase() {
        let s = serde_json::to_string(&Severity::Critical).unwrap();
        assert_eq!(s, r#""CRITICAL""#);
    }
}
