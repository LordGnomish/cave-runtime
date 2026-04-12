//! Data models for cave-mesh — services, routing, policies, mTLS, fault injection.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
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
    pub consecutive_errors: u32,
    pub interval_ms: u64,
    pub base_ejection_time_ms: u64,
    pub max_ejection_percent: u8,
    pub min_health_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub hosts: Vec<String>,
    pub addresses: Vec<String>,
    pub ports: Vec<ServicePort>,
    pub location: ServiceLocation,
    pub resolution: ServiceResolution,
    pub endpoints: Vec<ServiceEndpoint>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServiceLocation {
    MeshExternal,
    MeshInternal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServiceResolution {
    None,
    Static,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEndpoint {
    pub address: String,
    pub ports: HashMap<String, u16>,
    pub labels: HashMap<String, String>,
    pub weight: u32,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
