//! Data models for cave-gateway — Kong + Gravitee unified.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ── Kong-side models ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub name: String,
    pub path_prefix: String,
    pub methods: Vec<String>,
    pub upstream_id: Uuid,
    pub plugins: Vec<PluginConfig>,
    pub rate_limit: Option<RateLimitConfig>,
    pub auth: Option<AuthConfig>,
    pub strip_prefix: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamService {
    pub id: Uuid,
    pub name: String,
    pub lb_algorithm: LbAlgorithm,
    pub nodes: Vec<UpstreamNode>,
    pub health_check: Option<HealthCheckConfig>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamNode {
    pub id: Uuid,
    pub address: String,
    pub weight: u32,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LbAlgorithm {
    RoundRobin,
    LeastConnections,
    Weighted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub path: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub healthy_threshold: u32,
    pub unhealthy_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub algorithm: RateLimitAlgorithm,
    pub requests_per_second: f64,
    pub burst: u32,
    pub key_by: RateLimitKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RateLimitAlgorithm {
    TokenBucket,
    SlidingWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RateLimitKey {
    Ip,
    ApiKey,
    UserId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub method: AuthMethod,
    pub jwt_secret: Option<String>,
    pub api_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    None,
    Jwt,
    ApiKey,
    OAuth2Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PluginConfig {
    Cors(CorsConfig),
    RequestSizeLimit(RequestSizeLimitConfig),
    IpRestriction(IpRestrictionConfig),
    BotDetection(BotDetectionConfig),
    RequestTransform(RequestTransformConfig),
    ResponseTransform(ResponseTransformConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allow_credentials: bool,
    pub max_age_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSizeLimitConfig {
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpRestrictionConfig {
    pub allow_list: Vec<String>,
    pub deny_list: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotDetectionConfig {
    pub block_known_bots: bool,
    pub custom_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestTransformConfig {
    pub add_headers: Vec<(String, String)>,
    pub remove_headers: Vec<String>,
    pub rename_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTransformConfig {
    pub add_headers: Vec<(String, String)>,
    pub remove_headers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayMetrics {
    pub total_requests: u64,
    pub requests_allowed: u64,
    pub requests_blocked: u64,
    pub auth_failures: u64,
    pub rate_limit_hits: u64,
    pub circuit_breaker_trips: u64,
    pub upstream_errors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerStatus {
    pub upstream_id: Uuid,
    pub upstream_name: String,
    pub state: String,
    pub failure_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct CheckRequest {
    pub path: String,
    pub method: String,
    pub client_ip: String,
    pub auth_header: Option<String>,
    pub user_agent: Option<String>,
    pub body_size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct CheckResponse {
    pub allowed: bool,
    pub route_matched: Option<String>,
    pub upstream_address: Option<String>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRouteRequest {
    pub name: String,
    pub path_prefix: String,
    pub methods: Vec<String>,
    pub upstream_id: Uuid,
    pub plugins: Option<Vec<PluginConfig>>,
    pub rate_limit: Option<RateLimitConfig>,
    pub auth: Option<AuthConfig>,
    pub strip_prefix: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUpstreamRequest {
    pub name: String,
    pub lb_algorithm: LbAlgorithm,
    pub nodes: Vec<CreateNodeRequest>,
    pub health_check: Option<HealthCheckConfig>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNodeRequest {
    pub address: String,
    pub weight: Option<u32>,
}

// ── Gravitee-side models ──────────────────────────────────────────────────────

// --- API Designer & Quality ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSpec {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub format: ApiFormat,
    /// Raw YAML or JSON content of the spec.
    pub content: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApiFormat {
    OpenApi30,
    OpenApi31,
    AsyncApi20,
    AsyncApi26,
    GraphQL,
    Protobuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiQualityScore {
    pub spec_id: Uuid,
    /// Weighted composite 0–100.
    pub overall: f64,
    pub documentation: f64,
    pub security: f64,
    pub design: f64,
    pub completeness: f64,
    pub issues: Vec<QualityIssue>,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityIssue {
    pub severity: IssueSeverity,
    pub category: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Deserialize)]
pub struct CreateSpecRequest {
    pub name: String,
    pub version: String,
    pub format: ApiFormat,
    pub content: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSpecRequest {
    pub content: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MockRequest {
    pub path: String,
    pub method: String,
}

// --- Marketplace / Developer Portal ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionPlan {
    pub id: Uuid,
    pub name: String,
    pub tier: PlanTier,
    pub rate_limit: Option<RateLimitConfig>,
    pub max_api_keys: u32,
    pub price_per_month: f64,
    pub price_per_1k_requests: f64,
    pub included_requests: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlanTier {
    Free,
    Basic,
    Pro,
    Enterprise,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConsumer {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub organization: Option<String>,
    pub api_keys: Vec<ApiKeyEntry>,
    pub subscriptions: Vec<ConsumerSubscription>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub key: String,
    pub label: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerSubscription {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub api_id: Option<Uuid>,
    pub active: bool,
    pub subscribed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerDashboard {
    pub consumer_id: Uuid,
    pub consumer_name: String,
    pub total_requests_this_month: u64,
    pub active_keys: usize,
    pub active_subscriptions: usize,
    pub top_apis: Vec<(String, u64)>,
}

#[derive(Debug, Deserialize)]
pub struct CreateConsumerRequest {
    pub name: String,
    pub email: String,
    pub organization: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePlanRequest {
    pub name: String,
    pub tier: PlanTier,
    pub max_api_keys: Option<u32>,
    pub price_per_month: Option<f64>,
    pub price_per_1k_requests: Option<f64>,
    pub included_requests: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisionKeyRequest {
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub plan_id: Uuid,
    pub api_id: Option<Uuid>,
}

// --- Monetization / Billing ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingPlan {
    pub id: Uuid,
    pub name: String,
    pub pricing_model: PricingModel,
    pub base_price: f64,
    pub tiers: Vec<PricingTier>,
    pub billing_period_days: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PricingModel {
    PerRequest,
    PerMonth,
    Tiered,
    UsageBased,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingTier {
    pub from_requests: u64,
    pub to_requests: Option<u64>,
    pub price_per_1k: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageMeter {
    pub consumer_id: Uuid,
    pub api_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub latency_samples: Vec<u64>,
}

impl UsageMeter {
    pub fn avg_latency_ms(&self) -> f64 {
        if self.latency_samples.is_empty() {
            return 0.0;
        }
        self.latency_samples.iter().sum::<u64>() as f64 / self.latency_samples.len() as f64
    }

    pub fn p99_latency_ms(&self) -> f64 {
        if self.latency_samples.is_empty() {
            return 0.0;
        }
        let mut sorted = self.latency_samples.clone();
        sorted.sort_unstable();
        let idx = (sorted.len() as f64 * 0.99) as usize;
        sorted[idx.min(sorted.len() - 1)] as f64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub billing_plan_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub lines: Vec<InvoiceLine>,
    pub total_amount: f64,
    pub currency: String,
    pub status: InvoiceStatus,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvoiceStatus {
    Draft,
    Issued,
    Paid,
    Overdue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub description: String,
    pub quantity: u64,
    pub unit_price: f64,
    pub amount: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateBillingPlanRequest {
    pub name: String,
    pub pricing_model: PricingModel,
    pub base_price: Option<f64>,
    pub tiers: Option<Vec<PricingTier>>,
    pub billing_period_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RecordUsageRequest {
    pub consumer_id: Uuid,
    pub api_id: Uuid,
    pub requests: u64,
    pub successful: u64,
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateInvoiceRequest {
    pub consumer_id: Uuid,
    pub billing_plan_id: Uuid,
    pub period_days: Option<u32>,
}

// --- API Lifecycle & Review ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiLifecycle {
    pub id: Uuid,
    pub api_name: String,
    pub version: String,
    pub state: LifecycleState,
    pub spec_id: Option<Uuid>,
    pub changelog: Vec<ChangelogEntry>,
    pub migration_guide: Option<String>,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub retire_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LifecycleState {
    Draft,
    PendingReview,
    Published,
    Deprecated,
    Retired,
}

impl std::fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "Draft"),
            Self::PendingReview => write!(f, "PendingReview"),
            Self::Published => write!(f, "Published"),
            Self::Deprecated => write!(f, "Deprecated"),
            Self::Retired => write!(f, "Retired"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: DateTime<Utc>,
    pub description: String,
    pub breaking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub id: Uuid,
    pub api_lifecycle_id: Uuid,
    pub submitted_by: String,
    pub status: ReviewStatus,
    pub comments: Vec<ReviewComment>,
    pub submitted_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub author: String,
    pub comment: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub action: String,
    pub actor: String,
    pub details: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiVersionRequest {
    pub api_name: String,
    pub version: String,
    pub spec_id: Option<Uuid>,
    pub migration_guide: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub target_state: LifecycleState,
    pub reason: Option<String>,
    pub actor: String,
}

#[derive(Debug, Deserialize)]
pub struct SubmitReviewRequest {
    pub submitted_by: String,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewDecisionRequest {
    pub reviewer: String,
    pub comment: String,
}

#[derive(Debug, Deserialize)]
pub struct AddChangelogRequest {
    pub description: String,
    pub breaking: Option<bool>,
}

// --- Multi-Protocol Gateway ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProtocolType {
    Http,
    Grpc,
    WebSocket,
    GraphQL,
    Mqtt,
    Sse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolEndpoint {
    pub id: Uuid,
    pub name: String,
    pub protocol: ProtocolType,
    pub listen_path: String,
    pub upstream_address: String,
    pub config: ProtocolConfig,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "protocol")]
pub enum ProtocolConfig {
    Http(HttpProtocolConfig),
    Grpc(GrpcProtocolConfig),
    WebSocket(WebSocketProtocolConfig),
    GraphQL(GraphQLProtocolConfig),
    Mqtt(MqttProtocolConfig),
    Sse(SseProtocolConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProtocolConfig {
    pub methods: Vec<String>,
    pub strip_path: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcProtocolConfig {
    /// gRPC service name e.g. "mypackage.MyService"
    pub service: String,
    pub methods: Vec<String>,
    /// Transcode HTTP/JSON → gRPC automatically.
    pub transcoding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketProtocolConfig {
    pub idle_timeout_secs: u64,
    pub max_message_size_bytes: usize,
    pub ping_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLProtocolConfig {
    pub schema: String,
    pub allow_introspection: bool,
    pub max_depth: u32,
    pub max_complexity: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttProtocolConfig {
    pub broker_address: String,
    pub topic_prefix: String,
    pub qos: u8,
    pub retain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseProtocolConfig {
    pub event_source_path: String,
    pub heartbeat_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct RegisterEndpointRequest {
    pub name: String,
    pub protocol: ProtocolType,
    pub listen_path: String,
    pub upstream_address: String,
    pub config: ProtocolConfig,
}

#[derive(Debug, Deserialize)]
pub struct RouteMessageRequest {
    pub protocol: ProtocolType,
    pub topic_or_path: String,
    pub payload: Option<Value>,
}

// --- Flow-based Policy Designer ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFlow {
    pub id: Uuid,
    pub name: String,
    /// None = global flow applied to all APIs.
    pub api_id: Option<Uuid>,
    pub pre_route: Vec<PolicyStep>,
    pub route: Vec<PolicyStep>,
    pub post_route: Vec<PolicyStep>,
    pub error: Vec<PolicyStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStep {
    pub id: Uuid,
    pub policy_type: PolicyType,
    /// Optional condition expression — None means always execute.
    pub condition: Option<String>,
    pub config: Value,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicyType {
    RateLimit,
    Auth,
    Transform,
    Cache,
    CircuitBreaker,
    Retry,
    Logger,
    Mock,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEvaluation {
    pub flow_id: Uuid,
    pub path: String,
    pub method: String,
    pub executed_steps: Vec<ExecutedStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutedStep {
    pub stage: String,
    pub step_id: Uuid,
    pub policy_type: PolicyType,
    pub would_execute: bool,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFlowRequest {
    pub name: String,
    pub api_id: Option<Uuid>,
    pub pre_route: Option<Vec<PolicyStep>>,
    pub route: Option<Vec<PolicyStep>>,
    pub post_route: Option<Vec<PolicyStep>>,
    pub error: Option<Vec<PolicyStep>>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateFlowRequest {
    pub path: String,
    pub method: String,
    pub headers: Option<std::collections::HashMap<String, String>>,
}
