//! ratelimit_service plugin — Envoy global rate-limit service parity.
//!
//! Differs from the existing local `rate_limiting.rs` (Kong-style per-gateway
//! counter) in that it builds **descriptors** the gateway sends to a separate
//! rate-limit service (RLS), then applies the RLS decision. Envoy parity:
//! `envoy.filters.http.ratelimit` + `envoy.config.ratelimit.v3.RateLimitService`.
//!
//! A descriptor is an ordered list of (key, value) entries derived from
//! request data: source IP, route key, headers, request labels. The RLS
//! looks each up against its rule tree and replies OK / OVER_LIMIT.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RateLimitDescriptor {
    pub entries: Vec<DescriptorEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DescriptorEntry {
    pub key: String,
    pub value: String,
}

impl RateLimitDescriptor {
    pub fn new() -> Self {
        Self { entries: vec![] }
    }

    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries
            .push(DescriptorEntry { key: key.into(), value: value.into() });
    }

    /// Stable canonical form for caching / hashing — sorts by key, then value.
    pub fn canonical(&self) -> String {
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        sorted
            .into_iter()
            .map(|e| format!("{}={}", e.key, e.value))
            .collect::<Vec<_>>()
            .join(";")
    }
}

impl Default for RateLimitDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Action — how to derive a descriptor entry from the request ──────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    SourceCluster {
        descriptor_key: Option<String>,
    },
    DestinationCluster {
        descriptor_key: Option<String>,
    },
    /// Drives the descriptor from a request header.
    RequestHeaders {
        header_name: String,
        descriptor_key: String,
        skip_if_absent: bool,
    },
    /// Constant value that always emits.
    GenericKey {
        descriptor_value: String,
        descriptor_key: Option<String>,
    },
    /// Source-IP / remote address.
    RemoteAddress {
        descriptor_key: Option<String>,
    },
    /// The matched route (Envoy `dynamic_metadata`-driven key).
    RouteKey {
        descriptor_key: Option<String>,
    },
    /// Header-value match against a regex; emits the captured group.
    HeaderValueMatch {
        descriptor_key: String,
        descriptor_value: String,
        header_name: String,
        regex: String,
        expect_match: bool,
    },
    /// Query parameter value match.
    QueryParameter {
        descriptor_key: String,
        parameter_name: String,
        skip_if_absent: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub domain: String,
    pub stage: u32,
    pub timeout: Duration,
    pub failure_mode_deny: bool,
    pub rate_limited_as_resource_exhausted: bool,
    pub disable_x_envoy_ratelimited_header: bool,
    pub rate_limit_service_uri: String,
    pub actions: Vec<Vec<Action>>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            domain: "cave-gateway".into(),
            stage: 0,
            timeout: Duration::from_millis(20),
            failure_mode_deny: false,
            rate_limited_as_resource_exhausted: true,
            disable_x_envoy_ratelimited_header: false,
            rate_limit_service_uri: "".into(),
            actions: vec![],
        }
    }
}

impl RateLimitConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.domain.is_empty() {
            return Err("domain is empty".into());
        }
        if self.timeout.is_zero() {
            return Err("timeout must be > 0".into());
        }
        if self.actions.is_empty() {
            return Err("at least one action group required".into());
        }
        for group in &self.actions {
            if group.is_empty() {
                return Err("action group is empty".into());
            }
            for a in group {
                if let Action::HeaderValueMatch { regex, .. } = a {
                    regex::Regex::new(regex)
                        .map_err(|e| format!("invalid regex '{}': {}", regex, e))?;
                }
            }
        }
        Ok(())
    }
}

// ── Inputs the actions read from ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DescriptorInputs<'a> {
    pub source_cluster: Option<&'a str>,
    pub destination_cluster: Option<&'a str>,
    pub headers: &'a HashMap<String, String>,
    pub query: &'a HashMap<String, String>,
    pub source_ip: &'a str,
    pub route_key: Option<&'a str>,
}

/// Build all descriptors for a given request (one per action group).
pub fn build_descriptors(
    cfg: &RateLimitConfig,
    inputs: &DescriptorInputs<'_>,
) -> Vec<RateLimitDescriptor> {
    let mut out = Vec::with_capacity(cfg.actions.len());
    for group in &cfg.actions {
        let mut desc = RateLimitDescriptor::new();
        let mut group_complete = true;
        for a in group {
            if !apply_action(a, inputs, &mut desc) {
                group_complete = false;
                break;
            }
        }
        if group_complete && !desc.entries.is_empty() {
            out.push(desc);
        }
    }
    out
}

