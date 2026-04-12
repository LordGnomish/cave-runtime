//! Data models for the CAVE Gateway — Kong + Gravitee compatible types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────
//  Protocols
// ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Http,
    Https,
    Grpc,
    Grpcs,
    Tcp,
    Tls,
    Ws,
    Wss,
}

// ─────────────────────────────────────────────
//  Service (upstream backend definition)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: Uuid,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub path: Option<String>,
    /// HTTP path prefix rewrite
    pub retries: u32,
    pub connect_timeout: u64,
    pub write_timeout: u64,
    pub read_timeout: u64,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Service {
    pub fn base_url(&self) -> String {
        let proto = match self.protocol {
            Protocol::Https | Protocol::Wss | Protocol::Grpcs => "https",
            _ => "http",
        };
        let path = self.path.as_deref().unwrap_or("");
        format!("{proto}://{}:{}{path}", self.host, self.port)
    }
}

// ─────────────────────────────────────────────
//  Route
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub name: Option<String>,
    pub service_id: Uuid,
    pub protocols: Vec<Protocol>,
    /// HTTP methods: GET, POST, … (empty = all)
    pub methods: Vec<String>,
    /// Host patterns (empty = all)
    pub hosts: Vec<String>,
    /// Path patterns — prefix (`/api`) or regex (`~/api/v[0-9]+`)
    pub paths: Vec<String>,
    /// Header matchers: header name → list of acceptable values
    pub headers: HashMap<String, Vec<String>>,
    /// SNI names for TLS routes
    pub snis: Vec<String>,
    /// Strip the matched path prefix before forwarding
    pub strip_path: bool,
    /// Forward the original `Host` header unchanged
    pub preserve_host: bool,
    /// Higher = evaluated first when multiple routes match
    pub regex_priority: i32,
    pub path_handling: PathHandling,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PathHandling {
    #[default]
    V0,
    V1,
}

