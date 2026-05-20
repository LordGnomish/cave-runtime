// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Istio-equivalent resource types for the CAVE service mesh
//! (Ambient-only — sidecar/EnvoyFilter/WorkloadGroup CRDs are absent).
//!
//! Ambient-mode surface tracked against Istio 1.30.0:
//!   Traffic:     VirtualService, DestinationRule, Gateway, ServiceEntry
//!   Workload:    WorkloadEntry (carrier for ServiceEntry.endpoints only)
//!   Security:    PeerAuthentication, RequestAuthentication, AuthorizationPolicy
//!   Observability: Telemetry (metrics/logs/tracing per workload)
//!   Rate Limit:  RateLimitPolicy (CAVE extension)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub address: String,
    pub port: u16,
    pub health: HealthStatus,
    pub weight: u32,
    pub labels: HashMap<String, String>,
    pub last_checked: DateTime<Utc>,
    pub locality: Option<Locality>,
    pub network: Option<String>,
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
            locality: None,
            network: None,
        }
    }

    pub fn healthy(mut self) -> Self {
        self.health = HealthStatus::Healthy;
        self
    }
}

/// Locality descriptor (region/zone/subzone) matching Istio's Locality proto.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Locality {
    pub region: String,
    pub zone: Option<String>,
    pub sub_zone: Option<String>,
}

impl Locality {
    pub fn new(region: impl Into<String>) -> Self {
        Self { region: region.into(), zone: None, sub_zone: None }
    }