fn apply_action(
    action: &Action,
    inputs: &DescriptorInputs<'_>,
    desc: &mut RateLimitDescriptor,
) -> bool {
    match action {
        Action::SourceCluster { descriptor_key } => {
            let key = descriptor_key.as_deref().unwrap_or("source_cluster");
            if let Some(v) = inputs.source_cluster {
                desc.push(key, v);
                true
            } else {
                false
            }
        }
        Action::DestinationCluster { descriptor_key } => {
            let key = descriptor_key.as_deref().unwrap_or("destination_cluster");
            if let Some(v) = inputs.destination_cluster {
                desc.push(key, v);
                true
            } else {
                false
            }
        }
        Action::RequestHeaders {
            header_name,
            descriptor_key,
            skip_if_absent,
        } => {
            let lower = header_name.to_ascii_lowercase();
            match inputs.headers.get(&lower) {
                Some(v) => {
                    desc.push(descriptor_key.as_str(), v);
                    true
                }
                None => *skip_if_absent,
            }
        }
        Action::GenericKey {
            descriptor_value,
            descriptor_key,
        } => {
            let key = descriptor_key.as_deref().unwrap_or("generic_key");
            desc.push(key, descriptor_value.as_str());
            true
        }
        Action::RemoteAddress { descriptor_key } => {
            let key = descriptor_key.as_deref().unwrap_or("remote_address");
            desc.push(key, inputs.source_ip);
            true
        }
        Action::RouteKey { descriptor_key } => {
            let key = descriptor_key.as_deref().unwrap_or("route_key");
            match inputs.route_key {
                Some(v) => {
                    desc.push(key, v);
                    true
                }
                None => false,
            }
        }
        Action::HeaderValueMatch {
            descriptor_key,
            descriptor_value,
            header_name,
            regex,
            expect_match,
        } => {
            let lower = header_name.to_ascii_lowercase();
            let v = inputs.headers.get(&lower);
            let re = match regex::Regex::new(regex) {
                Ok(r) => r,
                Err(_) => return false,
            };
            let matched = v.map(|s| re.is_match(s)).unwrap_or(false);
            if matched == *expect_match {
                desc.push(descriptor_key.as_str(), descriptor_value.as_str());
                true
            } else {
                false
            }
        }
        Action::QueryParameter {
            descriptor_key,
            parameter_name,
            skip_if_absent,
        } => {
            match inputs.query.get(parameter_name) {
                Some(v) => {
                    desc.push(descriptor_key.as_str(), v);
                    true
                }
                None => *skip_if_absent,
            }
        }
    }
}

