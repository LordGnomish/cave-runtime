//! Core gateway engine: rate limiting, auth, load balancing, circuit breaker.

use crate::models::*;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

// ── Rate limiting ─────────────────────────────────────────────────────────────

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    capacity: f64,
    refill_rate: f64,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self { tokens: capacity, last_refill: Instant::now(), capacity, refill_rate }
    }

    fn try_consume(&mut self) -> bool {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = Instant::now();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct SlidingWindow {
    timestamps: VecDeque<Instant>,
    max_requests: usize,
    window: Duration,
}

impl SlidingWindow {
    fn new(max_requests: usize, window: Duration) -> Self {
        Self { timestamps: VecDeque::new(), max_requests, window }
    }

    fn try_consume(&mut self) -> bool {
        let cutoff = Instant::now() - self.window;
        while self.timestamps.front().map_or(false, |&t| t < cutoff) {
            self.timestamps.pop_front();
        }
        if self.timestamps.len() < self.max_requests {
            self.timestamps.push_back(Instant::now());
            true
        } else {
            false
        }
    }
}

enum RateLimiterState {
    TokenBucket(TokenBucket),
    SlidingWindow(SlidingWindow),
}

impl RateLimiterState {
    fn try_consume(&mut self) -> bool {
        match self {
            Self::TokenBucket(b) => b.try_consume(),
            Self::SlidingWindow(w) => w.try_consume(),
        }
    }
}

// ── Circuit breaker ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure: Option<Instant>,
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure: None,
            config,
        }
    }

    fn is_open(&mut self) -> bool {
        if self.state == CircuitState::Open {
            if self.last_failure.map_or(false, |t| t.elapsed().as_secs() >= self.config.timeout_secs) {
                self.state = CircuitState::HalfOpen;
                self.failure_count = 0;
                return false;
            }
            return true;
        }
        false
    }

    fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.success_count = 0;
                    self.failure_count = 0;
                }
            }
            CircuitState::Closed => self.failure_count = 0,
            CircuitState::Open => {}
        }
    }

    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());
        if self.failure_count >= self.config.failure_threshold {
            self.state = CircuitState::Open;
        }
    }

    fn state_label(&self) -> &'static str {
        match self.state {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half-open",
        }
    }
}

// ── Load balancer ─────────────────────────────────────────────────────────────

struct LoadBalancerState {
    round_robin_idx: usize,
    connections: HashMap<Uuid, u32>,
}

impl LoadBalancerState {
    fn new() -> Self {
        Self { round_robin_idx: 0, connections: HashMap::new() }
    }

    fn pick_node<'a>(&mut self, nodes: &'a [UpstreamNode], algo: &LbAlgorithm) -> Option<&'a UpstreamNode> {
        let healthy: Vec<&'a UpstreamNode> = nodes.iter().filter(|n| n.healthy).collect();
        if healthy.is_empty() {
            return None;
        }
        match algo {
            LbAlgorithm::RoundRobin => {
                let idx = self.round_robin_idx % healthy.len();
                self.round_robin_idx = self.round_robin_idx.wrapping_add(1);
                Some(healthy[idx])
            }
            LbAlgorithm::LeastConnections => {
                healthy.iter()
                    .min_by_key(|n| self.connections.get(&n.id).copied().unwrap_or(0))
                    .copied()
            }
            LbAlgorithm::Weighted => {
                let total_weight: u32 = healthy.iter().map(|n| n.weight).sum();
                if total_weight == 0 {
                    return Some(healthy[0]);
                }
                let pick = (self.round_robin_idx as u32) % total_weight;
                self.round_robin_idx = self.round_robin_idx.wrapping_add(1);
                let mut acc = 0u32;
                for &node in &healthy {
                    acc += node.weight;
                    if pick < acc {
                        return Some(node);
                    }
                }
                Some(healthy[0])
            }
        }
    }
}

// ── Known bot signatures ──────────────────────────────────────────────────────