    pub fn with_zone(mut self, zone: impl Into<String>) -> Self {
        self.zone = Some(zone.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────
// VirtualService
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualService {
    pub name: String,
    pub namespace: String,
    pub hosts: Vec<String>,
    pub gateways: Vec<String>,
    pub http: Vec<HttpRoute>,
    pub tcp: Vec<TcpRoute>,
    pub tls: Vec<TlsRoute>,
    pub export_to: Vec<String>,
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
            tcp: vec![],
            tls: vec![],
            export_to: vec![".".to_string()],
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRoute {
    pub name: Option<String>,
    pub match_rules: Vec<HttpMatchRequest>,
    pub route: Vec<HttpRouteDestination>,
    pub redirect: Option<HttpRedirect>,
    pub direct_response: Option<HttpDirectResponse>,
    pub rewrite: Option<HttpRewrite>,
    pub fault: Option<HttpFaultInjection>,
    pub retries: Option<HttpRetry>,
    pub timeout_ms: Option<u64>,
    pub mirror: Option<Destination>,
    pub mirror_percentage: Option<f64>,
    pub headers: Option<HeaderOperations>,
    pub cors_policy: Option<CorsPolicy>,
}

/// HTTP redirect (307/301/302).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRedirect {
    pub uri: Option<String>,
    pub authority: Option<String>,
    pub port: Option<u16>,
    pub scheme: Option<String>,
    /// HTTP status code (301, 302, 303, 307, 308).  Default 301.
    pub redirect_code: u32,
}

impl Default for HttpRedirect {
    fn default() -> Self {
        Self { uri: None, authority: None, port: None, scheme: None, redirect_code: 301 }
    }
}

/// URI / authority rewrite before forwarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRewrite {
    pub uri: Option<String>,
    pub authority: Option<String>,
}

/// Return a fixed response without forwarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpDirectResponse {
    pub status: u32,
    pub body: Option<HttpBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBody {
    pub string: Option<String>,
    pub bytes: Option<String>, // base64-encoded
}

/// Match predicate for an HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpMatchRequest {
    pub name: Option<String>,
    pub uri: Option<StringMatch>,
    pub headers: HashMap<String, StringMatch>,
    pub authority: Option<StringMatch>,
    pub method: Option<StringMatch>,
    pub query_params: HashMap<String, StringMatch>,
    pub source_labels: HashMap<String, String>,
    pub gateways: Vec<String>,
    pub source_namespace: Option<String>,
    pub without_headers: HashMap<String, StringMatch>,
    pub port: Option<u32>,
    pub ignore_uri_case: bool,
}

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
            StringMatch::Regex(pattern) => regex::Regex::new(pattern)
                .map(|re| re.is_match(value))
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRouteDestination {
    pub destination: Destination,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsPolicy {
    pub allow_origins: Vec<StringMatch>,
    pub allow_methods: Vec<String>,
    pub allow_headers: Vec<String>,
    pub expose_headers: Vec<String>,
    pub max_age_seconds: Option<u64>,
    pub allow_credentials: bool,
}

// ─── Fault Injection ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFaultInjection {
    pub delay: Option<FixedDelay>,
    pub abort: Option<HttpAbort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedDelay {
    pub percent: f64,
    pub fixed_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpAbort {
    pub percent: f64,
    pub http_status: u16,
    pub grpc_status: Option<String>,
}

// ─── Retry ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRetry {
    pub attempts: u32,
    pub per_try_timeout_ms: Option<u64>,
    pub retry_on: Vec<String>,
    pub retry_remote_localities: bool,
}

// ─── TCP / TLS routes ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpRoute {
    pub match_rules: Vec<L4MatchAttributes>,
    pub route: Vec<RouteDestination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4MatchAttributes {
    pub destination_subnets: Vec<String>,
    pub port: Option<u32>,
    pub source_labels: HashMap<String, String>,
    pub gateways: Vec<String>,
    pub source_namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDestination {
    pub destination: Destination,
    pub weight: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsRoute {
    pub match_rules: Vec<TlsMatchAttributes>,
    pub route: Vec<RouteDestination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsMatchAttributes {
    pub sni_hosts: Vec<String>,
    pub destination_subnets: Vec<String>,
    pub port: Option<u32>,
    pub source_labels: HashMap<String, String>,
    pub gateways: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// DestinationRule
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationRule {
    pub name: String,
    pub namespace: String,
    pub host: String,
    pub traffic_policy: Option<TrafficPolicy>,
    pub subsets: Vec<SubsetDefinition>,
    pub export_to: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DestinationRule {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        host: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            namespace: namespace.into(),
            host: host.into(),
            traffic_policy: None,
            subsets: vec![],
            export_to: vec![".".to_string()],
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
    pub tls: Option<ClientTlsSettings>,
    pub port_level_settings: Vec<PortTrafficPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LoadBalancerMode {
    RoundRobin,
    LeastConn,
    Random,
    Passthrough,
    ConsistentHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerSettings {
    pub mode: LoadBalancerMode,
    pub consistent_hash: Option<ConsistentHashLb>,
    pub locality_lb_setting: Option<LocalityLbSetting>,
    pub warmup_duration_secs: Option<u64>,
}

/// Consistent-hash load balancing — multiple key types (Istio parity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistentHashLb {
    pub key_type: ConsistentHashKey,
    pub minimum_ring_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConsistentHashKey {
    /// Hash on an HTTP header value.
    HttpHeaderName(String),
    /// Hash on a cookie (name, path, TTL).
    HttpCookie { name: String, path: Option<String>, ttl_seconds: u64 },
    /// Hash on the source IP.
    UseSourceIp,
    /// Hash on a query parameter.
    HttpQueryParameterName(String),
}

/// Locality-aware load balancing distribution and failover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalityLbSetting {
    pub enabled: Option<bool>,
    pub distribute: Vec<LocalityWeightSetting>,
    pub failover: Vec<LocalityFailover>,
    pub failover_priority: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalityWeightSetting {
    /// Origin locality (e.g. "us-east1/*").
    pub from: String,
    /// Distribution map: locality → weight (0-100).
    pub to: HashMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalityFailover {
    pub from: String,
    pub to: String,
}

/// Per-port traffic policy (overrides top-level for a specific port).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortTrafficPolicy {
    pub port: PortSelector,
    pub load_balancer: Option<LoadBalancerSettings>,
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
    pub tls: Option<ClientTlsSettings>,
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
    pub tcp_keepalive: Option<TcpKeepalive>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpKeepalive {
    pub probes: u32,
    pub time_seconds: u32,
    pub interval_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpPoolSettings {
    pub max_requests_per_connection: u32,
    pub max_retries: u32,
    pub max_pending_requests: u32,
    pub idle_timeout_ms: Option<u64>,
    pub h2_upgrade_policy: H2UpgradePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum H2UpgradePolicy {
    Default,
    DoNotUpgrade,
    Upgrade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierDetection {
    pub consecutive_errors: u32,
    pub consecutive_gateway_errors: Option<u32>,
    pub consecutive_local_origin_errors: Option<u32>,
    pub interval_ms: u64,
    pub base_ejection_time_ms: u64,
    pub max_ejection_percent: u8,
    pub min_health_percent: u8,
    pub split_external_local_origin_errors: bool,
}

/// Client-side TLS settings for outbound connections (DestinationRule).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientTlsSettings {
    pub mode: ClientTlsMode,
    pub client_certificate: Option<String>,
    pub private_key: Option<String>,
    pub ca_certificates: Option<String>,
    pub credential_name: Option<String>,
    pub subject_alt_names: Vec<String>,
    pub sni: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClientTlsMode {
    Disable,
    Simple,
    Mutual,
    IstioMutual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsetDefinition {
    pub name: String,
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
    pub selector: HashMap<String, String>,
    pub servers: Vec<Server>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub port: ServerPort,
    pub hosts: Vec<String>,
    pub tls: Option<ServerTlsSettings>,
    pub default_endpoint: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPort {
    pub number: u16,
    pub name: String,
    pub protocol: String,
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTlsSettings {
    pub mode: TlsMode,
    pub server_certificate: Option<String>,
    pub private_key: Option<String>,
    pub ca_certificates: Option<String>,
    pub credential_name: Option<String>,
    pub subject_alt_names: Vec<String>,
    pub verify_certificate_spki: Vec<String>,
    pub verify_certificate_hash: Vec<String>,
    pub min_protocol_version: Option<TlsProtocol>,
    pub max_protocol_version: Option<TlsProtocol>,
    pub cipher_suites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TlsMode {
    Passthrough,
    Simple,
    Mutual,
    AutoPassthrough,
    IstioMutual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TlsProtocol {
    TlsAuto,
    Tlsv10,
    Tlsv11,
    Tlsv12,
    Tlsv13,
}

// ─────────────────────────────────────────────────────────────
// ServiceEntry
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub namespace: String,
    pub hosts: Vec<String>,
    pub addresses: Vec<String>,
    pub ports: Vec<ServicePort>,
    pub location: ServiceLocation,
    pub resolution: ServiceResolution,
    pub endpoints: Vec<WorkloadEntry>,
    pub export_to: Vec<String>,
    pub subject_alt_names: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub number: u16,
    pub name: String,
    pub protocol: String,
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServiceLocation {
    MeshExternal,
    MeshInternal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServiceResolution {
    None,
    Static,
    Dns,
    DnsRoundRobin,
}

// ─────────────────────────────────────────────────────────────
// WorkloadEntry + WorkloadGroup
// ─────────────────────────────────────────────────────────────

/// Represents a single VM/bare-metal workload endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadEntry {
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub address: String,
    pub ports: HashMap<String, u16>,
    pub labels: HashMap<String, String>,
    pub weight: u32,
    pub network: Option<String>,
    pub locality: Option<String>,
    pub service_account: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl WorkloadEntry {
    pub fn new(address: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: None,
            namespace: None,
            address: address.into(),
            ports: HashMap::new(),
            labels: HashMap::new(),
            weight: 100,
            network: None,
            locality: None,
            service_account: None,
            created_at: Some(now),
            updated_at: Some(now),
        }
    }
}

// WorkloadGroup, Sidecar, EnvoyFilter and their dependent types are
// intentionally absent — sidecar legacy plane removed per Cave Runtime
// no-backcompat Ambient-only mandate. See `src/ambient/` for the
// ztunnel+waypoint surface that replaces them.

// ─────────────────────────────────────────────────────────────
// PeerAuthentication (mTLS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAuthentication {
    pub name: String,
    pub namespace: String,
    pub selector: Option<HashMap<String, String>>,
    pub mtls: MtlsConfig,
    pub port_level_mtls: HashMap<u16, MtlsConfig>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtlsConfig {
    pub mode: MtlsMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MtlsMode {
    Strict,
    Permissive,
    Disable,
    /// Unset — inherits from parent scope.
    Unset,
}

// ─────────────────────────────────────────────────────────────
// RequestAuthentication (JWT / OIDC)
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
    pub jwks: Option<String>,
    pub from_headers: Vec<JwtHeader>,
    pub from_params: Vec<String>,
    pub from_cookies: Vec<String>,
    pub forward_original_token: bool,
    pub output_claim_to_headers: Vec<ClaimToHeader>,
    pub timeout_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtHeader {
    pub name: String,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimToHeader {
    pub header: String,
    pub claim: String,
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
    /// Provider name for CUSTOM action (external authz).
    pub provider: Option<ExtensionProvider>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthzAction {
    Allow,
    Deny,
    /// Delegate to an external authorization provider.
    Custom,
    /// Log the request but do not enforce.
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionProvider {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthzRule {
    pub from: Vec<Source>,
    pub to: Vec<Operation>,
    pub when: Vec<Condition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Source {
    pub principals: Vec<String>,
    pub request_principals: Vec<String>,
    pub namespaces: Vec<String>,
    pub ip_blocks: Vec<String>,
    pub remote_ip_blocks: Vec<String>,
    pub not_principals: Vec<String>,
    pub not_request_principals: Vec<String>,
    pub not_namespaces: Vec<String>,
    pub not_ip_blocks: Vec<String>,
    pub not_remote_ip_blocks: Vec<String>,
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
    pub key: String,
    pub values: Vec<String>,
    pub not_values: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// Telemetry API (Istio Telemetry resource)
// ─────────────────────────────────────────────────────────────

/// Controls metrics/logs/tracing per workload (Istio Telemetry API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Telemetry {
    pub name: String,
    pub namespace: String,
    pub selector: Option<HashMap<String, String>>,
    pub tracing: Vec<Tracing>,
    pub metrics: Vec<Metrics>,
    pub access_logging: Vec<AccessLogging>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Telemetry {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            namespace: namespace.into(),
            selector: None,
            tracing: vec![],
            metrics: vec![],
            access_logging: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tracing {
    pub providers: Vec<ProviderRef>,
    pub random_sampling_percentage: Option<f64>,
    pub disable_span_reporting: Option<bool>,
    pub custom_tags: HashMap<String, TraceTag>,
    pub use_request_id_for_trace_sampling: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceTag {
    Literal(String),
    Environment { name: String, default_value: Option<String> },
    Header { name: String, default_value: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub providers: Vec<ProviderRef>,
    pub overrides: Vec<MetricsOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsOverride {
    pub match_rule: MetricSelector,
    pub disabled: Option<bool>,
    pub tag_overrides: HashMap<String, MetricTagOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSelector {
    pub metric: Option<MetricType>,
    pub mode: Option<WorkloadMode>,
    pub custom_metric: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MetricType {
    AllMetrics,
    RequestCount,
    RequestDuration,
    RequestSize,
    ResponseSize,
    TcpSentBytes,
    TcpReceivedBytes,
    TcpConnections,
    GrpcRequestMessages,
    GrpcResponseMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkloadMode {
    ClientAndServer,
    Client,
    Server,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricTagOverride {
    pub operation: MetricTagOperation,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MetricTagOperation {
    Upsert,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogging {
    pub providers: Vec<ProviderRef>,
    pub disabled: Option<bool>,
    pub filter: Option<AccessLogFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogFilter {
    /// CEL expression for conditional logging (e.g. "response.code >= 400").
    pub expression: String,
}

// ─────────────────────────────────────────────────────────────
// Rate Limit Policy (CAVE extension)
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
    pub fn to_rps(&self, requests_per_unit: u64) -> f64 {
        match self {
            RateLimitUnit::Second => requests_per_unit as f64,
            RateLimitUnit::Minute => requests_per_unit as f64 / 60.0,
            RateLimitUnit::Hour => requests_per_unit as f64 / 3600.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// SPIFFE / certificate types
// ─────────────────────────────────────────────────────────────

/// A parsed SPIFFE ID (spiffe://<trust-domain>/<path>).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SpiffeId {
    pub trust_domain: String,
    pub path: String,
}

impl SpiffeId {
    /// Parse from a SPIFFE URI string.
    pub fn parse(uri: &str) -> Option<Self> {
        let stripped = uri.strip_prefix("spiffe://")?;
        let (trust_domain, path) = stripped.split_once('/')?;
        Some(Self {
            trust_domain: trust_domain.to_string(),
            path: format!("/{path}"),
        })
    }

    /// Format as a SPIFFE URI.
    pub fn to_uri(&self) -> String {
        format!("spiffe://{}{}", self.trust_domain, self.path)
    }

    /// Build the canonical Istio SPIFFE ID for a workload.
    pub fn for_workload(trust_domain: &str, namespace: &str, service_account: &str) -> Self {
        Self {
            trust_domain: trust_domain.to_string(),
            path: format!("/ns/{namespace}/sa/{service_account}"),
        }
    }
}

impl std::fmt::Display for SpiffeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_uri())
    }
}

/// DER-encoded X.509 certificate bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertBundle {
    pub spiffe_id: SpiffeId,
    /// PEM-encoded certificate chain.
    pub cert_pem: String,
    /// PEM-encoded private key (stored only in the issuing CA's memory).
    pub key_pem: Option<String>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub serial: String,
}

// ─────────────────────────────────────────────────────────────
// Traffic Decision (output of TrafficManager)
// ─────────────────────────────────────────────────────────────

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
    pub request_headers_remove: Vec<String>,
    pub response_headers_add: HashMap<String, String>,
    pub response_headers_remove: Vec<String>,
    pub traceparent: Option<String>,
    pub redirect: Option<HttpRedirect>,
    pub rewrite: Option<HttpRewrite>,
    pub mirror: Option<MirrorDecision>,
    pub cors_policy: Option<CorsPolicy>,
}

#[derive(Debug, Clone)]
pub struct MirrorDecision {
    pub host: String,
    pub subset: Option<String>,
    pub port: Option<u16>,
    pub percentage: f64,
}

#[derive(Debug, Clone)]
pub enum FaultEffect {
    Delay(u64),
    Abort(u16),
}

/// Inbound request descriptor consumed by the traffic engine.
#[derive(Debug, Clone, Default)]
pub struct IncomingRequest {
    pub uri: String,
    pub method: String,
    pub authority: Option<String>,
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub source_labels: HashMap<String, String>,
    pub source_namespace: Option<String>,
    pub traceparent: Option<String>,
    pub tracestate: Option<String>,
    pub gateway: Option<String>,
}

/// Authorization check context.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub source_principal: Option<String>,
    pub source_namespace: Option<String>,
    pub source_ip: Option<String>,
    pub remote_ip: Option<String>,
    pub method: String,
    pub path: String,
    pub host: String,
    pub port: Option<u16>,
    pub jwt_claims: Option<HashMap<String, serde_json::Value>>,
    pub request_principal: Option<String>,
}

/// mTLS peer connection descriptor.
#[derive(Debug, Clone, Default)]
pub struct TlsContext {
    pub peer_principal: Option<String>,
    pub is_mtls: bool,
    pub peer_cert_san: Vec<String>,
}
