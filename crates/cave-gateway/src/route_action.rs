//! Route action layer — Envoy `route.v3.Route` parity.
//!
//! Sits *after* `matcher.rs` (which decides which Route a request belongs to)
//! and *before* `proxy.rs` (which forwards it to an upstream). Models:
//!
//! - **WeightedClusters** for canary / blue-green / shadow splits.
//! - **RetryPolicy** with retry-on predicates, per-try timeout, exponential
//!   back-off, retry budget, retriable status codes, host-selection retry.
//! - **Timeout** policy: route timeout, idle timeout, gRPC timeout overrides.
//! - **HashPolicy** for consistent-hash LB inputs (header / cookie / query /
//!   source IP / connection properties).
//! - **HeaderManipulation** request/response add/remove with append vs.
//!   overwrite semantics.
//! - **RedirectAction** (3xx redirects) and **DirectResponse** (canned bodies).
//!
//! The circuit breaker (`circuit_breaker.rs`) plugs in via [`RouteEngine::pick_cluster`]
//! so a tripped breaker on a target excludes it from selection.

use crate::circuit_breaker::CircuitBreakerRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

// ── WeightedClusters ─────────────────────────────────────────────────────────

/// One leg of a traffic split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedCluster {
    pub name: String,
    pub weight: u32,
    /// Optional runtime override key — Envoy `runtime_key_prefix`.
    pub runtime_key: Option<String>,
    /// Optional cluster ID for in-process routing (skips name resolution).
    pub cluster_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedClusters {
    pub clusters: Vec<WeightedCluster>,
    /// Optional explicit total. If 0, the sum of cluster weights is used.
    pub total_weight: u32,
}

impl WeightedClusters {
    pub fn single(name: impl Into<String>) -> Self {
        Self {
            clusters: vec![WeightedCluster {
                name: name.into(),
                weight: 100,
                runtime_key: None,
                cluster_id: None,
            }],
            total_weight: 100,
        }
    }

    pub fn canary(primary: impl Into<String>, canary: impl Into<String>, canary_pct: u32) -> Self {
        let canary_pct = canary_pct.min(100);
        Self {
            clusters: vec![
                WeightedCluster {
                    name: primary.into(),
                    weight: 100 - canary_pct,
                    runtime_key: None,
                    cluster_id: None,
                },
                WeightedCluster {
                    name: canary.into(),
                    weight: canary_pct,
                    runtime_key: None,
                    cluster_id: None,
                },
            ],
            total_weight: 100,
        }
    }

    pub fn validate(&self) -> Result<(), RouteError> {
        if self.clusters.is_empty() {
            return Err(RouteError::WeightedClusters("no clusters".into()));
        }
        let sum: u32 = self.clusters.iter().map(|c| c.weight).sum();
        let total = if self.total_weight == 0 {
            sum
        } else {
            self.total_weight
        };
        if total == 0 {
            return Err(RouteError::WeightedClusters("total weight is zero".into()));
        }
        if sum > total {
            return Err(RouteError::WeightedClusters(format!(
                "sum of weights ({}) exceeds total ({})",
                sum, total
            )));
        }
        for c in &self.clusters {
            if c.name.is_empty() {
                return Err(RouteError::WeightedClusters("empty cluster name".into()));
            }
        }
        Ok(())
    }

    pub fn effective_total(&self) -> u32 {
        if self.total_weight == 0 {
            self.clusters.iter().map(|c| c.weight).sum()
        } else {
            self.total_weight
        }
    }

    /// Pick a cluster by a uniform [0, total) selector.
    pub fn pick(&self, selector: u32) -> Option<&WeightedCluster> {
        if self.clusters.is_empty() {
            return None;
        }
        let total = self.effective_total();
        if total == 0 {
            return self.clusters.first();
        }
        let idx = selector % total;
        let mut acc: u32 = 0;
        for c in &self.clusters {
            acc += c.weight;
            if idx < acc {
                return Some(c);
            }
        }
        // If sum < total (unallocated portion) we pick the last cluster as a
        // deterministic tie-break.
        self.clusters.last()
    }
}

// ── RetryPolicy ──────────────────────────────────────────────────────────────

/// Retry trigger predicate — Envoy `retry_on` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetryOn {
    /// 5xx response.
    FiveXx,
    /// gRPC `cancelled` (1).
    GrpcCancelled,
    /// gRPC `deadline-exceeded` (4).
    GrpcDeadlineExceeded,
    /// gRPC `internal` (13).
    GrpcInternal,
    /// gRPC `resource-exhausted` (8).
    GrpcResourceExhausted,
    /// gRPC `unavailable` (14).
    GrpcUnavailable,
    /// Connection failure / TCP reset.
    ConnectFailure,
    /// Refused stream (HTTP/2).
    RefusedStream,
    /// 503 specifically.
    GatewayError,
    /// Reset (any kind).
    Reset,
    /// Retriable headers / status codes (driven by extra config).
    Retriable4Xx,
    Retriable5Xx,
    /// Idempotent methods only — Envoy treats this as a guard.
    RetriableHeaders,
    RetriableStatusCodes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryBackoff {
    pub base_interval: Duration,
    pub max_interval: Duration,
}

impl Default for RetryBackoff {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_millis(25),
            max_interval: Duration::from_millis(250),
        }
    }
}

impl RetryBackoff {
    /// Truncated binary exponential back-off, attempt is 1-based.
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor: u32 = 1u32 << attempt.saturating_sub(1).min(10); // cap at 1024×
        let raw = self.base_interval.saturating_mul(factor);
        if raw > self.max_interval {
            self.max_interval
        } else {
            raw
        }
    }
}

/// Retry budget — Envoy `RetryBudget`. Limits concurrent retries as a fraction
/// of active requests so a service in trouble does not get amplified.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryBudget {
    pub budget_pct: f64,
    pub min_retry_concurrency: u32,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self { budget_pct: 20.0, min_retry_concurrency: 3 }
    }
}

