<<<<<<< HEAD
//! Istio-equivalent resource types for the CAVE service mesh.
//!
//! Covers VirtualService, DestinationRule, Gateway, ServiceEntry,
//! PeerAuthentication, RequestAuthentication, and AuthorizationPolicy.
=======
//! Data models for cave-mesh — services, routing, policies, mTLS, fault injection.
>>>>>>> claude/peaceful-lederberg

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
<<<<<<< HEAD

// ─────────────────────────────────────────────────────────────
// Service Registry
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMeta {
    pub name: String,
    pub namespace: String,
    pub labels: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
=======
use uuid::Uuid;

// ─── Service Registry ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub labels: HashMap<String, String>,
    pub ports: Vec<ServicePort>,
    pub protocol: Protocol,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub port: u16,
    pub target_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Protocol {
    Http,
    Http2,
    Grpc,
    Tcp,
    Tls,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInstance {
    pub id: Uuid,
    pub service_id: Uuid,
    pub address: String,
    pub port: u16,
    pub weight: u32,
    pub health: HealthStatus,
    pub labels: HashMap<String, String>,
    pub version: Option<String>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
>>>>>>> claude/peaceful-lederberg
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
<<<<<<< HEAD
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    /// host:port or bare IP
    pub address: String,
    pub port: u16,
    pub health: HealthStatus,
    /// Relative routing weight (default 100)
    pub weight: u32,
    pub labels: HashMap<String, String>,
    pub last_checked: DateTime<Utc>,
}

impl Endpoint {
    pub fn new(address: impl Into<String>, port: u16) -> Self {
        Self {
            address: address.into(),
            port,
            health: HealthStatus::Unknown,
            weight: 100,
            labels: HashMap::new(),
            last_checked: Utc::now(),
        }
    }

    pub fn healthy(mut self) -> Self {
        self.health = HealthStatus::Healthy;
        self
    }
}

// ─────────────────────────────────────────────────────────────
// VirtualService
// ─────────────────────────────────────────────────────────────

/// Maps to Istio VirtualService: host-based HTTP routing rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualService {
    pub name: String,
    pub namespace: String,
    /// Hosts this VS applies to (e.g. "reviews.prod.svc.cluster.local")
    pub hosts: Vec<String>,
    /// Gateway names this VS is attached to (empty = mesh-internal)
    pub gateways: Vec<String>,
    pub http: Vec<HttpRoute>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl VirtualService {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>, hosts: Vec<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            namespace: namespace.into(),
            hosts,
            gateways: vec![],
            http: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRoute {
    pub name: Option<String>,
    /// All match rules must pass (AND logic).  Empty = match everything.
    pub match_rules: Vec<HttpMatchRequest>,
    /// Weighted destinations
    pub route: Vec<HttpRouteDestination>,
    pub fault: Option<HttpFaultInjection>,
    pub retries: Option<HttpRetry>,
    /// Route-level timeout in milliseconds
    pub timeout_ms: Option<u64>,
    pub mirror: Option<Destination>,
    pub headers: Option<HeaderOperations>,
}

/// A single match predicate for an HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpMatchRequest {
    pub name: Option<String>,
    pub uri: Option<StringMatch>,
    pub headers: HashMap<String, StringMatch>,
    pub authority: Option<StringMatch>,
    pub method: Option<StringMatch>,
    pub query_params: HashMap<String, StringMatch>,
    /// Source labels the request pod must carry
    pub source_labels: HashMap<String, String>,
}

/// Istio StringMatch: exact, prefix, or regex.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StringMatch {
    Exact(String),
    Prefix(String),
    Regex(String),
}

impl StringMatch {
    pub fn matches(&self, value: &str) -> bool {
        match self {
            StringMatch::Exact(s) => s == value,
            StringMatch::Prefix(s) => value.starts_with(s.as_str()),
            StringMatch::Regex(pattern) => {
                regex::Regex::new(pattern)
                    .map(|re| re.is_match(value))
                    .unwrap_or(false)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRouteDestination {
    pub destination: Destination,
    /// 0-100 weight; all destinations in a route must sum to 100
    pub weight: Option<u32>,
    pub headers: Option<HeaderOperations>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Destination {
    pub host: String,
    pub subset: Option<String>,
    pub port: Option<PortSelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSelector {
    pub number: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderOperations {
    pub request: Option<HeaderManipulation>,
    pub response: Option<HeaderManipulation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderManipulation {
    pub set: HashMap<String, String>,
    pub add: HashMap<String, String>,
    pub remove: Vec<String>,
}

// ─── Fault Injection ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFaultInjection {
    pub delay: Option<FixedDelay>,
    pub abort: Option<HttpAbort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedDelay {
    /// 0.0-100.0 — percentage of requests to delay
    pub percent: f64,
    pub fixed_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpAbort {
    /// 0.0-100.0 — percentage of requests to abort
    pub percent: f64,
    /// HTTP status code to return on abort
    pub http_status: u16,
}

// ─── Retry ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRetry {
    pub attempts: u32,
    pub per_try_timeout_ms: Option<u64>,
    /// Comma-separated retry conditions: "5xx", "gateway-error", "retriable-4xx"
    pub retry_on: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// DestinationRule
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationRule {
    pub name: String,
    pub namespace: String,
    /// Host this rule applies to
    pub host: String,
    pub traffic_policy: Option<TrafficPolicy>,
    pub subsets: Vec<SubsetDefinition>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DestinationRule {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>, host: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            namespace: namespace.into(),
            host: host.into(),
            traffic_policy: None,
            subsets: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficPolicy {
    pub load_balancer: Option<LoadBalancerSettings>,
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LoadBalancerMode {
    RoundRobin,
    LeastConn,
    Random,
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerSettings {
    pub mode: LoadBalancerMode,
    pub consistent_hash_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPoolSettings {
    pub tcp: Option<TcpSettings>,
    pub http: Option<HttpPoolSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpSettings {
    pub max_connections: u32,
    pub connect_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpPoolSettings {
    pub max_requests_per_connection: u32,
    pub max_retries: u32,
    pub max_pending_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierDetection {
=======
    Draining,
}

// ─── Traffic Policy ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficPolicy {
    pub id: Uuid,
    pub name: String,
    pub service_id: Uuid,
    pub retry_policy: Option<RetryPolicy>,
    pub timeout: Option<TimeoutPolicy>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub rate_limit: Option<RateLimitPolicy>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Number of retry attempts
    pub attempts: u32,
    /// Per-attempt timeout in milliseconds
    pub per_try_timeout_ms: u64,
    /// Conditions that trigger a retry, e.g. ["5xx", "connect-failure", "reset"]
    pub retry_on: Vec<String>,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutPolicy {
    pub request_timeout_ms: u64,
    pub idle_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
>>>>>>> claude/peaceful-lederberg
    pub consecutive_errors: u32,
    pub interval_ms: u64,
    pub base_ejection_time_ms: u64,
    pub max_ejection_percent: u8,
    pub min_health_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
<<<<<<< HEAD
pub struct SubsetDefinition {
    pub name: String,
    /// Pod labels that select this subset
    pub labels: HashMap<String, String>,
    pub traffic_policy: Option<TrafficPolicy>,
}

// ─────────────────────────────────────────────────────────────
// Gateway
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gateway {
    pub name: String,
    pub namespace: String,
    /// Selects the gateway pod by label
    pub selector: HashMap<String, String>,
    pub servers: Vec<Server>,
=======
pub struct RateLimitPolicy {
    pub requests_per_unit: u64,
    pub unit: RateLimitUnit,
    pub burst: Option<u64>,
    /// Rate limit by specific header values (e.g. per API key)
    pub headers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitUnit {
    Second,
    Minute,
    Hour,
}

// ─── Virtual Service (routing rules + traffic splitting) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualService {
    pub id: Uuid,
    pub name: String,
    pub hosts: Vec<String>,
    pub http_routes: Vec<HttpRoute>,
    pub tls_routes: Vec<TlsRoute>,
    pub fault_injection: Option<FaultInjection>,
>>>>>>> claude/peaceful-lederberg
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
<<<<<<< HEAD
pub struct Server {
    pub port: ServerPort,
    /// Virtual hosts served through this server block
    pub hosts: Vec<String>,
    pub tls: Option<ServerTlsSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPort {
    pub number: u16,
    pub name: String,
    /// HTTP | HTTPS | GRPC | HTTP2 | MONGO | TCP | TLS
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTlsSettings {
    pub mode: TlsMode,
    pub server_certificate: Option<String>,
    pub private_key: Option<String>,
    pub ca_certificates: Option<String>,
    pub credential_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TlsMode {
    Passthrough,
    Simple,
    Mutual,
    AutoPassthrough,
    IstioMutual,
}

// ─────────────────────────────────────────────────────────────
// ServiceEntry
// ─────────────────────────────────────────────────────────────

/// Registers an external (off-mesh) service so it can be referenced in VS/DR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub namespace: String,
=======
pub struct HttpRoute {
    pub name: String,
    pub match_rules: Vec<RouteMatch>,
    pub destinations: Vec<WeightedDestination>,
    pub headers: Option<HeaderOperations>,
    pub timeout_ms: Option<u64>,
    pub mirror: Option<MirrorConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatch {
    pub uri: Option<StringMatch>,
    pub method: Option<String>,
    pub headers: HashMap<String, StringMatch>,
    pub query_params: HashMap<String, StringMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StringMatch {
    Exact { value: String },
    Prefix { value: String },
    Regex { value: String },
}

impl StringMatch {
    pub fn matches(&self, input: &str) -> bool {
        match self {
            Self::Exact { value } => input == value,
            Self::Prefix { value } => input.starts_with(value.as_str()),
            // Simplified: substring match (avoids pulling in regex crate)
            Self::Regex { value } => input.contains(value.as_str()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightedDestination {
    pub host: String,
    pub subset: Option<String>,
    pub port: Option<u16>,
    /// Weight relative to other destinations; total need not sum to 100
    pub weight: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderOperations {
    pub add: HashMap<String, String>,
    pub remove: Vec<String>,
    pub set: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorConfig {
    pub host: String,
    pub port: Option<u16>,
    /// Percentage of traffic to mirror (0.0–100.0)
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsRoute {
    pub match_rules: Vec<TlsMatch>,
    pub destinations: Vec<WeightedDestination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsMatch {
    pub sni_hosts: Vec<String>,
    pub destination_subnets: Vec<String>,
    pub port: Option<u16>,
}

// ─── Fault Injection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultInjection {
    pub delay: Option<FaultDelay>,
    pub abort: Option<FaultAbort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultDelay {
    /// Percentage of requests to delay (0.0–100.0)
    pub percentage: f64,
    pub fixed_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultAbort {
    /// Percentage of requests to abort (0.0–100.0)
    pub percentage: f64,
    pub http_status: u16,
    pub grpc_status: Option<String>,
}

// ─── Destination Rule ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationRule {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub traffic_policy: Option<TrafficPolicySpec>,
    pub subsets: Vec<Subset>,
    pub mtls: Option<MtlsConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficPolicySpec {
    pub load_balancer: LoadBalancerAlgorithm,
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalancerAlgorithm {
    RoundRobin,
    LeastConn,
    Random,
    IpHash,
    ConsistentHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPoolSettings {
    pub max_connections: u32,
    pub connect_timeout_ms: u64,
    pub max_requests_per_connection: Option<u32>,
    pub max_pending_requests: u32,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierDetection {
    pub consecutive_5xx: u32,
    pub interval_ms: u64,
    pub base_ejection_time_ms: u64,
    pub max_ejection_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subset {
    pub name: String,
    pub labels: HashMap<String, String>,
    pub traffic_policy: Option<TrafficPolicySpec>,
}

// ─── mTLS Config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtlsConfig {
    pub mode: MtlsMode,
    pub client_certificate: Option<String>,
    pub private_key: Option<String>,
    pub ca_certificates: Option<String>,
    pub subject_alt_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MtlsMode {
    Disable,
    Permissive,
    Strict,
}

// ─── Service Entry (external services) ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub id: Uuid,
    pub name: String,
>>>>>>> claude/peaceful-lederberg
    pub hosts: Vec<String>,
    pub addresses: Vec<String>,
    pub ports: Vec<ServicePort>,
    pub location: ServiceLocation,
    pub resolution: ServiceResolution,
<<<<<<< HEAD
    pub endpoints: Vec<WorkloadEntry>,
    pub export_to: Vec<String>,
=======
    pub endpoints: Vec<ServiceEndpoint>,
>>>>>>> claude/peaceful-lederberg
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

<<<<<<< HEAD
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub number: u16,
    pub name: String,
    pub protocol: String,
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
=======
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
>>>>>>> claude/peaceful-lederberg
pub enum ServiceLocation {
    MeshExternal,
    MeshInternal,
}

<<<<<<< HEAD
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
=======
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
>>>>>>> claude/peaceful-lederberg
pub enum ServiceResolution {
    None,
    Static,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
<<<<<<< HEAD
pub struct WorkloadEntry {
=======
pub struct ServiceEndpoint {
>>>>>>> claude/peaceful-lederberg
    pub address: String,
    pub ports: HashMap<String, u16>,
    pub labels: HashMap<String, String>,
    pub weight: u32,
<<<<<<< HEAD
    pub network: Option<String>,
    pub locality: Option<String>,
}

// ─────────────────────────────────────────────────────────────
// PeerAuthentication (mTLS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAuthentication {
    pub name: String,
    pub namespace: String,
    /// Workload selector (None = namespace-wide policy)
    pub selector: Option<HashMap<String, String>>,
    pub mtls: MtlsConfig,
    /// Per-port overrides
    pub port_level_mtls: HashMap<u16, MtlsConfig>,
=======
}

// ─── Sidecar Config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarConfig {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub workload_selector: HashMap<String, String>,
    pub ingress: Vec<IngressListener>,
    pub egress: Vec<EgressListener>,
    pub outbound_traffic_policy: OutboundTrafficPolicy,
>>>>>>> claude/peaceful-lederberg
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
<<<<<<< HEAD
pub struct MtlsConfig {
    pub mode: MtlsMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MtlsMode {
    /// Only mTLS connections accepted
    Strict,
    /// Both plaintext and mTLS accepted
    Permissive,
    /// mTLS disabled
    Disable,
}

// ─────────────────────────────────────────────────────────────
// RequestAuthentication (JWT)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestAuthentication {
    pub name: String,
    pub namespace: String,
    pub selector: Option<HashMap<String, String>>,
    pub jwt_rules: Vec<JwtRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtRule {
    pub issuer: String,
    pub audiences: Vec<String>,
    pub jwks_uri: Option<String>,
    /// Inline JWKS JSON (alternative to jwks_uri)
    pub jwks: Option<String>,
    /// Header names to extract the JWT from (default: Authorization: Bearer)
    pub from_headers: Vec<JwtHeader>,
    /// Query parameter names to extract the JWT from
    pub from_params: Vec<String>,
    /// If true, token validation errors are forwarded to the application
    pub forward_original_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtHeader {
    pub name: String,
    pub prefix: Option<String>,
}

// ─────────────────────────────────────────────────────────────
// AuthorizationPolicy
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationPolicy {
    pub name: String,
    pub namespace: String,
    pub selector: Option<HashMap<String, String>>,
    pub action: AuthzAction,
    pub rules: Vec<AuthzRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthzAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthzRule {
    /// Source constraints
    pub from: Vec<Source>,
    /// Operation constraints
    pub to: Vec<Operation>,
    /// Condition constraints (custom key/value from JWT claims or headers)
    pub when: Vec<Condition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Source {
    pub principals: Vec<String>,
    pub namespaces: Vec<String>,
    pub ip_blocks: Vec<String>,
    pub not_principals: Vec<String>,
    pub not_namespaces: Vec<String>,
    pub not_ip_blocks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Operation {
    pub hosts: Vec<String>,
    pub ports: Vec<String>,
    pub methods: Vec<String>,
    pub paths: Vec<String>,
    pub not_hosts: Vec<String>,
    pub not_ports: Vec<String>,
    pub not_methods: Vec<String>,
    pub not_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// Attribute key, e.g. "request.auth.claims[group]"
    pub key: String,
    pub values: Vec<String>,
    pub not_values: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// Rate Limit Policy
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub name: String,
    pub namespace: String,
    pub selector: Option<HashMap<String, String>>,
    pub rules: Vec<RateLimitRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    pub requests_per_unit: u64,
    pub unit: RateLimitUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RateLimitUnit {
    Second,
    Minute,
    Hour,
}

impl RateLimitUnit {
    /// Convert to requests-per-second rate.
    pub fn to_rps(&self, requests_per_unit: u64) -> f64 {
        match self {
            RateLimitUnit::Second => requests_per_unit as f64,
            RateLimitUnit::Minute => requests_per_unit as f64 / 60.0,
            RateLimitUnit::Hour => requests_per_unit as f64 / 3600.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Traffic Decision (output of TrafficManager)
// ─────────────────────────────────────────────────────────────

/// Resolved routing decision returned by the traffic manager.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub destination_host: String,
    pub destination_subset: Option<String>,
    pub destination_port: Option<u16>,
    pub weight: u32,
    pub fault: Option<FaultEffect>,
    pub retry: Option<HttpRetry>,
    pub timeout_ms: Option<u64>,
    pub request_headers_add: HashMap<String, String>,
    pub response_headers_add: HashMap<String, String>,
    /// W3C traceparent propagated / generated for this hop
    pub traceparent: Option<String>,
}

#[derive(Debug, Clone)]
pub enum FaultEffect {
    Delay(u64),     // milliseconds
    Abort(u16),     // HTTP status code
}

/// Minimal inbound request descriptor used by the traffic engine.
#[derive(Debug, Clone, Default)]
pub struct IncomingRequest {
    pub uri: String,
    pub method: String,
    pub authority: Option<String>,
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub source_labels: HashMap<String, String>,
    /// W3C Trace Context — carried in for propagation
    pub traceparent: Option<String>,
    pub tracestate: Option<String>,
}

/// Authz check input.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub source_principal: Option<String>,
    pub source_namespace: Option<String>,
    pub source_ip: Option<String>,
    pub method: String,
    pub path: String,
    pub host: String,
    pub port: Option<u16>,
    pub jwt_claims: Option<HashMap<String, serde_json::Value>>,
}

/// mTLS connection descriptor.
#[derive(Debug, Clone, Default)]
pub struct TlsContext {
    /// SPIFFE ID of the peer (e.g. "spiffe://cluster.local/ns/default/sa/reviews")
    pub peer_principal: Option<String>,
    pub is_mtls: bool,
=======
pub struct IngressListener {
    pub port: ServicePort,
    pub bind: Option<String>,
    pub capture_mode: CaptureMode,
    pub default_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressListener {
    pub port: Option<ServicePort>,
    pub bind: Option<String>,
    pub capture_mode: CaptureMode,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaptureMode {
    Default,
    Iptables,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OutboundTrafficPolicy {
    RegistryOnly,
    AllowAny,
>>>>>>> claude/peaceful-lederberg
}