const KNOWN_BOTS: &[&str] = &[
    "Googlebot", "bingbot", "Slurp", "DuckDuckBot", "Baiduspider",
    "YandexBot", "Sogou", "Exabot", "facebot", "ia_archiver",
    "python-requests", "Go-http-client", "scrapy", "HTTrack",
    "libwww", "Semrushbot", "AhrefsBot", "MJ12bot",
];

// ── JWT claims ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct JwtClaims {
    sub: String,
    exp: usize,
}

// ── Gateway engine ────────────────────────────────────────────────────────────

pub struct GatewayEngine {
    pub routes: Vec<Route>,
    pub upstreams: Vec<UpstreamService>,
    pub metrics: GatewayMetrics,
    // route_id -> client_key -> limiter
    rate_limiters: HashMap<Uuid, HashMap<String, RateLimiterState>>,
    circuit_breakers: HashMap<Uuid, CircuitBreaker>,
    lb_states: HashMap<Uuid, LoadBalancerState>,
}

impl GatewayEngine {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            upstreams: Vec::new(),
            metrics: GatewayMetrics::default(),
            rate_limiters: HashMap::new(),
            circuit_breakers: HashMap::new(),
            lb_states: HashMap::new(),
        }
    }

    pub fn add_route(&mut self, req: CreateRouteRequest) -> Route {
        let route = Route {
            id: Uuid::new_v4(),
            name: req.name,
            path_prefix: req.path_prefix,
            methods: req.methods,
            upstream_id: req.upstream_id,
            plugins: req.plugins.unwrap_or_default(),
            rate_limit: req.rate_limit,
            auth: req.auth,
            strip_prefix: req.strip_prefix.unwrap_or(false),
            created_at: chrono::Utc::now(),
        };
        self.routes.push(route.clone());
        route
    }

    pub fn remove_route(&mut self, id: Uuid) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| r.id != id);
        self.routes.len() < before
    }

    pub fn add_upstream(&mut self, req: CreateUpstreamRequest) -> UpstreamService {
        let upstream = UpstreamService {
            id: Uuid::new_v4(),
            name: req.name,
            lb_algorithm: req.lb_algorithm,
            nodes: req.nodes.into_iter().map(|n| UpstreamNode {
                id: Uuid::new_v4(),
                address: n.address,
                weight: n.weight.unwrap_or(1),
                healthy: true,
            }).collect(),
            health_check: req.health_check,
            circuit_breaker: req.circuit_breaker,
            created_at: chrono::Utc::now(),
        };
        self.upstreams.push(upstream.clone());
        upstream
    }

    pub fn remove_upstream(&mut self, id: Uuid) -> bool {
        let before = self.upstreams.len();
        self.upstreams.retain(|u| u.id != id);
        self.upstreams.len() < before
    }

    /// Evaluate a simulated request: match route, check plugins, auth, rate limit, circuit breaker.
    pub fn evaluate_request(
        &mut self,
        path: &str,
        method: &str,
        client_ip: &str,
        auth_header: Option<&str>,
        user_agent: Option<&str>,
        body_size: usize,
    ) -> CheckResponse {
        self.metrics.total_requests += 1;

        // Match route by path prefix and method
        let route = self.routes.iter().find(|r| {
            path.starts_with(&r.path_prefix)
                && (r.methods.is_empty() || r.methods.iter().any(|m| m.eq_ignore_ascii_case(method)))
        }).cloned();

        let route = match route {
            Some(r) => r,
            None => {
                self.metrics.requests_blocked += 1;
                return CheckResponse {
                    allowed: false,
                    route_matched: None,
                    upstream_address: None,
                    blocked_reason: Some("no matching route".into()),
                };
            }
        };

        // Plugin: IP restriction
        for plugin in &route.plugins {
            if let PluginConfig::IpRestriction(cfg) = plugin {
                if !cfg.deny_list.is_empty() && cfg.deny_list.contains(&client_ip.to_string()) {
                    self.metrics.requests_blocked += 1;
                    return CheckResponse {
                        allowed: false,
                        route_matched: Some(route.name.clone()),
                        upstream_address: None,
                        blocked_reason: Some("ip denied".into()),
                    };
                }
                if !cfg.allow_list.is_empty() && !cfg.allow_list.contains(&client_ip.to_string()) {
                    self.metrics.requests_blocked += 1;
                    return CheckResponse {
                        allowed: false,
                        route_matched: Some(route.name.clone()),
                        upstream_address: None,
                        blocked_reason: Some("ip not in allow list".into()),
                    };
                }
            }

            // Plugin: Bot detection
            if let PluginConfig::BotDetection(cfg) = plugin {
                if let Some(ua) = user_agent {
                    if cfg.block_known_bots && KNOWN_BOTS.iter().any(|bot| ua.contains(bot)) {
                        self.metrics.requests_blocked += 1;
                        return CheckResponse {
                            allowed: false,
                            route_matched: Some(route.name.clone()),
                            upstream_address: None,
                            blocked_reason: Some("bot detected".into()),
                        };
                    }
                    if cfg.custom_patterns.iter().any(|p| ua.contains(p.as_str())) {
                        self.metrics.requests_blocked += 1;
                        return CheckResponse {
                            allowed: false,
                            route_matched: Some(route.name.clone()),
                            upstream_address: None,
                            blocked_reason: Some("bot detected (custom pattern)".into()),
                        };
                    }
                }
            }

            // Plugin: Request size limit
            if let PluginConfig::RequestSizeLimit(cfg) = plugin {
                if body_size > cfg.max_bytes {
                    self.metrics.requests_blocked += 1;
                    return CheckResponse {
                        allowed: false,
                        route_matched: Some(route.name.clone()),
                        upstream_address: None,
                        blocked_reason: Some(format!("body too large: {} > {}", body_size, cfg.max_bytes)),
                    };
                }
            }
        }

        // Auth check
        if let Some(ref auth_cfg) = route.auth {
            if !self.check_auth(auth_cfg, auth_header) {
                self.metrics.auth_failures += 1;
                self.metrics.requests_blocked += 1;
                return CheckResponse {
                    allowed: false,
                    route_matched: Some(route.name.clone()),
                    upstream_address: None,
                    blocked_reason: Some("authentication failed".into()),
                };
            }
        }

        // Rate limit check
        if let Some(ref rl_cfg) = route.rate_limit {
            let key = match rl_cfg.key_by {
                RateLimitKey::Ip => client_ip.to_string(),
                RateLimitKey::ApiKey => auth_header.unwrap_or(client_ip).to_string(),
                RateLimitKey::UserId => auth_header.unwrap_or(client_ip).to_string(),
            };
            if !self.check_rate_limit(route.id, &key, rl_cfg) {
                self.metrics.rate_limit_hits += 1;
                self.metrics.requests_blocked += 1;
                return CheckResponse {
                    allowed: false,
                    route_matched: Some(route.name.clone()),
                    upstream_address: None,
                    blocked_reason: Some("rate limit exceeded".into()),
                };
            }
        }

        // Circuit breaker check
        if self.is_circuit_open(route.upstream_id) {
            self.metrics.circuit_breaker_trips += 1;
            self.metrics.requests_blocked += 1;
            return CheckResponse {
                allowed: false,
                route_matched: Some(route.name.clone()),
                upstream_address: None,
                blocked_reason: Some("circuit breaker open".into()),
            };
        }

        // Pick upstream node
        let address = self.pick_upstream_node(route.upstream_id);
        self.metrics.requests_allowed += 1;
        CheckResponse {
            allowed: true,
            route_matched: Some(route.name),
            upstream_address: address,
            blocked_reason: None,
        }
    }

    fn check_auth(&self, cfg: &AuthConfig, auth_header: Option<&str>) -> bool {
        match cfg.method {
            AuthMethod::None | AuthMethod::OAuth2Passthrough => true,
            AuthMethod::ApiKey => {
                let key = auth_header.unwrap_or("").trim_start_matches("Bearer ").trim_start_matches("ApiKey ");
                cfg.api_keys.iter().any(|k| k == key)
            }
            AuthMethod::Jwt => {
                let token = auth_header.unwrap_or("").trim_start_matches("Bearer ");
                cfg.jwt_secret.as_deref().map_or(false, |secret| self.validate_jwt(token, secret))
            }
        }
    }

    fn validate_jwt(&self, token: &str, secret: &str) -> bool {
        use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
        let key = DecodingKey::from_secret(secret.as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        decode::<JwtClaims>(token, &key, &validation).is_ok()
    }

    fn check_rate_limit(&mut self, route_id: Uuid, client_key: &str, cfg: &RateLimitConfig) -> bool {
        let bucket_map = self.rate_limiters.entry(route_id).or_default();
        let limiter = bucket_map.entry(client_key.to_string()).or_insert_with(|| {
            match cfg.algorithm {
                RateLimitAlgorithm::TokenBucket => RateLimiterState::TokenBucket(
                    TokenBucket::new(cfg.burst as f64, cfg.requests_per_second),
                ),
                RateLimitAlgorithm::SlidingWindow => RateLimiterState::SlidingWindow(
                    SlidingWindow::new(cfg.burst as usize, Duration::from_secs(1)),
                ),
            }
        });
        limiter.try_consume()
    }

    fn is_circuit_open(&mut self, upstream_id: Uuid) -> bool {
        let config = self.upstreams.iter()
            .find(|u| u.id == upstream_id)
            .and_then(|u| u.circuit_breaker.clone());

        let cb = self.circuit_breakers.entry(upstream_id).or_insert_with(|| {
            CircuitBreaker::new(config.unwrap_or(CircuitBreakerConfig {
                failure_threshold: 5,
                success_threshold: 2,
                timeout_secs: 30,
            }))
        });
        cb.is_open()
    }

    pub fn record_upstream_result(&mut self, upstream_id: Uuid, success: bool) {
        let config = self.upstreams.iter()
            .find(|u| u.id == upstream_id)
            .and_then(|u| u.circuit_breaker.clone());

        let cb = self.circuit_breakers.entry(upstream_id).or_insert_with(|| {
            CircuitBreaker::new(config.unwrap_or(CircuitBreakerConfig {
                failure_threshold: 5,
                success_threshold: 2,
                timeout_secs: 30,
            }))
        });
        if success {
            cb.record_success();
        } else {
            cb.record_failure();
            self.metrics.upstream_errors += 1;
        }
    }

    fn pick_upstream_node(&mut self, upstream_id: Uuid) -> Option<String> {
        let upstream = self.upstreams.iter().find(|u| u.id == upstream_id)?.clone();
        let lb = self.lb_states.entry(upstream_id).or_insert_with(LoadBalancerState::new);
        let node = lb.pick_node(&upstream.nodes, &upstream.lb_algorithm)?;
        Some(node.address.clone())
    }

    pub fn set_node_health(&mut self, upstream_id: Uuid, node_id: Uuid, healthy: bool) {
        if let Some(upstream) = self.upstreams.iter_mut().find(|u| u.id == upstream_id) {
            if let Some(node) = upstream.nodes.iter_mut().find(|n| n.id == node_id) {
                node.healthy = healthy;
            }
        }
    }

    pub fn circuit_breaker_statuses(&self) -> Vec<CircuitBreakerStatus> {
        self.upstreams.iter().map(|u| {
            let state = self.circuit_breakers.get(&u.id)
                .map(|cb| cb.state_label())
                .unwrap_or("closed");
            let failure_count = self.circuit_breakers.get(&u.id)
                .map(|cb| cb.failure_count)
                .unwrap_or(0);
            CircuitBreakerStatus {
                upstream_id: u.id,
                upstream_name: u.name.clone(),
                state: state.to_string(),
                failure_count,
            }
        }).collect()
    }
}

impl Default for GatewayEngine {
    fn default() -> Self {
        Self::new()
    }
}