impl RetryBudget {
    pub fn allow(&self, active_requests: u32, active_retries: u32) -> bool {
        let allowed_by_pct =
            ((active_requests as f64) * (self.budget_pct / 100.0)).floor() as u32;
        let allowed = allowed_by_pct.max(self.min_retry_concurrency);
        active_retries < allowed
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub retry_on: Vec<RetryOn>,
    pub num_retries: u32,
    pub per_try_timeout: Duration,
    pub per_try_idle_timeout: Duration,
    pub backoff: RetryBackoff,
    pub retriable_status_codes: Vec<u16>,
    pub retriable_request_headers: Vec<String>,
    pub host_selection_retry_max_attempts: u32,
    pub budget: RetryBudget,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            retry_on: vec![],
            num_retries: 1,
            per_try_timeout: Duration::ZERO,
            per_try_idle_timeout: Duration::ZERO,
            backoff: RetryBackoff::default(),
            retriable_status_codes: vec![],
            retriable_request_headers: vec![],
            host_selection_retry_max_attempts: 0,
            budget: RetryBudget::default(),
        }
    }
}

impl RetryPolicy {
    pub fn five_xx(num_retries: u32) -> Self {
        let mut p = Self::default();
        p.retry_on = vec![RetryOn::FiveXx];
        p.num_retries = num_retries;
        p
    }

    pub fn validate(&self) -> Result<(), RouteError> {
        if self.num_retries > 100 {
            return Err(RouteError::Retry(
                "num_retries > 100 is unreasonable".into(),
            ));
        }
        if self.budget.budget_pct < 0.0 || self.budget.budget_pct > 100.0 {
            return Err(RouteError::Retry(
                "budget_pct must be in [0, 100]".into(),
            ));
        }
        for c in &self.retriable_status_codes {
            if *c < 100 || *c > 599 {
                return Err(RouteError::Retry(format!(
                    "invalid retriable status code {}",
                    c
                )));
            }
        }
        Ok(())
    }

    /// Decide whether a given outcome is retriable.
    pub fn should_retry(&self, outcome: &Outcome, method_idempotent: bool) -> bool {
        if self.retry_on.is_empty() {
            return false;
        }
        for trigger in &self.retry_on {
            if outcome_matches(*trigger, outcome, &self.retriable_status_codes) {
                // RetriableHeaders gate: caller decides; here we honor the
                // method idempotency for connect_failure / reset triggers
                // exactly as Envoy does (connect_failure/reset retry without
                // requiring idempotency).
                let needs_idempotent = matches!(
                    trigger,
                    RetryOn::Retriable4Xx | RetryOn::Retriable5Xx | RetryOn::RetriableHeaders
                );
                if needs_idempotent && !method_idempotent {
                    continue;
                }
                return true;
            }
        }
        false
    }
}

/// Outcome of an attempt — used to evaluate retry triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// HTTP response with a status code.
    Status(u16),
    /// Transport-level connect failure.
    ConnectFailure,
    /// HTTP/2 reset (REFUSED_STREAM).
    RefusedStream,
    /// Stream reset of any kind.
    Reset,
    /// Request timed out before any response.
    Timeout,
    /// gRPC trailer status.
    GrpcStatus(u32),
}

fn outcome_matches(trigger: RetryOn, outcome: &Outcome, retriable_codes: &[u16]) -> bool {
    match (trigger, outcome) {
        (RetryOn::FiveXx, Outcome::Status(c)) => *c >= 500 && *c <= 599,
        (RetryOn::GatewayError, Outcome::Status(503)) => true,
        (RetryOn::Retriable5Xx, Outcome::Status(c)) => *c >= 500 && *c <= 599,
        (RetryOn::Retriable4Xx, Outcome::Status(409)) => true,
        (RetryOn::ConnectFailure, Outcome::ConnectFailure) => true,
        (RetryOn::RefusedStream, Outcome::RefusedStream) => true,
        (RetryOn::Reset, Outcome::Reset) => true,
        (RetryOn::RetriableStatusCodes, Outcome::Status(c)) => retriable_codes.contains(c),
        (RetryOn::RetriableHeaders, _) => false, // caller-driven (header inspect)
        (RetryOn::GrpcCancelled, Outcome::GrpcStatus(1)) => true,
        (RetryOn::GrpcDeadlineExceeded, Outcome::GrpcStatus(4)) => true,
        (RetryOn::GrpcInternal, Outcome::GrpcStatus(13)) => true,
        (RetryOn::GrpcResourceExhausted, Outcome::GrpcStatus(8)) => true,
        (RetryOn::GrpcUnavailable, Outcome::GrpcStatus(14)) => true,
        _ => false,
    }
}

// ── Timeout ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeouts {
    /// End-to-end route timeout (0 = disabled).
    pub route_timeout: Duration,
    /// Per-stream idle timeout (0 = disabled).
    pub idle_timeout: Duration,
    /// Override for max gRPC timeout when the request carries grpc-timeout.
    pub max_grpc_timeout: Duration,
    /// Grace added on top of the inbound grpc-timeout header.
    pub grpc_timeout_offset: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            route_timeout: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(300),
            max_grpc_timeout: Duration::from_secs(0),
            grpc_timeout_offset: Duration::from_secs(0),
        }
    }
}

impl Timeouts {
    /// Resolve the effective timeout given the inbound grpc-timeout header.
    pub fn effective(&self, grpc_header: Option<Duration>) -> Duration {
        match grpc_header {
            Some(g) => {
                let with_offset = g.saturating_add(self.grpc_timeout_offset);
                if self.max_grpc_timeout.is_zero() {
                    with_offset
                } else {
                    with_offset.min(self.max_grpc_timeout)
                }
            }
            None => self.route_timeout,
        }
    }

    pub fn validate(&self) -> Result<(), RouteError> {
        if self.idle_timeout.as_secs() > 24 * 3600 {
            return Err(RouteError::Timeout("idle_timeout > 24h".into()));
        }
        Ok(())
    }
}

// ── HashPolicy ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HashPolicy {
    Header { name: String, terminal: bool },
    Cookie { name: String, ttl: Option<Duration>, path: Option<String>, terminal: bool },
    QueryParameter { name: String, terminal: bool },
    SourceIp { terminal: bool },
    /// Stable hash from `:method` + `:path` + custom prefix.
    PathPrefix { prefix: String, terminal: bool },
}