// ── Decisions returned from the RLS ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorStatus {
    Ok,
    OverLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitResponse {
    pub overall: DescriptorStatus,
    pub statuses: Vec<DescriptorStatus>,
    /// Quotas the RLS optionally returns — `Some` for OK with quota, None for OK static.
    pub quotas: Vec<Option<Quota>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quota {
    pub requests_per_unit: u32,
    pub unit: TimeUnit,
    pub remaining: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitOutcome {
    Allow {
        headers: Vec<(String, String)>,
    },
    Deny {
        status: u16,
        headers: Vec<(String, String)>,
    },
}

/// Convert an RLS response + config into a final outcome.
pub fn apply_response(
    cfg: &RateLimitConfig,
    response: Result<RateLimitResponse, String>,
) -> RateLimitOutcome {
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return if cfg.failure_mode_deny {
                RateLimitOutcome::Deny {
                    status: 503,
                    headers: vec![],
                }
            } else {
                RateLimitOutcome::Allow { headers: vec![] }
            };
        }
    };

    match response.overall {
        DescriptorStatus::Ok => {
            let mut headers = vec![];
            // RFC 9745 / draft RateLimit headers — emit when the RLS provided
            // quota information for any descriptor.
            if let Some(q) = response.quotas.iter().flatten().next() {
                headers.push((
                    "x-ratelimit-limit".into(),
                    q.requests_per_unit.to_string(),
                ));
                headers.push((
                    "x-ratelimit-remaining".into(),
                    q.remaining.to_string(),
                ));
                headers.push((
                    "x-ratelimit-reset-unit".into(),
                    format!("{:?}", q.unit).to_lowercase(),
                ));
            }
            RateLimitOutcome::Allow { headers }
        }
        DescriptorStatus::OverLimit => {
            let status = if cfg.rate_limited_as_resource_exhausted {
                429
            } else {
                429
            };
            let mut headers = vec![];
            if !cfg.disable_x_envoy_ratelimited_header {
                headers.push(("x-envoy-ratelimited".into(), "true".into()));
            }
            RateLimitOutcome::Deny { status, headers }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(headers: HashMap<String, String>) -> DescriptorInputs<'static> {
        // Box-leak the headers so the lifetime works in this helper.
        let headers: &'static HashMap<String, String> = Box::leak(Box::new(headers));
        let query: &'static HashMap<String, String> = Box::leak(Box::new(HashMap::new()));
        DescriptorInputs {
            source_cluster: Some("inbound|0|http|edge"),
            destination_cluster: Some("outbound|80||svc"),
            headers,
            query,
            source_ip: "10.0.0.1",
            route_key: Some("route-foo"),
        }
    }

    fn cfg_with(actions: Vec<Vec<Action>>) -> RateLimitConfig {
        RateLimitConfig {
            domain: "cave".into(),
            stage: 0,
            timeout: Duration::from_millis(20),
            failure_mode_deny: false,
            rate_limited_as_resource_exhausted: true,
            disable_x_envoy_ratelimited_header: false,
            rate_limit_service_uri: "grpc://rls:8081".into(),
            actions,
        }
    }

    // ── Descriptor canonical form ──

    #[test]
    fn descriptor_canonical_sorted() {
        let mut d = RateLimitDescriptor::new();
        d.push("z", "1");
        d.push("a", "2");
        d.push("m", "3");
        assert_eq!(d.canonical(), "a=2;m=3;z=1");
    }

    #[test]
    fn descriptor_canonical_empty() {
        assert_eq!(RateLimitDescriptor::new().canonical(), "");
    }

    // ── Config validation ──

    #[test]
    fn config_default_invalid_no_actions() {
        let mut c = RateLimitConfig::default();
        c.actions = vec![];
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_empty_domain_rejects() {
        let mut c = RateLimitConfig::default();
        c.domain = "".into();
        c.actions = vec![vec![Action::RemoteAddress { descriptor_key: None }]];
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_zero_timeout_rejects() {
        let mut c = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        c.timeout = Duration::ZERO;
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_empty_action_group_rejects() {
        let c = cfg_with(vec![vec![]]);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_invalid_regex_rejects() {
        let c = cfg_with(vec![vec![Action::HeaderValueMatch {
            descriptor_key: "k".into(),
            descriptor_value: "v".into(),
            header_name: "X".into(),
            regex: "[".into(),
            expect_match: true,
        }]]);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_valid_regex_passes() {
        let c = cfg_with(vec![vec![Action::HeaderValueMatch {
            descriptor_key: "k".into(),
            descriptor_value: "v".into(),
            header_name: "X".into(),
            regex: r"^v\d+$".into(),
            expect_match: true,
        }]]);
        c.validate().unwrap();
    }

    // ── Action: SourceCluster / DestinationCluster ──

    #[test]
    fn action_source_cluster_emits() {
        let cfg = cfg_with(vec![vec![Action::SourceCluster {
            descriptor_key: None,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].entries[0].key, "source_cluster");
    }

    #[test]
    fn action_destination_cluster_emits_with_custom_key() {
        let cfg = cfg_with(vec![vec![Action::DestinationCluster {
            descriptor_key: Some("dst".into()),
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs[0].entries[0].key, "dst");
    }

    // ── Action: RequestHeaders ──

    #[test]
    fn action_request_headers_present() {
        let mut h = HashMap::new();
        h.insert("x-tenant".into(), "acme".into());
        let cfg = cfg_with(vec![vec![Action::RequestHeaders {
            header_name: "X-Tenant".into(),
            descriptor_key: "tenant".into(),
            skip_if_absent: false,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(h));
        assert_eq!(descs[0].entries[0].key, "tenant");
        assert_eq!(descs[0].entries[0].value, "acme");
    }

    #[test]
    fn action_request_headers_absent_skips_group() {
        let cfg = cfg_with(vec![vec![Action::RequestHeaders {
            header_name: "X-Tenant".into(),
            descriptor_key: "tenant".into(),
            skip_if_absent: false,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert!(descs.is_empty());
    }

    #[test]
    fn action_request_headers_absent_with_skip_if_absent_continues() {
        let cfg = cfg_with(vec![vec![
            Action::RequestHeaders {
                header_name: "X-Tenant".into(),
                descriptor_key: "tenant".into(),
                skip_if_absent: true,
            },
            Action::GenericKey {
                descriptor_value: "fallback".into(),
                descriptor_key: None,
            },
        ]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs[0].entries.len(), 1);
        assert_eq!(descs[0].entries[0].value, "fallback");
    }

    // ── Action: GenericKey ──

    #[test]
    fn action_generic_key_default_key() {
        let cfg = cfg_with(vec![vec![Action::GenericKey {
            descriptor_value: "premium".into(),
            descriptor_key: None,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs[0].entries[0].key, "generic_key");
        assert_eq!(descs[0].entries[0].value, "premium");
    }

    // ── Action: RemoteAddress ──

    #[test]
    fn action_remote_address() {
        let cfg = cfg_with(vec![vec![Action::RemoteAddress {
            descriptor_key: None,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs[0].entries[0].key, "remote_address");
        assert_eq!(descs[0].entries[0].value, "10.0.0.1");
    }

    // ── Action: RouteKey ──

    #[test]
    fn action_route_key_present() {
        let cfg = cfg_with(vec![vec![Action::RouteKey {
            descriptor_key: None,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs[0].entries[0].value, "route-foo");
    }

    // ── Action: HeaderValueMatch ──

    #[test]
    fn action_header_value_match_hit() {
        let mut h = HashMap::new();
        h.insert("user-agent".into(), "curl/7.0".into());
        let cfg = cfg_with(vec![vec![Action::HeaderValueMatch {
            descriptor_key: "ua".into(),
            descriptor_value: "curl".into(),
            header_name: "User-Agent".into(),
            regex: r"^curl/".into(),
            expect_match: true,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(h));
        assert_eq!(descs[0].entries[0].value, "curl");
    }

    #[test]
    fn action_header_value_match_miss_skips_group() {
        let mut h = HashMap::new();
        h.insert("user-agent".into(), "Mozilla".into());
        let cfg = cfg_with(vec![vec![Action::HeaderValueMatch {
            descriptor_key: "ua".into(),
            descriptor_value: "curl".into(),
            header_name: "User-Agent".into(),
            regex: r"^curl/".into(),
            expect_match: true,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(h));
        assert!(descs.is_empty());
    }

    #[test]
    fn action_header_value_match_inverted() {
        let mut h = HashMap::new();
        h.insert("x-flag".into(), "off".into());
        let cfg = cfg_with(vec![vec![Action::HeaderValueMatch {
            descriptor_key: "flag".into(),
            descriptor_value: "off".into(),
            header_name: "X-Flag".into(),
            regex: r"^on$".into(),
            expect_match: false,
        }]]);
        let descs = build_descriptors(&cfg, &inputs(h));
        assert_eq!(descs[0].entries[0].key, "flag");
    }

    // ── Action: QueryParameter ──

    #[test]
    fn action_query_parameter_present() {
        let mut q = HashMap::new();
        q.insert("token_type".into(), "premium".into());
        let q: &'static HashMap<String, String> = Box::leak(Box::new(q));
        let h: &'static HashMap<String, String> = Box::leak(Box::new(HashMap::new()));
        let inputs = DescriptorInputs {
            source_cluster: None,
            destination_cluster: None,
            headers: h,
            query: q,
            source_ip: "1.1.1.1",
            route_key: None,
        };
        let cfg = cfg_with(vec![vec![Action::QueryParameter {
            descriptor_key: "tier".into(),
            parameter_name: "token_type".into(),
            skip_if_absent: false,
        }]]);
        let descs = build_descriptors(&cfg, &inputs);
        assert_eq!(descs[0].entries[0].key, "tier");
        assert_eq!(descs[0].entries[0].value, "premium");
    }

    // ── Multi-group ──

    #[test]
    fn multiple_action_groups_emit_multiple_descriptors() {
        let cfg = cfg_with(vec![
            vec![Action::RemoteAddress { descriptor_key: None }],
            vec![Action::GenericKey {
                descriptor_value: "free".into(),
                descriptor_key: None,
            }],
        ]);
        let descs = build_descriptors(&cfg, &inputs(HashMap::new()));
        assert_eq!(descs.len(), 2);
    }

    // ── apply_response ──

    #[test]
    fn apply_response_ok_no_quota() {
        let cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        let r = Ok(RateLimitResponse {
            overall: DescriptorStatus::Ok,
            statuses: vec![DescriptorStatus::Ok],
            quotas: vec![None],
        });
        match apply_response(&cfg, r) {
            RateLimitOutcome::Allow { headers } => assert!(headers.is_empty()),
            _ => panic!("expected allow"),
        }
    }

    #[test]
    fn apply_response_ok_with_quota_emits_headers() {
        let cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        let r = Ok(RateLimitResponse {
            overall: DescriptorStatus::Ok,
            statuses: vec![DescriptorStatus::Ok],
            quotas: vec![Some(Quota {
                requests_per_unit: 100,
                unit: TimeUnit::Minute,
                remaining: 42,
            })],
        });
        match apply_response(&cfg, r) {
            RateLimitOutcome::Allow { headers } => {
                assert!(headers.iter().any(|(k, v)| k == "x-ratelimit-limit" && v == "100"));
                assert!(headers.iter().any(|(k, v)| k == "x-ratelimit-remaining" && v == "42"));
            }
            _ => panic!("expected allow"),
        }
    }

    #[test]
    fn apply_response_over_limit_denies_429() {
        let cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        let r = Ok(RateLimitResponse {
            overall: DescriptorStatus::OverLimit,
            statuses: vec![DescriptorStatus::OverLimit],
            quotas: vec![],
        });
        match apply_response(&cfg, r) {
            RateLimitOutcome::Deny { status, headers } => {
                assert_eq!(status, 429);
                assert!(headers.iter().any(|(k, v)| k == "x-envoy-ratelimited" && v == "true"));
            }
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn apply_response_disable_envoy_header() {
        let mut cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        cfg.disable_x_envoy_ratelimited_header = true;
        let r = Ok(RateLimitResponse {
            overall: DescriptorStatus::OverLimit,
            statuses: vec![DescriptorStatus::OverLimit],
            quotas: vec![],
        });
        match apply_response(&cfg, r) {
            RateLimitOutcome::Deny { headers, .. } => {
                assert!(!headers.iter().any(|(k, _)| k == "x-envoy-ratelimited"));
            }
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn apply_response_failure_mode_allow_default() {
        let cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        let r = Err("rls down".into());
        match apply_response(&cfg, r) {
            RateLimitOutcome::Allow { .. } => {}
            _ => panic!("expected allow on failure"),
        }
    }

    #[test]
    fn apply_response_failure_mode_deny() {
        let mut cfg = cfg_with(vec![vec![Action::RemoteAddress { descriptor_key: None }]]);
        cfg.failure_mode_deny = true;
        let r = Err("rls down".into());
        match apply_response(&cfg, r) {
            RateLimitOutcome::Deny { status, .. } => assert_eq!(status, 503),
            _ => panic!("expected deny"),
        }
    }

    // ── Serde round-trips ──

    #[test]
    fn rate_limit_config_json_roundtrip() {
        let c = cfg_with(vec![vec![
            Action::RemoteAddress { descriptor_key: None },
            Action::RequestHeaders {
                header_name: "X-Tenant".into(),
                descriptor_key: "tenant".into(),
                skip_if_absent: false,
            },
        ]]);
        let s = serde_json::to_string(&c).unwrap();
        let c2: RateLimitConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn descriptor_json_roundtrip() {
        let mut d = RateLimitDescriptor::new();
        d.push("a", "1");
        d.push("b", "2");
        let s = serde_json::to_string(&d).unwrap();
        let d2: RateLimitDescriptor = serde_json::from_str(&s).unwrap();
        assert_eq!(d, d2);
    }

    #[test]
    fn quota_unit_json_roundtrip() {
        let units = vec![TimeUnit::Second, TimeUnit::Minute, TimeUnit::Hour, TimeUnit::Day];
        let s = serde_json::to_string(&units).unwrap();
        let u2: Vec<TimeUnit> = serde_json::from_str(&s).unwrap();
        assert_eq!(units, u2);
    }
}