// ─────────────────────────────────────────────
//  Upstream (load-balanced pool)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub id: Uuid,
    pub name: String,
    pub algorithm: LoadBalancingAlgorithm,
    pub hash_on: HashOn,
    pub hash_fallback: HashFallback,
    /// Header name used when hash_on = Header
    pub hash_on_header: Option<String>,
    pub healthchecks: HealthCheckConfig,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalancingAlgorithm {
    #[default]
    RoundRobin,
    ConsistentHashing,
    LeastConnections,
    LatencyAware,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HashOn {
    #[default]
    None,
    Ip,
    Header,
    Cookie,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HashFallback {
    #[default]
    None,
    Ip,
    Header,
    Cookie,
}

// ─────────────────────────────────────────────
//  Health check config (attached to Upstream)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub active: ActiveHealthCheck,
    pub passive: PassiveHealthCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveHealthCheck {
    pub enabled: bool,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub http_path: String,
    pub https_verify_certificate: bool,
    pub healthy: HealthThreshold,
    pub unhealthy: HealthThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassiveHealthCheck {
    pub enabled: bool,
    pub healthy: HealthThreshold,
    pub unhealthy: HealthThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthThreshold {
    pub http_statuses: Vec<u16>,
    pub successes: u32,
    pub failures: u32,
    pub timeouts: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            active: ActiveHealthCheck {
                enabled: false,
                interval_secs: 5,
                timeout_secs: 1,
                http_path: "/".into(),
                https_verify_certificate: true,
                healthy: HealthThreshold {
                    http_statuses: vec![200, 302],
                    successes: 5,
                    failures: 0,
                    timeouts: 0,
                },
                unhealthy: HealthThreshold {
                    http_statuses: vec![429, 500, 503],
                    successes: 0,
                    failures: 5,
                    timeouts: 3,
                },
            },
            passive: PassiveHealthCheck {
                enabled: true,
                healthy: HealthThreshold {
                    http_statuses: (200u16..=308).collect(),
                    successes: 5,
                    failures: 0,
                    timeouts: 0,
                },
                unhealthy: HealthThreshold {
                    http_statuses: vec![429, 500, 502, 503, 504],
                    successes: 0,
                    failures: 5,
                    timeouts: 3,
                },
            },
        }
    }
}

// ─────────────────────────────────────────────
//  Target (individual upstream host:port)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: Uuid,
    pub upstream_id: Uuid,
    /// "host:port" string
    pub target: String,
    pub weight: u32,
    pub health: TargetHealth,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TargetHealth {
    #[default]
    Healthy,
    Unhealthy,
    Degraded,
}

// ─────────────────────────────────────────────
//  Consumer
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    pub id: Uuid,
    pub username: Option<String>,
    pub custom_id: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────
//  Credentials
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyAuthCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub key: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    /// Identifies the credential (used as JWT `iss` or `kid`)
    pub key: String,
    /// HMAC secret or RSA public key PEM
    pub secret: String,
    pub algorithm: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAuthCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub username: String,
    /// Stored as plain for simplicity; in production use bcrypt/argon2
    pub password_hash: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HmacAuthCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub username: String,
    pub secret: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Credential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uris: Vec<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────
//  Plugin
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub id: Uuid,
    pub name: String,
    pub service_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub consumer_id: Option<Uuid>,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub protocols: Vec<Protocol>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────
//  API Versioning / Lifecycle
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiVersion {
    pub id: Uuid,
    pub service_id: Uuid,
    pub version: String,
    pub status: ApiVersionStatus,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub sunset_at: Option<DateTime<Utc>>,
    pub changelog: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiVersionStatus {
    #[default]
    Active,
    Deprecated,
    Retired,
}

// ─────────────────────────────────────────────
//  Developer Portal (Gravitee-inspired)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalSubscription {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub service_id: Uuid,
    pub plan: SubscriptionPlan,
    pub status: SubscriptionStatus,
    /// Provisioned API key for this subscription
    pub api_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionPlan {
    #[default]
    Free,
    Basic,
    Pro,
    Enterprise,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionStatus {
    Pending,
    #[default]
    Active,
    Suspended,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDoc {
    pub id: Uuid,
    pub service_id: Uuid,
    pub title: String,
    pub content: String,
    pub format: DocFormat,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    #[default]
    Markdown,
    OpenApi,
    AsyncApi,
    Graphql,
}

// ─────────────────────────────────────────────
//  Monetization / Usage tracking
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub service_id: Uuid,
    pub route_id: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub request_count: u64,
    pub response_bytes: u64,
    pub latency_ms: u64,
    pub status_code: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub consumer_id: Uuid,
    pub service_id: Uuid,
    pub total_requests: u64,
    pub total_bytes: u64,
    pub avg_latency_ms: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

// ─────────────────────────────────────────────
//  Request / Response DTOs
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub protocol: Option<Protocol>,
    pub host: String,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub retries: Option<u32>,
    pub connect_timeout: Option<u64>,
    pub write_timeout: Option<u64>,
    pub read_timeout: Option<u64>,
    pub tags: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRouteRequest {
    pub name: Option<String>,
    pub service_id: Uuid,
    pub protocols: Option<Vec<Protocol>>,
    pub methods: Option<Vec<String>>,
    pub hosts: Option<Vec<String>>,
    pub paths: Option<Vec<String>>,
    pub headers: Option<HashMap<String, Vec<String>>>,
    pub snis: Option<Vec<String>>,
    pub strip_path: Option<bool>,
    pub preserve_host: Option<bool>,
    pub regex_priority: Option<i32>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUpstreamRequest {
    pub name: String,
    pub algorithm: Option<LoadBalancingAlgorithm>,
    pub hash_on: Option<HashOn>,
    pub hash_on_header: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTargetRequest {
    pub target: String,
    pub weight: Option<u32>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateConsumerRequest {
    pub username: Option<String>,
    pub custom_id: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePluginRequest {
    pub name: String,
    pub service_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub consumer_id: Option<Uuid>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateKeyAuthRequest {
    pub key: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJwtRequest {
    pub key: Option<String>,
    pub secret: Option<String>,
    pub algorithm: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBasicAuthRequest {
    pub username: String,
    pub password: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateHmacAuthRequest {
    pub username: String,
    pub secret: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse<T> {
    pub data: Vec<T>,
    pub total: usize,
    pub next: Option<String>,
}

impl<T> ListResponse<T> {
    pub fn new(data: Vec<T>) -> Self {
        let total = data.len();
        Self { data, total, next: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub consumer_id: Uuid,
    pub service_id: Uuid,
    pub plan: Option<SubscriptionPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVersionRequest {
    pub version: String,
    pub changelog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocRequest {
    pub title: String,
    pub content: String,
    pub format: Option<DocFormat>,
}
