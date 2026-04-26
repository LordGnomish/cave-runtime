//! Data models for cave-knative (Serving + Eventing).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Knative Serving ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnService {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: ServiceSpec,
    pub status: ServiceStatus,
    pub latest_ready_revision: Option<String>,
    pub latest_created_revision: Option<String>,
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub template: RevisionTemplate,
    pub traffic: Vec<TrafficTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionTemplate {
    pub container: Container,
    pub scale: ScaleConfig,
    pub service_account: Option<String>,
    pub timeout_seconds: u32,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

impl Default for RevisionTemplate {
    fn default() -> Self {
        Self {
            container: Container::default(),
            scale: ScaleConfig::default(),
            service_account: None,
            timeout_seconds: 300,
            labels: HashMap::new(),
            annotations: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Container {
    pub image: String,
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub ports: Vec<ContainerPort>,
    pub resources: ResourceRequirements,
    pub readiness_probe: Option<Probe>,
    pub liveness_probe: Option<Probe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: Option<String>,
    pub value_from: Option<EnvVarSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarSource {
    pub secret_key_ref: Option<SecretKeySelector>,
    pub config_map_key_ref: Option<ConfigMapKeySelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretKeySelector {
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapKeySelector {
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContainerPort {
    pub name: Option<String>,
    pub container_port: u16,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceRequirements {
    pub requests: HashMap<String, String>,
    pub limits: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub http_get: Option<HttpGetAction>,
    pub initial_delay_seconds: u32,
    pub period_seconds: u32,
    pub timeout_seconds: u32,
    pub failure_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpGetAction {
    pub path: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleConfig {
    pub min_scale: u32,
    pub max_scale: Option<u32>,
    pub target_concurrency: Option<u32>,
    pub utilization_percentage: Option<u32>,
    pub scale_to_zero_grace_period_secs: u64,
    pub scale_up_rate: Option<f64>,
    pub scale_down_rate: Option<f64>,
}

impl Default for ScaleConfig {
    fn default() -> Self {
        Self {
            min_scale: 0,
            max_scale: None,
            target_concurrency: Some(100),
            utilization_percentage: Some(70),
            scale_to_zero_grace_period_secs: 30,
            scale_up_rate: Some(1000.0),
            scale_down_rate: Some(2.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficTarget {
    pub revision_name: Option<String>,
    pub latest_revision: Option<bool>,
    pub percent: u32,
    pub tag: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub id: Uuid,
    pub name: String,
    pub service_name: String,
    pub namespace: String,
    pub spec: RevisionSpec,
    pub status: RevisionStatus,
    pub current_replicas: u32,
    pub desired_replicas: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionSpec {
    pub container: Container,
    pub scale: ScaleConfig,
    pub service_account: Option<String>,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    Active,
    Reserve,
    Inactive,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnRoute {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec_traffic: Vec<TrafficTarget>,
    pub status_traffic: Vec<TrafficTarget>,
    pub url: String,
    pub status: RouteStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RouteStatus {
    Ready,
    NotReady,
    Unknown,
}

// ── Knative Eventing ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudEvent {
    pub specversion: String,
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub source: String,
    pub subject: Option<String>,
    pub time: Option<DateTime<Utc>>,
    pub datacontenttype: Option<String>,
    pub data: Option<serde_json::Value>,
    pub extensions: HashMap<String, serde_json::Value>,
}

impl CloudEvent {
    pub fn new(event_type: impl Into<String>, source: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            specversion: "1.0".into(),
            id: Uuid::new_v4().to_string(),
            event_type: event_type.into(),
            source: source.into(),
            subject: None,
            time: Some(Utc::now()),
            datacontenttype: Some("application/json".into()),
            data: Some(data),
            extensions: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Broker {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub broker_class: BrokerClass,
    pub config: BrokerConfig,
    pub status: BrokerStatus,
    pub address: Option<BrokerAddress>,
    pub event_count: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    pub delivery: Option<DeliverySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliverySpec {
    pub dead_letter_sink: Option<Addressable>,
    pub retry: Option<u32>,
    pub backoff_policy: Option<BackoffPolicy>,
    pub backoff_delay_secs: Option<u64>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Addressable {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackoffPolicy {
    Linear,
    Exponential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerClass {
    MTChannelBasedBroker,
    KafkaBroker,
    RabbitMQBroker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerAddress {
    pub url: String,
    pub audience: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub broker: String,
    pub filter: TriggerFilter,
    pub subscriber: Addressable,
    pub delivery: Option<DeliverySpec>,
    pub status: TriggerStatus,
    pub event_count: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TriggerFilter {
    pub attributes: HashMap<String, String>,
    pub filters: Vec<FilterExpression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FilterExpression {
    Exact { attribute: String, value: String },
    Prefix { attribute: String, prefix: String },
    Suffix { attribute: String, suffix: String },
    Any { filters: Vec<FilterExpression> },
    All { filters: Vec<FilterExpression> },
    Not { filter: Box<FilterExpression> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub source_type: SourceType,
    pub spec: EventSourceSpec,
    pub sink: Addressable,
    pub status: SourceStatus,
    pub event_count: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    ApiServerSource,
    PingSource,
    SinkBinding,
    KafkaSource,
    GitHubSource,
    AwsSqs,
    GcpPubSub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSourceSpec {
    pub schedule: Option<String>,
    pub data: Option<serde_json::Value>,
    pub resources: Vec<ApiServerResource>,
    pub kafka_topics: Vec<String>,
    pub ping_data: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiServerResource {
    pub api_version: String,
    pub kind: String,
    pub event_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub channel_type: ChannelType,
    pub status: ChannelStatus,
    pub address: Option<String>,
    pub event_count: u64,
    pub subscriber_count: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    InMemoryChannel,
    KafkaChannel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub channel: String,
    pub subscriber: Option<Addressable>,
    pub reply: Option<Addressable>,
    pub delivery: Option<DeliverySpec>,
    pub status: SubscriptionStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Ready,
    NotReady,
    Unknown,
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub namespace: String,
    pub template: RevisionTemplate,
    pub traffic: Option<Vec<TrafficTarget>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServiceRequest {
    pub template: Option<RevisionTemplate>,
    pub traffic: Option<Vec<TrafficTarget>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBrokerRequest {
    pub name: String,
    pub namespace: String,
    pub broker_class: Option<BrokerClass>,
    pub delivery: Option<DeliverySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTriggerRequest {
    pub name: String,
    pub namespace: String,
    pub broker: String,
    pub filter: Option<TriggerFilter>,
    pub subscriber_uri: String,
    pub delivery: Option<DeliverySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSourceRequest {
    pub name: String,
    pub namespace: String,
    pub source_type: SourceType,
    pub spec: EventSourceSpec,
    pub sink_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub namespace: String,
    pub channel_type: Option<ChannelType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub name: String,
    pub namespace: String,
    pub channel: String,
    pub subscriber_uri: Option<String>,
    pub reply_uri: Option<String>,
    pub delivery: Option<DeliverySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleRequest {
    pub replicas: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendEventRequest {
    pub event_type: String,
    pub source: String,
    pub data: serde_json::Value,
    pub extensions: Option<HashMap<String, serde_json::Value>>,
}