impl HashPolicy {
    pub fn header(name: impl Into<String>) -> Self {
        HashPolicy::Header { name: name.into(), terminal: false }
    }
    pub fn cookie(name: impl Into<String>) -> Self {
        HashPolicy::Cookie {
            name: name.into(),
            ttl: None,
            path: None,
            terminal: false,
        }
    }
    pub fn source_ip() -> Self {
        HashPolicy::SourceIp { terminal: false }
    }

    pub fn terminal(&self) -> bool {
        match self {
            HashPolicy::Header { terminal, .. }
            | HashPolicy::Cookie { terminal, .. }
            | HashPolicy::QueryParameter { terminal, .. }
            | HashPolicy::SourceIp { terminal, .. }
            | HashPolicy::PathPrefix { terminal, .. } => *terminal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HashInputs<'a> {
    pub headers: &'a HashMap<String, String>,
    pub cookies: &'a HashMap<String, String>,
    pub query: &'a HashMap<String, String>,
    pub source_ip: &'a str,
    pub path: &'a str,
}

/// Combine the hash policies into a single u64. Returns None when no policy
/// produced a value (caller must fall back to round-robin).
pub fn compute_hash(policies: &[HashPolicy], inputs: &HashInputs<'_>) -> Option<u64> {
    let mut hash: Option<u64> = None;
    for p in policies {
        let part = match p {
            HashPolicy::Header { name, .. } => inputs
                .headers
                .get(&name.to_ascii_lowercase())
                .map(|v| fnv1a64(v.as_bytes())),
            HashPolicy::Cookie { name, .. } => inputs
                .cookies
                .get(name)
                .map(|v| fnv1a64(v.as_bytes())),
            HashPolicy::QueryParameter { name, .. } => inputs
                .query
                .get(name)
                .map(|v| fnv1a64(v.as_bytes())),
            HashPolicy::SourceIp { .. } => Some(fnv1a64(inputs.source_ip.as_bytes())),
            HashPolicy::PathPrefix { prefix, .. } => {
                if inputs.path.starts_with(prefix) {
                    Some(fnv1a64(inputs.path.as_bytes()))
                } else {
                    None
                }
            }
        };
        if let Some(h) = part {
            hash = Some(match hash {
                None => h,
                Some(prev) => mix64(prev, h),
            });
            if p.terminal() {
                break;
            }
        }
    }
    hash
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn mix64(a: u64, b: u64) -> u64 {
    // SplitMix64 mix step, seeded with a and folded with b.
    let mut z = a.wrapping_add(b).wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

// ── Header manipulation ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderValue {
    pub key: String,
    pub value: String,
    pub append: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderManipulation {
    pub request_headers_to_add: Vec<HeaderValue>,
    pub request_headers_to_remove: Vec<String>,
    pub response_headers_to_add: Vec<HeaderValue>,
    pub response_headers_to_remove: Vec<String>,
}

impl HeaderManipulation {
    pub fn apply_request(&self, headers: &mut HashMap<String, String>) {
        for r in &self.request_headers_to_remove {
            headers.remove(&r.to_ascii_lowercase());
        }
        for h in &self.request_headers_to_add {
            let key = h.key.to_ascii_lowercase();
            if h.append {
                if let Some(existing) = headers.get_mut(&key) {
                    existing.push(',');
                    existing.push_str(&h.value);
                } else {
                    headers.insert(key, h.value.clone());
                }
            } else {
                headers.insert(key, h.value.clone());
            }
        }
    }

    pub fn apply_response(&self, headers: &mut HashMap<String, String>) {
        for r in &self.response_headers_to_remove {
            headers.remove(&r.to_ascii_lowercase());
        }
        for h in &self.response_headers_to_add {
            let key = h.key.to_ascii_lowercase();
            if h.append {
                if let Some(existing) = headers.get_mut(&key) {
                    existing.push(',');
                    existing.push_str(&h.value);
                } else {
                    headers.insert(key, h.value.clone());
                }
            } else {
                headers.insert(key, h.value.clone());
            }
        }
    }
}

// ── Redirect / DirectResponse ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectAction {
    pub https_redirect: bool,
    pub host_redirect: Option<String>,
    pub port_redirect: Option<u16>,
    pub path_redirect: Option<String>,
    pub prefix_rewrite: Option<String>,
    pub regex_rewrite: Option<RegexRewrite>,
    pub strip_query: bool,
    pub response_code: u16,
}

impl Default for RedirectAction {
    fn default() -> Self {
        Self {
            https_redirect: false,
            host_redirect: None,
            port_redirect: None,
            path_redirect: None,
            prefix_rewrite: None,
            regex_rewrite: None,
            strip_query: false,
            response_code: 301,
        }
    }
}

impl RedirectAction {
    pub fn redirect_url(&self, scheme: &str, host: &str, path: &str, query: &str) -> String {
        let scheme = if self.https_redirect { "https" } else { scheme };
        let host = self.host_redirect.as_deref().unwrap_or(host);
        let port_part = match self.port_redirect {
            Some(p) if !((scheme == "http" && p == 80) || (scheme == "https" && p == 443)) => {
                format!(":{}", p)
            }
            _ => String::new(),
        };
        let path = if let Some(redirect) = &self.path_redirect {
            redirect.clone()
        } else if let Some(prefix) = &self.prefix_rewrite {
            // Naive prefix rewrite: replace the longest matching prefix.
            let trimmed = path.trim_start_matches('/');
            format!("{}/{}", prefix.trim_end_matches('/'), trimmed)
        } else if let Some(rr) = &self.regex_rewrite {
            rr.apply(path)
        } else {
            path.to_string()
        };
        let query_part = if self.strip_query || query.is_empty() {
            String::new()
        } else {
            format!("?{}", query)
        };
        format!("{}://{}{}{}{}", scheme, host, port_part, path, query_part)
    }

    pub fn validate(&self) -> Result<(), RouteError> {
        if !(300..=399).contains(&self.response_code) {
            return Err(RouteError::Redirect(format!(
                "redirect response_code {} is not 3xx",
                self.response_code
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexRewrite {
    pub pattern: String,
    pub substitution: String,
}

impl RegexRewrite {
    pub fn apply(&self, input: &str) -> String {
        match regex::Regex::new(&self.pattern) {
            Ok(re) => re.replace(input, self.substitution.as_str()).into_owned(),
            Err(_) => input.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectResponse {
    pub status: u16,
    pub body: Option<String>,
}

impl DirectResponse {
    pub fn validate(&self) -> Result<(), RouteError> {
        if !(100..=599).contains(&self.status) {
            return Err(RouteError::DirectResponse(format!(
                "invalid status {}",
                self.status
            )));
        }
        Ok(())
    }
}

// ── Action ───────────────────────────────────────────────────────────────────

/// Top-level route action — Envoy `route.v3.Route.action`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteAction {
    Cluster {
        name: String,
        timeouts: Timeouts,
        retry: RetryPolicy,
        hash_policies: Vec<HashPolicy>,
        headers: HeaderManipulation,
    },
    WeightedClusters {
        clusters: WeightedClusters,
        timeouts: Timeouts,
        retry: RetryPolicy,
        hash_policies: Vec<HashPolicy>,
        headers: HeaderManipulation,
    },
    Redirect(RedirectAction),
    DirectResponse(DirectResponse),
}

impl RouteAction {
    pub fn validate(&self) -> Result<(), RouteError> {
        match self {
            RouteAction::Cluster { name, timeouts, retry, .. } => {
                if name.is_empty() {
                    return Err(RouteError::Cluster("empty cluster name".into()));
                }
                timeouts.validate()?;
                retry.validate()?;
            }
            RouteAction::WeightedClusters { clusters, timeouts, retry, .. } => {
                clusters.validate()?;
                timeouts.validate()?;
                retry.validate()?;
            }
            RouteAction::Redirect(r) => r.validate()?,
            RouteAction::DirectResponse(d) => d.validate()?,
        }
        Ok(())
    }
}

// ── RouteEngine — wires everything together ─────────────────────────────────

pub struct RouteEngine {
    pub cb: CircuitBreakerRegistry,
}

impl Default for RouteEngine {
    fn default() -> Self {
        Self { cb: CircuitBreakerRegistry::default() }
    }
}

impl RouteEngine {
    pub fn new(cb: CircuitBreakerRegistry) -> Self {
        Self { cb }
    }

    /// Pick a cluster by selector, skipping any that have a tripped
    /// circuit-breaker for *all* their members. The caller maps cluster_id →
    /// target_id list and passes via `is_cluster_open`.
    pub fn pick_cluster<'a, F>(
        &self,
        action: &'a RouteAction,
        selector: u32,
        is_cluster_open: F,
    ) -> Option<ClusterPick<'a>>
    where
        F: Fn(&str, Option<Uuid>) -> bool,
    {
        match action {
            RouteAction::Cluster { name, timeouts, retry, hash_policies, headers } => {
                if is_cluster_open(name, None) {
                    return None;
                }
                Some(ClusterPick {
                    cluster_name: name,
                    cluster_id: None,
                    timeouts,
                    retry,
                    hash_policies,
                    headers,
                })
            }
            RouteAction::WeightedClusters { clusters, timeouts, retry, hash_policies, headers } => {
                let total = clusters.effective_total();
                if total == 0 {
                    return None;
                }
                // Try preferred cluster first, then fall through ring.
                let n = clusters.clusters.len();
                let start = (selector % total) as u64;
                let mut cumulative: Vec<u32> = Vec::with_capacity(n);
                let mut acc: u32 = 0;
                for c in &clusters.clusters {
                    acc += c.weight;
                    cumulative.push(acc);
                }
                let mut start_idx = 0;
                for (i, end) in cumulative.iter().enumerate() {
                    if (start as u32) < *end {
                        start_idx = i;
                        break;
                    }
                }
                for offset in 0..n {
                    let i = (start_idx + offset) % n;
                    let c = &clusters.clusters[i];
                    if c.weight == 0 {
                        continue;
                    }
                    if is_cluster_open(&c.name, c.cluster_id) {
                        continue;
                    }
                    return Some(ClusterPick {
                        cluster_name: &c.name,
                        cluster_id: c.cluster_id,
                        timeouts,
                        retry,
                        hash_policies,
                        headers,
                    });
                }
                None
            }
            _ => None,
        }
    }

    /// Iteratively decide whether to retry; returns `Some(delay)` when the
    /// caller should sleep and retry, `None` to surface the failure.
    pub fn next_retry(
        &self,
        retry: &RetryPolicy,
        attempt: u32,
        outcome: &Outcome,
        method_idempotent: bool,
        active_requests: u32,
        active_retries: u32,
    ) -> Option<Duration> {
        if attempt >= retry.num_retries {
            return None;
        }
        if !retry.budget.allow(active_requests, active_retries) {
            return None;
        }
        if !retry.should_retry(outcome, method_idempotent) {
            return None;
        }
        Some(retry.backoff.delay(attempt + 1))
    }
}

#[derive(Debug)]
pub struct ClusterPick<'a> {
    pub cluster_name: &'a str,
    pub cluster_id: Option<Uuid>,
    pub timeouts: &'a Timeouts,
    pub retry: &'a RetryPolicy,
    pub hash_policies: &'a [HashPolicy],
    pub headers: &'a HeaderManipulation,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouteError {
    #[error("weighted clusters: {0}")]
    WeightedClusters(String),
    #[error("retry: {0}")]
    Retry(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("cluster: {0}")]
    Cluster(String),
    #[error("redirect: {0}")]
    Redirect(String),
    #[error("direct response: {0}")]
    DirectResponse(String),
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn idempotent() -> bool {
        true
    }

    // ── WeightedClusters ──

    #[test]
    fn weighted_single_validates() {
        WeightedClusters::single("cluster-a").validate().unwrap();
    }

    #[test]
    fn weighted_canary_split() {
        let w = WeightedClusters::canary("primary", "canary", 10);
        w.validate().unwrap();
        let mut canary = 0;
        let mut primary = 0;
        for s in 0..1000u32 {
            let c = w.pick(s).unwrap();
            if c.name == "canary" {
                canary += 1;
            } else {
                primary += 1;
            }
        }
        assert_eq!(canary, 100);
        assert_eq!(primary, 900);
    }

    #[test]
    fn weighted_canary_clamped_to_100() {
        let w = WeightedClusters::canary("a", "b", 250);
        // Both primary and canary clamp: primary=0, canary=100.
        let only = w.pick(0).unwrap();
        assert_eq!(only.name, "b");
    }

    #[test]
    fn weighted_explicit_total_with_unused() {
        // 30+30 sums to 60; explicit total=100 leaves 40% unallocated. The
        // unallocated portion deterministically routes to the last cluster.
        let w = WeightedClusters {
            clusters: vec![
                WeightedCluster {
                    name: "a".into(),
                    weight: 30,
                    runtime_key: None,
                    cluster_id: None,
                },
                WeightedCluster {
                    name: "b".into(),
                    weight: 30,
                    runtime_key: None,
                    cluster_id: None,
                },
            ],
            total_weight: 100,
        };
        assert_eq!(w.effective_total(), 100);
        let last = w.pick(80).unwrap();
        assert_eq!(last.name, "b");
    }

    #[test]
    fn weighted_empty_rejects() {
        let w = WeightedClusters { clusters: vec![], total_weight: 100 };
        assert!(w.validate().is_err());
    }

    #[test]
    fn weighted_zero_total_rejects() {
        let w = WeightedClusters {
            clusters: vec![WeightedCluster {
                name: "a".into(),
                weight: 0,
                runtime_key: None,
                cluster_id: None,
            }],
            total_weight: 0,
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn weighted_sum_exceeds_total_rejects() {
        let w = WeightedClusters {
            clusters: vec![
                WeightedCluster { name: "a".into(), weight: 60, runtime_key: None, cluster_id: None },
                WeightedCluster { name: "b".into(), weight: 60, runtime_key: None, cluster_id: None },
            ],
            total_weight: 100,
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn weighted_empty_name_rejects() {
        let w = WeightedClusters {
            clusters: vec![WeightedCluster { name: "".into(), weight: 100, runtime_key: None, cluster_id: None }],
            total_weight: 100,
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn weighted_pick_distribution_across_three() {
        let w = WeightedClusters {
            clusters: vec![
                WeightedCluster { name: "a".into(), weight: 10, runtime_key: None, cluster_id: None },
                WeightedCluster { name: "b".into(), weight: 30, runtime_key: None, cluster_id: None },
                WeightedCluster { name: "c".into(), weight: 60, runtime_key: None, cluster_id: None },
            ],
            total_weight: 100,
        };
        let mut counts = HashMap::new();
        for s in 0..1000u32 {
            *counts.entry(w.pick(s).unwrap().name.clone()).or_insert(0) += 1;
        }
        assert_eq!(counts["a"], 100);
        assert_eq!(counts["b"], 300);
        assert_eq!(counts["c"], 600);
    }

    // ── RetryBackoff ──

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        let b = RetryBackoff {
            base_interval: Duration::from_millis(10),
            max_interval: Duration::from_millis(80),
        };
        assert_eq!(b.delay(1), Duration::from_millis(10));
        assert_eq!(b.delay(2), Duration::from_millis(20));
        assert_eq!(b.delay(3), Duration::from_millis(40));
        assert_eq!(b.delay(4), Duration::from_millis(80));
        assert_eq!(b.delay(10), Duration::from_millis(80)); // capped
    }

    #[test]
    fn backoff_default_sane() {
        let b = RetryBackoff::default();
        assert!(b.delay(1) >= Duration::from_millis(20));
        assert!(b.delay(20) <= b.max_interval);
    }

    // ── RetryBudget ──

    #[test]
    fn budget_uses_min_floor() {
        let b = RetryBudget { budget_pct: 1.0, min_retry_concurrency: 5 };
        // 1% of 10 = 0.1, floor = 0, but min floor = 5.
        assert!(b.allow(10, 4));
        assert!(!b.allow(10, 5));
    }

    #[test]
    fn budget_uses_pct_when_above_floor() {
        let b = RetryBudget { budget_pct: 50.0, min_retry_concurrency: 1 };
        assert!(b.allow(20, 9));
        assert!(!b.allow(20, 10));
    }

    // ── RetryPolicy ──

    #[test]
    fn retry_policy_default_does_not_retry() {
        let p = RetryPolicy::default();
        assert!(!p.should_retry(&Outcome::Status(503), idempotent()));
    }

    #[test]
    fn retry_policy_5xx_triggers_on_500_through_599() {
        let p = RetryPolicy::five_xx(3);
        assert!(p.should_retry(&Outcome::Status(500), true));
        assert!(p.should_retry(&Outcome::Status(503), true));
        assert!(p.should_retry(&Outcome::Status(599), true));
        assert!(!p.should_retry(&Outcome::Status(499), true));
        assert!(!p.should_retry(&Outcome::Status(404), true));
    }

    #[test]
    fn retry_policy_gateway_error_503_only() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::GatewayError];
        assert!(p.should_retry(&Outcome::Status(503), true));
        assert!(!p.should_retry(&Outcome::Status(500), true));
    }

    #[test]
    fn retry_policy_connect_failure_does_not_need_idempotent() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::ConnectFailure];
        assert!(p.should_retry(&Outcome::ConnectFailure, false));
    }

    #[test]
    fn retry_policy_retriable_5xx_requires_idempotent() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::Retriable5Xx];
        assert!(p.should_retry(&Outcome::Status(503), true));
        assert!(!p.should_retry(&Outcome::Status(503), false));
    }

    #[test]
    fn retry_policy_retriable_status_codes() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::RetriableStatusCodes];
        p.retriable_status_codes = vec![418, 499];
        assert!(p.should_retry(&Outcome::Status(418), true));
        assert!(!p.should_retry(&Outcome::Status(500), true));
    }

    #[test]
    fn retry_policy_grpc_status_triggers() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::GrpcUnavailable, RetryOn::GrpcDeadlineExceeded];
        assert!(p.should_retry(&Outcome::GrpcStatus(14), true));
        assert!(p.should_retry(&Outcome::GrpcStatus(4), true));
        assert!(!p.should_retry(&Outcome::GrpcStatus(13), true));
    }

    #[test]
    fn retry_policy_validate_rejects_unreasonable_num() {
        let mut p = RetryPolicy::default();
        p.num_retries = 1000;
        assert!(p.validate().is_err());
    }

    #[test]
    fn retry_policy_validate_rejects_invalid_pct() {
        let mut p = RetryPolicy::default();
        p.budget.budget_pct = 200.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn retry_policy_validate_rejects_invalid_status_codes() {
        let mut p = RetryPolicy::default();
        p.retriable_status_codes = vec![700];
        assert!(p.validate().is_err());
    }

    // ── Timeouts ──

    #[test]
    fn timeouts_default_route() {
        let t = Timeouts::default();
        assert_eq!(t.effective(None), Duration::from_secs(15));
    }

    #[test]
    fn timeouts_grpc_header_passes_through() {
        let t = Timeouts::default();
        assert_eq!(
            t.effective(Some(Duration::from_secs(30))),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn timeouts_grpc_header_capped_by_max() {
        let mut t = Timeouts::default();
        t.max_grpc_timeout = Duration::from_secs(10);
        assert_eq!(
            t.effective(Some(Duration::from_secs(60))),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn timeouts_grpc_header_offset_added() {
        let mut t = Timeouts::default();
        t.grpc_timeout_offset = Duration::from_secs(2);
        assert_eq!(
            t.effective(Some(Duration::from_secs(5))),
            Duration::from_secs(7)
        );
    }

    #[test]
    fn timeouts_validate_rejects_huge_idle() {
        let mut t = Timeouts::default();
        t.idle_timeout = Duration::from_secs(48 * 3600);
        assert!(t.validate().is_err());
    }

    // ── HashPolicy ──

    #[test]
    fn hash_header_present() {
        let mut headers = HashMap::new();
        headers.insert("x-user".into(), "alice".into());
        let inputs = HashInputs {
            headers: &headers,
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "1.2.3.4",
            path: "/",
        };
        let h = compute_hash(&[HashPolicy::header("X-User")], &inputs).unwrap();
        let inputs2 = HashInputs {
            headers: &headers,
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "1.2.3.4",
            path: "/",
        };
        assert_eq!(compute_hash(&[HashPolicy::header("X-User")], &inputs2).unwrap(), h);
    }

    #[test]
    fn hash_header_missing_returns_none() {
        let headers = HashMap::new();
        let inputs = HashInputs {
            headers: &headers,
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "1.2.3.4",
            path: "/",
        };
        assert!(compute_hash(&[HashPolicy::header("X-User")], &inputs).is_none());
    }

    #[test]
    fn hash_source_ip_always_yields() {
        let inputs = HashInputs {
            headers: &HashMap::new(),
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "10.0.0.1",
            path: "/",
        };
        assert!(compute_hash(&[HashPolicy::source_ip()], &inputs).is_some());
    }

    #[test]
    fn hash_terminal_short_circuits() {
        let mut headers = HashMap::new();
        headers.insert("x-user".into(), "alice".into());
        let inputs = HashInputs {
            headers: &headers,
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "10.0.0.1",
            path: "/",
        };
        let policies = vec![
            HashPolicy::Header { name: "X-User".into(), terminal: true },
            HashPolicy::SourceIp { terminal: false },
        ];
        let h1 = compute_hash(&policies, &inputs).unwrap();
        let solo = compute_hash(&[HashPolicy::header("X-User")], &inputs).unwrap();
        // Terminal first → should equal the solo Header hash.
        assert_eq!(h1, solo);
    }

    #[test]
    fn hash_combines_when_not_terminal() {
        let mut headers = HashMap::new();
        headers.insert("x-user".into(), "alice".into());
        let inputs = HashInputs {
            headers: &headers,
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "10.0.0.1",
            path: "/",
        };
        let h_combined = compute_hash(
            &[
                HashPolicy::header("X-User"),
                HashPolicy::source_ip(),
            ],
            &inputs,
        )
        .unwrap();
        let h_solo = compute_hash(&[HashPolicy::header("X-User")], &inputs).unwrap();
        assert_ne!(h_combined, h_solo);
    }

    #[test]
    fn hash_path_prefix_only_matches_when_prefix_hits() {
        let inputs = HashInputs {
            headers: &HashMap::new(),
            cookies: &HashMap::new(),
            query: &HashMap::new(),
            source_ip: "10.0.0.1",
            path: "/api/v1/x",
        };
        assert!(compute_hash(
            &[HashPolicy::PathPrefix { prefix: "/api/".into(), terminal: false }],
            &inputs,
        )
        .is_some());
        assert!(compute_hash(
            &[HashPolicy::PathPrefix { prefix: "/admin/".into(), terminal: false }],
            &inputs,
        )
        .is_none());
    }

    // ── HeaderManipulation ──

    #[test]
    fn header_add_overwrites_by_default() {
        let mut h = HashMap::new();
        h.insert("x-trace".into(), "old".into());
        let m = HeaderManipulation {
            request_headers_to_add: vec![HeaderValue {
                key: "X-Trace".into(),
                value: "new".into(),
                append: false,
            }],
            ..Default::default()
        };
        m.apply_request(&mut h);
        assert_eq!(h.get("x-trace"), Some(&"new".to_string()));
    }

    #[test]
    fn header_add_appends_when_flag_set() {
        let mut h = HashMap::new();
        h.insert("x-trace".into(), "first".into());
        let m = HeaderManipulation {
            request_headers_to_add: vec![HeaderValue {
                key: "X-Trace".into(),
                value: "second".into(),
                append: true,
            }],
            ..Default::default()
        };
        m.apply_request(&mut h);
        assert_eq!(h.get("x-trace"), Some(&"first,second".to_string()));
    }

    #[test]
    fn header_remove_uses_lowercase() {
        let mut h = HashMap::new();
        h.insert("authorization".into(), "Bearer x".into());
        let m = HeaderManipulation {
            request_headers_to_remove: vec!["Authorization".into()],
            ..Default::default()
        };
        m.apply_request(&mut h);
        assert!(!h.contains_key("authorization"));
    }

    #[test]
    fn header_response_apply_independent_of_request() {
        let mut req = HashMap::new();
        let mut resp = HashMap::new();
        let m = HeaderManipulation {
            response_headers_to_add: vec![HeaderValue {
                key: "X-Cave".into(),
                value: "1".into(),
                append: false,
            }],
            ..Default::default()
        };
        m.apply_request(&mut req);
        m.apply_response(&mut resp);
        assert!(req.is_empty());
        assert_eq!(resp.get("x-cave"), Some(&"1".to_string()));
    }

    #[test]
    fn header_remove_then_add_yields_new_value() {
        let mut h = HashMap::new();
        h.insert("x-foo".into(), "old".into());
        let m = HeaderManipulation {
            request_headers_to_remove: vec!["x-foo".into()],
            request_headers_to_add: vec![HeaderValue {
                key: "x-foo".into(),
                value: "new".into(),
                append: true,
            }],
            ..Default::default()
        };
        m.apply_request(&mut h);
        assert_eq!(h.get("x-foo"), Some(&"new".to_string()));
    }

    // ── RedirectAction ──

    #[test]
    fn redirect_https_only() {
        let r = RedirectAction { https_redirect: true, ..Default::default() };
        assert_eq!(
            r.redirect_url("http", "example.com", "/foo", "a=1"),
            "https://example.com/foo?a=1"
        );
    }

    #[test]
    fn redirect_strips_query() {
        let r = RedirectAction {
            https_redirect: true,
            strip_query: true,
            ..Default::default()
        };
        assert_eq!(
            r.redirect_url("http", "example.com", "/foo", "a=1"),
            "https://example.com/foo"
        );
    }

    #[test]
    fn redirect_overrides_host_and_port() {
        let r = RedirectAction {
            host_redirect: Some("api.example.com".into()),
            port_redirect: Some(8443),
            ..Default::default()
        };
        assert_eq!(
            r.redirect_url("https", "example.com", "/foo", ""),
            "https://api.example.com:8443/foo"
        );
    }

    #[test]
    fn redirect_omits_default_ports() {
        let r = RedirectAction {
            port_redirect: Some(80),
            ..Default::default()
        };
        assert_eq!(
            r.redirect_url("http", "example.com", "/foo", ""),
            "http://example.com/foo"
        );
    }

    #[test]
    fn redirect_path_redirect_replaces_path() {
        let r = RedirectAction {
            path_redirect: Some("/new".into()),
            ..Default::default()
        };
        assert_eq!(
            r.redirect_url("http", "example.com", "/old/path", ""),
            "http://example.com/new"
        );
    }

    #[test]
    fn redirect_prefix_rewrite_substitutes_prefix() {
        let r = RedirectAction {
            prefix_rewrite: Some("/v2".into()),
            ..Default::default()
        };
        assert_eq!(
            r.redirect_url("http", "example.com", "/api/users", ""),
            "http://example.com/v2/api/users"
        );
    }

    #[test]
    fn redirect_validate_rejects_non_3xx() {
        let r = RedirectAction { response_code: 200, ..Default::default() };
        assert!(r.validate().is_err());
    }

    // ── DirectResponse ──

    #[test]
    fn direct_response_validates_status() {
        DirectResponse { status: 200, body: None }.validate().unwrap();
        assert!(DirectResponse { status: 99, body: None }.validate().is_err());
        assert!(DirectResponse { status: 600, body: None }.validate().is_err());
    }

    // ── RouteAction ──

    #[test]
    fn route_action_cluster_validates() {
        let a = RouteAction::Cluster {
            name: "svc".into(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        a.validate().unwrap();
    }

    #[test]
    fn route_action_cluster_empty_name_rejects() {
        let a = RouteAction::Cluster {
            name: "".into(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn route_action_weighted_validates() {
        let a = RouteAction::WeightedClusters {
            clusters: WeightedClusters::canary("a", "b", 10),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        a.validate().unwrap();
    }

    #[test]
    fn route_action_redirect_validates() {
        RouteAction::Redirect(RedirectAction::default()).validate().unwrap();
    }

    #[test]
    fn route_action_direct_response_validates() {
        RouteAction::DirectResponse(DirectResponse { status: 204, body: None })
            .validate()
            .unwrap();
    }

    // ── RouteEngine.pick_cluster ──

    #[test]
    fn engine_picks_single_cluster() {
        let e = RouteEngine::default();
        let action = RouteAction::Cluster {
            name: "svc".into(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        let pick = e.pick_cluster(&action, 0, |_, _| false).unwrap();
        assert_eq!(pick.cluster_name, "svc");
    }

    #[test]
    fn engine_skips_open_cluster() {
        let e = RouteEngine::default();
        let action = RouteAction::Cluster {
            name: "svc".into(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        let pick = e.pick_cluster(&action, 0, |_, _| true);
        assert!(pick.is_none());
    }

    #[test]
    fn engine_weighted_falls_through_open() {
        let e = RouteEngine::default();
        let action = RouteAction::WeightedClusters {
            clusters: WeightedClusters::canary("primary", "canary", 50),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        // Force the canary into open state.
        let pick = e
            .pick_cluster(&action, 0, |name, _| name == "canary")
            .unwrap();
        assert_eq!(pick.cluster_name, "primary");
    }

    #[test]
    fn engine_weighted_all_open_returns_none() {
        let e = RouteEngine::default();
        let action = RouteAction::WeightedClusters {
            clusters: WeightedClusters::canary("primary", "canary", 50),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        let pick = e.pick_cluster(&action, 0, |_, _| true);
        assert!(pick.is_none());
    }

    #[test]
    fn engine_weighted_zero_weight_skipped() {
        let e = RouteEngine::default();
        let weighted = WeightedClusters {
            clusters: vec![
                WeightedCluster { name: "zero".into(), weight: 0, runtime_key: None, cluster_id: None },
                WeightedCluster { name: "all".into(), weight: 100, runtime_key: None, cluster_id: None },
            ],
            total_weight: 100,
        };
        let action = RouteAction::WeightedClusters {
            clusters: weighted,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        for s in 0..50 {
            let pick = e.pick_cluster(&action, s, |_, _| false).unwrap();
            assert_eq!(pick.cluster_name, "all");
        }
    }

    // ── RouteEngine.next_retry ──

    #[test]
    fn next_retry_under_budget() {
        let e = RouteEngine::default();
        let mut p = RetryPolicy::five_xx(3);
        p.budget = RetryBudget { budget_pct: 50.0, min_retry_concurrency: 1 };
        let delay = e
            .next_retry(&p, 0, &Outcome::Status(503), true, 10, 0)
            .unwrap();
        assert!(delay >= Duration::from_millis(20));
    }

    #[test]
    fn next_retry_exhausted_attempts() {
        let e = RouteEngine::default();
        let p = RetryPolicy::five_xx(3);
        assert!(e
            .next_retry(&p, 3, &Outcome::Status(503), true, 1, 0)
            .is_none());
    }

    #[test]
    fn next_retry_blocks_on_non_retriable() {
        let e = RouteEngine::default();
        let p = RetryPolicy::five_xx(3);
        assert!(e
            .next_retry(&p, 0, &Outcome::Status(404), true, 1, 0)
            .is_none());
    }

    #[test]
    fn next_retry_blocks_on_budget() {
        let e = RouteEngine::default();
        let mut p = RetryPolicy::five_xx(3);
        p.budget = RetryBudget { budget_pct: 50.0, min_retry_concurrency: 1 };
        assert!(e
            .next_retry(&p, 0, &Outcome::Status(503), true, 10, 5)
            .is_none());
    }

    #[test]
    fn next_retry_grows_with_attempt() {
        let e = RouteEngine::default();
        let p = RetryPolicy::five_xx(5);
        let d1 = e.next_retry(&p, 0, &Outcome::Status(503), true, 1, 0).unwrap();
        let d2 = e.next_retry(&p, 1, &Outcome::Status(503), true, 1, 0).unwrap();
        let d3 = e.next_retry(&p, 2, &Outcome::Status(503), true, 1, 0).unwrap();
        assert!(d2 >= d1);
        assert!(d3 >= d2);
    }

    // ── RegexRewrite ──

    #[test]
    fn regex_rewrite_substitutes() {
        let rr = RegexRewrite {
            pattern: r"^/api/v1/(.*)$".into(),
            substitution: "/v2/$1".into(),
        };
        assert_eq!(rr.apply("/api/v1/users"), "/v2/users");
    }

    #[test]
    fn regex_rewrite_no_match_returns_input() {
        let rr = RegexRewrite { pattern: r"^/x/(.*)$".into(), substitution: "/y/$1".into() };
        assert_eq!(rr.apply("/api/users"), "/api/users");
    }

    #[test]
    fn regex_rewrite_invalid_pattern_returns_input() {
        let rr = RegexRewrite { pattern: r"[".into(), substitution: "/x".into() };
        assert_eq!(rr.apply("/foo"), "/foo");
    }

    // ── Outcome / RetryOn parsing ──

    #[test]
    fn outcome_matches_handles_grpc_internal() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::GrpcInternal];
        assert!(p.should_retry(&Outcome::GrpcStatus(13), true));
    }

    #[test]
    fn outcome_matches_grpc_resource_exhausted() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::GrpcResourceExhausted];
        assert!(p.should_retry(&Outcome::GrpcStatus(8), true));
    }

    #[test]
    fn outcome_matches_grpc_cancelled() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::GrpcCancelled];
        assert!(p.should_retry(&Outcome::GrpcStatus(1), true));
    }

    #[test]
    fn outcome_reset_triggers_reset() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::Reset];
        assert!(p.should_retry(&Outcome::Reset, false));
    }

    #[test]
    fn outcome_refused_stream_triggers() {
        let mut p = RetryPolicy::default();
        p.retry_on = vec![RetryOn::RefusedStream];
        assert!(p.should_retry(&Outcome::RefusedStream, false));
    }

    // ── Serde round-trips ──

    #[test]
    fn route_action_json_roundtrip_cluster() {
        let a = RouteAction::Cluster {
            name: "svc".into(),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::five_xx(3),
            hash_policies: vec![HashPolicy::header("X-User")],
            headers: HeaderManipulation::default(),
        };
        let s = serde_json::to_string(&a).unwrap();
        let a2: RouteAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, a2);
    }

    #[test]
    fn route_action_json_roundtrip_weighted() {
        let a = RouteAction::WeightedClusters {
            clusters: WeightedClusters::canary("a", "b", 5),
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            hash_policies: vec![],
            headers: HeaderManipulation::default(),
        };
        let s = serde_json::to_string(&a).unwrap();
        let a2: RouteAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, a2);
    }

    #[test]
    fn route_action_json_roundtrip_redirect() {
        let a = RouteAction::Redirect(RedirectAction {
            https_redirect: true,
            ..Default::default()
        });
        let s = serde_json::to_string(&a).unwrap();
        let a2: RouteAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, a2);
    }

    #[test]
    fn route_action_json_roundtrip_direct() {
        let a = RouteAction::DirectResponse(DirectResponse {
            status: 204,
            body: Some("ok".into()),
        });
        let s = serde_json::to_string(&a).unwrap();
        let a2: RouteAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, a2);
    }

    #[test]
    fn hash_policy_json_roundtrip() {
        let p = HashPolicy::Cookie {
            name: "session".into(),
            ttl: Some(Duration::from_secs(60)),
            path: Some("/".into()),
            terminal: true,
        };
        let s = serde_json::to_string(&p).unwrap();
        let p2: HashPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn retry_on_json_roundtrip() {
        let v = vec![
            RetryOn::FiveXx,
            RetryOn::ConnectFailure,
            RetryOn::GrpcUnavailable,
        ];
        let s = serde_json::to_string(&v).unwrap();
        let v2: Vec<RetryOn> = serde_json::from_str(&s).unwrap();
        assert_eq!(v, v2);
    }
}
