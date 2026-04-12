//! xDS v3 control-plane API.
//!
//! Implements:
//!   • Full resource-type taxonomy: LDS, RDS, CDS, EDS, SDS, RTDS, ECDS
//!   • XdsSnapshot — per-node config snapshot (consistent versioning)
//!   • XdsManager  — manages snapshots, distributes updates to proxies
//!   • DeltaXdsState — incremental (delta) xDS subscription tracking
//!   • Config validation helpers
//!   • Status reporting (node sync state)
//!
//! Note: This is a pure Rust implementation of the xDS *data model*.
//!       Actual gRPC transport (ADS/SotW/Delta) is wired via the admin API
//!       (REST snapshot endpoints) or a future tonic-based transport layer.

use crate::models::{DestinationRule, Gateway, ServiceEntry, VirtualService};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{debug, info};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────
// xDS resource type constants (TypeURL format)
// ─────────────────────────────────────────────────────────────

pub const LDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.config.listener.v3.Listener";
pub const RDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.config.route.v3.RouteConfiguration";
pub const CDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.config.cluster.v3.Cluster";
pub const EDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.config.endpoint.v3.ClusterLoadAssignment";
pub const SDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.Secret";
pub const RTDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.service.runtime.v3.Runtime";
pub const ECDS_TYPE_URL: &str =
    "type.googleapis.com/envoy.config.core.v3.TypedExtensionConfig";

// ─────────────────────────────────────────────────────────────
// Resource type enum
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum XdsResourceType {
    /// Listener Discovery Service
    Lds,
    /// Route Discovery Service
    Rds,
    /// Cluster Discovery Service
    Cds,
    /// Endpoint Discovery Service
    Eds,
    /// Secret Discovery Service
    Sds,
    /// Runtime Discovery Service
    Rtds,
    /// Extension Config Discovery Service
    Ecds,
}

impl XdsResourceType {
    pub fn type_url(&self) -> &'static str {
        match self {
            XdsResourceType::Lds => LDS_TYPE_URL,
            XdsResourceType::Rds => RDS_TYPE_URL,
            XdsResourceType::Cds => CDS_TYPE_URL,
            XdsResourceType::Eds => EDS_TYPE_URL,
            XdsResourceType::Sds => SDS_TYPE_URL,
            XdsResourceType::Rtds => RTDS_TYPE_URL,
            XdsResourceType::Ecds => ECDS_TYPE_URL,
        }
    }

    pub fn from_type_url(url: &str) -> Option<Self> {
        match url {
            LDS_TYPE_URL => Some(Self::Lds),
            RDS_TYPE_URL => Some(Self::Rds),
            CDS_TYPE_URL => Some(Self::Cds),
            EDS_TYPE_URL => Some(Self::Eds),
            SDS_TYPE_URL => Some(Self::Sds),
            RTDS_TYPE_URL => Some(Self::Rtds),
            ECDS_TYPE_URL => Some(Self::Ecds),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Envoy Listener (LDS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsListener {
    pub name: String,
    pub address: SocketAddress,
    pub filter_chains: Vec<FilterChain>,
    pub listener_filters: Vec<ListenerFilter>,
    pub use_original_dst: bool,
    pub traffic_direction: TrafficDirection,
    pub stat_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketAddress {
    pub address: String,
    pub port_value: u32,
    pub protocol: SocketProtocol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SocketProtocol {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TrafficDirection {
    Unspecified,
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterChain {
    pub name: Option<String>,
    pub filter_chain_match: Option<FilterChainMatch>,
    pub filters: Vec<NetworkFilter>,
    pub transport_socket: Option<TransportSocket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterChainMatch {
    pub destination_port: Option<u32>,
    pub prefix_ranges: Vec<CidrRange>,
    pub server_names: Vec<String>,
    pub transport_protocol: Option<String>,
    pub application_protocols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidrRange {
    pub address_prefix: String,
    pub prefix_len: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkFilter {
    pub name: String,
    pub typed_config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerFilter {
    pub name: String,
    pub typed_config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportSocket {
    pub name: String,
    pub typed_config: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────
// Route Configuration (RDS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRouteConfiguration {
    pub name: String,
    pub virtual_hosts: Vec<XdsVirtualHost>,
    pub request_headers_to_add: Vec<XdsHeaderValueOption>,
    pub response_headers_to_add: Vec<XdsHeaderValueOption>,
    pub request_headers_to_remove: Vec<String>,
    pub response_headers_to_remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsVirtualHost {
    pub name: String,
    pub domains: Vec<String>,
    pub routes: Vec<XdsRoute>,
    pub cors: Option<XdsCorsPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRoute {
    pub name: Option<String>,
    pub match_rule: XdsRouteMatch,
    pub action: XdsRouteAction,
    pub request_headers_to_add: Vec<XdsHeaderValueOption>,
    pub response_headers_to_add: Vec<XdsHeaderValueOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRouteMatch {
    pub path_specifier: XdsPathSpecifier,
    pub headers: Vec<XdsHeaderMatcher>,
    pub query_parameters: Vec<XdsQueryParamMatcher>,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsPathSpecifier {
    Prefix(String),
    Path(String),
    SafeRegex(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsHeaderMatcher {
    pub name: String,
    pub header_match_specifier: XdsHeaderMatchSpecifier,
    pub invert_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsHeaderMatchSpecifier {
    ExactMatch(String),
    PrefixMatch(String),
    SuffixMatch(String),
    SafeRegexMatch(String),
    PresentMatch(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsQueryParamMatcher {
    pub name: String,
    pub string_match: Option<String>,
    pub present_match: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsRouteAction {
    Route(XdsRouteActionRoute),
    Redirect(XdsRedirectAction),
    DirectResponse(XdsDirectResponseAction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRouteActionRoute {
    pub cluster_specifier: XdsClusterSpecifier,
    pub timeout_ms: Option<u64>,
    pub retry_policy: Option<XdsRetryPolicy>,
    pub request_mirror_policies: Vec<XdsRequestMirrorPolicy>,
    pub host_rewrite: Option<String>,
    pub prefix_rewrite: Option<String>,
    pub hash_policy: Vec<XdsHashPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsClusterSpecifier {
    Cluster(String),
    WeightedClusters(Vec<XdsWeightedCluster>),
    ClusterHeader(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsWeightedCluster {
    pub name: String,
    pub weight: u32,
    pub request_headers_to_add: Vec<XdsHeaderValueOption>,
    pub response_headers_to_add: Vec<XdsHeaderValueOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRetryPolicy {
    pub retry_on: String,
    pub num_retries: u32,
    pub per_try_timeout_ms: Option<u64>,
    pub retriable_status_codes: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRequestMirrorPolicy {
    pub cluster: String,
    pub runtime_fraction: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsHashPolicy {
    pub policy_specifier: XdsHashPolicySpecifier,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsHashPolicySpecifier {
    Header { header_name: String },
    Cookie { name: String, ttl_seconds: Option<u64> },
    ConnectionProperties { source_ip: bool },
    QueryParameter { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsRedirectAction {
    pub scheme_rewrite: Option<String>,
    pub host_redirect: Option<String>,
    pub port_redirect: Option<u32>,
    pub path_redirect: Option<String>,
    pub response_code: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsDirectResponseAction {
    pub status: u32,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsHeaderValueOption {
    pub header: XdsHeaderValue,
    pub append: Option<bool>,
    pub keep_empty_value: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsHeaderValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsCorsPolicy {
    pub allow_origins: Vec<String>,
    pub allow_methods: String,
    pub allow_headers: String,
    pub expose_headers: String,
    pub max_age: Option<String>,
    pub allow_credentials: bool,
}

// ─────────────────────────────────────────────────────────────
// Cluster (CDS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsCluster {
    pub name: String,
    pub cluster_type: XdsClusterType,
    pub eds_cluster_config: Option<XdsEdsClusterConfig>,
    pub load_assignment: Option<XdsClusterLoadAssignment>,
    pub connect_timeout_ms: u64,
    pub lb_policy: XdsLbPolicy,
    pub circuit_breakers: Option<XdsCircuitBreakers>,
    pub outlier_detection: Option<XdsOutlierDetection>,
    pub http2_protocol_options: Option<XdsHttp2Options>,
    pub transport_socket: Option<TransportSocket>,
    pub upstream_http_protocol_options: Option<XdsUpstreamHttpOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsClusterType {
    Static,
    Strict,
    LogicalDns,
    Eds,
    OriginalDst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsEdsClusterConfig {
    pub eds_config: XdsConfigSource,
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsConfigSource {
    pub resource_api_version: String,
    pub api_type: XdsApiType,
    pub cluster_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsApiType {
    Rest,
    Grpc,
    Ads,
    Delta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsLbPolicy {
    RoundRobin,
    LeastRequest,
    RingHash,
    Random,
    Maglev,
    ClusterProvided,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsCircuitBreakers {
    pub thresholds: Vec<XdsCircuitBreakerThreshold>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsCircuitBreakerThreshold {
    pub priority: XdsRoutingPriority,
    pub max_connections: u32,
    pub max_pending_requests: u32,
    pub max_requests: u32,
    pub max_retries: u32,
    pub max_connection_pools: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsRoutingPriority {
    Default,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsOutlierDetection {
    pub consecutive_5xx: Option<u32>,
    pub consecutive_gateway_failure: Option<u32>,
    pub interval_ms: Option<u64>,
    pub base_ejection_time_ms: Option<u64>,
    pub max_ejection_percent: Option<u32>,
    pub split_external_local_origin_errors: bool,
    pub consecutive_local_origin_failure: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsHttp2Options {
    pub max_concurrent_streams: Option<u32>,
    pub initial_stream_window_size: Option<u32>,
    pub initial_connection_window_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsUpstreamHttpOptions {
    pub upstream_http_protocol: XdsUpstreamHttpProtocol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsUpstreamHttpProtocol {
    HttpAuto,
    Http1,
    Http2,
}

// ─────────────────────────────────────────────────────────────
// Endpoint (EDS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsClusterLoadAssignment {
    pub cluster_name: String,
    pub endpoints: Vec<XdsLocalityLbEndpoints>,
    pub policy: Option<XdsLoadBalancingPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsLocalityLbEndpoints {
    pub locality: Option<XdsLocality>,
    pub lb_endpoints: Vec<XdsLbEndpoint>,
    pub load_balancing_weight: Option<u32>,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsLocality {
    pub region: String,
    pub zone: Option<String>,
    pub sub_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsLbEndpoint {
    pub endpoint: XdsEndpoint,
    pub health_status: XdsHealthStatus,
    pub load_balancing_weight: Option<u32>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsEndpoint {
    pub address: SocketAddress,
    pub additional_addresses: Vec<SocketAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsHealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
    Draining,
    Timeout,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsLoadBalancingPolicy {
    pub drop_overloads: Vec<XdsDropOverload>,
    pub overprovisioning_factor: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsDropOverload {
    pub category: String,
    pub drop_percentage: f64,
}

// ─────────────────────────────────────────────────────────────
// Secret (SDS)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsSecret {
    pub name: String,
    pub secret_type: XdsSecretType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XdsSecretType {
    TlsCertificate {
        certificate_chain_pem: String,
        private_key_pem: String,
    },
    ValidationContext {
        trusted_ca_pem: String,
        verify_certificate_spki: Vec<String>,
        match_typed_subject_alt_names: Vec<XdsSanMatcher>,
    },
    CombinedValidationContext {
        default_validation_context: Box<XdsSecretType>,
        combined_validation_context_type: Box<XdsSecretType>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsSanMatcher {
    pub san_type: XdsSanType,
    pub matcher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum XdsSanType {
    Email,
    Dns,
    Uri,
    IpAddress,
}

// ─────────────────────────────────────────────────────────────
// Node info (proxy identification)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    pub cluster: String,
    pub locality: Option<XdsLocality>,
    pub metadata: serde_json::Value,
    pub user_agent_name: Option<String>,
    pub user_agent_version: Option<String>,
}

// ─────────────────────────────────────────────────────────────
// XdsSnapshot — per-node config snapshot
// ─────────────────────────────────────────────────────────────

/// A point-in-time configuration snapshot for one or more proxy nodes.
/// All resource types share a consistent version string so nodes can detect
/// whether they are up to date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdsSnapshot {
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub listeners: HashMap<String, XdsListener>,
    pub routes: HashMap<String, XdsRouteConfiguration>,
    pub clusters: HashMap<String, XdsCluster>,
    pub endpoints: HashMap<String, XdsClusterLoadAssignment>,
    pub secrets: HashMap<String, XdsSecret>,
}

impl XdsSnapshot {
    pub fn empty() -> Self {
        Self {
            version: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            listeners: HashMap::new(),
            routes: HashMap::new(),
            clusters: HashMap::new(),
            endpoints: HashMap::new(),
            secrets: HashMap::new(),
        }
    }

    pub fn resource_count(&self) -> usize {
        self.listeners.len()
            + self.routes.len()
            + self.clusters.len()
            + self.endpoints.len()
            + self.secrets.len()
    }
}

// ─────────────────────────────────────────────────────────────
// Delta xDS subscription tracking
// ─────────────────────────────────────────────────────────────

/// Per-node, per-resource-type delta xDS state (nonce, acknowledged versions).
#[derive(Debug, Clone, Default)]
pub struct DeltaXdsState {
    /// Resource name → acknowledged version string
    pub acknowledged_versions: HashMap<String, String>,
    /// Last nonce we sent to this node for this resource type
    pub last_nonce: Option<String>,
    /// Resources explicitly subscribed to (empty = wildcard)
    pub subscribed: std::collections::HashSet<String>,
}

impl DeltaXdsState {
    pub fn acknowledge(&mut self, nonce: &str, resources: HashMap<String, String>) {
        if self.last_nonce.as_deref() == Some(nonce) {
            self.acknowledged_versions.extend(resources);
            self.last_nonce = None;
        }
    }

    pub fn new_nonce(&mut self) -> String {
        let nonce = Uuid::new_v4().to_string();
        self.last_nonce = Some(nonce.clone());
        nonce
    }

    pub fn is_subscribed(&self, resource_name: &str) -> bool {
        self.subscribed.is_empty() || self.subscribed.contains(resource_name)
    }
}

// ─────────────────────────────────────────────────────────────
// Node sync status
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSyncStatus {
    pub node_id: String,
    pub last_ack_version: Option<String>,
    pub pending_resources: usize,
    pub last_seen: DateTime<Utc>,
    pub synced: bool,
}

// ─────────────────────────────────────────────────────────────
// XdsManager — manages snapshots per node group
// ─────────────────────────────────────────────────────────────

/// Central xDS manager: stores per-node snapshots and delta subscription state.
#[derive(Clone)]
pub struct XdsManager {
    /// Keyed by node-group (typically cluster name for SotW, node-id for delta)
    snapshots: Arc<RwLock<HashMap<String, XdsSnapshot>>>,
    /// Per-node delta state: node_id → resource_type → DeltaXdsState
    delta_state: Arc<RwLock<HashMap<String, HashMap<XdsResourceType, DeltaXdsState>>>>,
    /// Node registry: node_id → NodeInfo
    nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
    /// Node last-seen / sync status
    sync_status: Arc<RwLock<HashMap<String, NodeSyncStatus>>>,
}

impl Default for XdsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl XdsManager {
    pub fn new() -> Self {
        Self {
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            delta_state: Arc::new(RwLock::new(HashMap::new())),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            sync_status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ─── Snapshot management ─────────────────────────────────

    /// Set a new snapshot for a node group.
    pub fn set_snapshot(&self, node_group: impl Into<String>, snapshot: XdsSnapshot) {
        let key = node_group.into();
        info!(node_group = %key, version = %snapshot.version, "xDS snapshot updated");
        self.snapshots.write().unwrap().insert(key, snapshot);
    }

    /// Get the current snapshot for a node group.
    pub fn get_snapshot(&self, node_group: &str) -> Option<XdsSnapshot> {
        self.snapshots.read().unwrap().get(node_group).cloned()
    }

    /// Get the default snapshot (node_group = "_default").
    pub fn default_snapshot(&self) -> XdsSnapshot {
        self.snapshots
            .read()
            .unwrap()
            .get("_default")
            .cloned()
            .unwrap_or_else(XdsSnapshot::empty)
    }

    /// List all node groups with snapshots.
    pub fn node_groups(&self) -> Vec<String> {
        self.snapshots.read().unwrap().keys().cloned().collect()
    }

    // ─── Node registry ───────────────────────────────────────

    pub fn register_node(&self, node: NodeInfo) {
        let now = Utc::now();
        let node_id = node.id.clone();
        self.nodes.write().unwrap().insert(node_id.clone(), node);
        let mut status = self.sync_status.write().unwrap();
        status
            .entry(node_id.clone())
            .and_modify(|s| s.last_seen = now)
            .or_insert_with(|| NodeSyncStatus {
                node_id: node_id.clone(),
                last_ack_version: None,
                pending_resources: 0,
                last_seen: now,
                synced: false,
            });
        debug!(node = %node_id, "xDS node registered");
    }

    pub fn get_node(&self, node_id: &str) -> Option<NodeInfo> {
        self.nodes.read().unwrap().get(node_id).cloned()
    }

    pub fn list_nodes(&self) -> Vec<NodeInfo> {
        self.nodes.read().unwrap().values().cloned().collect()
    }

    // ─── Sync status ─────────────────────────────────────────

    pub fn mark_ack(&self, node_id: &str, version: &str) {
        let mut status = self.sync_status.write().unwrap();
        if let Some(s) = status.get_mut(node_id) {
            s.last_ack_version = Some(version.to_string());
            s.synced = true;
            s.pending_resources = 0;
            s.last_seen = Utc::now();
        }
    }

    pub fn mark_nack(&self, node_id: &str, error: &str) {
        let mut status = self.sync_status.write().unwrap();
        if let Some(s) = status.get_mut(node_id) {
            s.synced = false;
            s.last_seen = Utc::now();
            debug!(node = %node_id, error = %error, "xDS NACK received");
        }
    }

    pub fn list_sync_status(&self) -> Vec<NodeSyncStatus> {
        self.sync_status.read().unwrap().values().cloned().collect()
    }

    // ─── Delta xDS ───────────────────────────────────────────

    pub fn get_or_create_delta_state(
        &self,
        node_id: &str,
        resource_type: XdsResourceType,
    ) -> DeltaXdsState {
        let map = self.delta_state.read().unwrap();
        map.get(node_id)
            .and_then(|m| m.get(&resource_type))
            .cloned()
            .unwrap_or_default()
    }

    pub fn update_delta_state(
        &self,
        node_id: &str,
        resource_type: XdsResourceType,
        state: DeltaXdsState,
    ) {
        let mut map = self.delta_state.write().unwrap();
        map.entry(node_id.to_string())
            .or_default()
            .insert(resource_type, state);
    }

    /// Compute the delta (added/updated resources) since the node's last ACK.
    pub fn compute_delta(
        &self,
        node_id: &str,
        resource_type: XdsResourceType,
        node_group: &str,
    ) -> DeltaResponse {
        let snapshot = self.get_snapshot(node_group).unwrap_or_else(XdsSnapshot::empty);
        let state = self.get_or_create_delta_state(node_id, resource_type);

        let current_resources: HashMap<String, String> = match resource_type {
            XdsResourceType::Lds => {
                snapshot.listeners.keys().map(|k| (k.clone(), snapshot.version.clone())).collect()
            }
            XdsResourceType::Rds => {
                snapshot.routes.keys().map(|k| (k.clone(), snapshot.version.clone())).collect()
            }
            XdsResourceType::Cds => {
                snapshot.clusters.keys().map(|k| (k.clone(), snapshot.version.clone())).collect()
            }
            XdsResourceType::Eds => {
                snapshot.endpoints.keys().map(|k| (k.clone(), snapshot.version.clone())).collect()
            }
            XdsResourceType::Sds => {
                snapshot.secrets.keys().map(|k| (k.clone(), snapshot.version.clone())).collect()
            }
            _ => HashMap::new(),
        };

        let mut updated = vec![];
        let mut removed = vec![];

        for (name, version) in &current_resources {
            if !state.is_subscribed(name) {
                continue;
            }
            let stale = state
                .acknowledged_versions
                .get(name)
                .map(|v| v != version)
                .unwrap_or(true);
            if stale {
                updated.push(name.clone());
            }
        }
        for name in state.acknowledged_versions.keys() {
            if !current_resources.contains_key(name) {
                removed.push(name.clone());
            }
        }

        DeltaResponse {
            system_version: snapshot.version,
            updated_resources: updated,
            removed_resources: removed,
        }
    }

    // ─── Config validation ────────────────────────────────────

    /// Validate a snapshot for basic consistency.
    pub fn validate_snapshot(snapshot: &XdsSnapshot) -> Vec<ValidationError> {
        let mut errors = vec![];

        // Check that all RDS references in listeners resolve
        for (lname, listener) in &snapshot.listeners {
            for fc in &listener.filter_chains {
                for filter in &fc.filters {
                    if filter.name.contains("http_connection_manager") {
                        if let Some(rds_name) =
                            filter.typed_config.get("rds").and_then(|r| r.get("route_config_name")).and_then(|n| n.as_str())
                        {
                            if !snapshot.routes.contains_key(rds_name) {
                                errors.push(ValidationError {
                                    resource_type: XdsResourceType::Lds,
                                    resource_name: lname.clone(),
                                    message: format!(
                                        "references RDS route config '{rds_name}' which does not exist in snapshot"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Check EDS cluster references
        for (cname, cluster) in &snapshot.clusters {
            if cluster.cluster_type == XdsClusterType::Eds {
                if let Some(eds_config) = &cluster.eds_cluster_config {
                    let service_name =
                        eds_config.service_name.as_deref().unwrap_or(cname.as_str());
                    if !snapshot.endpoints.contains_key(service_name) {
                        errors.push(ValidationError {
                            resource_type: XdsResourceType::Cds,
                            resource_name: cname.clone(),
                            message: format!(
                                "EDS cluster references endpoint '{service_name}' not in snapshot"
                            ),
                        });
                    }
                }
            }
        }

        errors
    }

    // ─── Snapshot builder helpers ─────────────────────────────

    /// Build a default xDS snapshot from CAVE mesh resources.
    pub fn build_snapshot_from_resources(
        virtual_services: &[VirtualService],
        destination_rules: &[crate::models::DestinationRule],
        gateways: &[Gateway],
        service_entries: &[ServiceEntry],
    ) -> XdsSnapshot {
        let mut snapshot = XdsSnapshot::empty();

        // Build clusters from destination rules
        for dr in destination_rules {
            let cluster_name = dr.host.replace('.', "_");
            let cluster = XdsCluster {
                name: cluster_name.clone(),
                cluster_type: XdsClusterType::Eds,
                eds_cluster_config: Some(XdsEdsClusterConfig {
                    eds_config: XdsConfigSource {
                        resource_api_version: "V3".to_string(),
                        api_type: XdsApiType::Ads,
                        cluster_name: None,
                    },
                    service_name: Some(dr.host.clone()),
                }),
                load_assignment: None,
                connect_timeout_ms: 10_000,
                lb_policy: XdsLbPolicy::RoundRobin,
                circuit_breakers: None,
                outlier_detection: None,
                http2_protocol_options: None,
                transport_socket: None,
                upstream_http_protocol_options: None,
            };
            snapshot.clusters.insert(cluster_name, cluster);
        }

        // Build listeners from gateways
        for gw in gateways {
            for server in &gw.servers {
                let listener_name =
                    format!("{}.{}.{}", gw.namespace, gw.name, server.port.number);
                let listener = XdsListener {
                    name: listener_name.clone(),
                    address: SocketAddress {
                        address: "0.0.0.0".to_string(),
                        port_value: server.port.number as u32,
                        protocol: SocketProtocol::Tcp,
                    },
                    filter_chains: vec![],
                    listener_filters: vec![],
                    use_original_dst: false,
                    traffic_direction: TrafficDirection::Inbound,
                    stat_prefix: Some(listener_name.clone()),
                };
                snapshot.listeners.insert(listener_name, listener);
            }
        }

        // Build route configurations from virtual services
        for vs in virtual_services {
            for host in &vs.hosts {
                let rc_name = format!("{}_{}", vs.namespace, host.replace('.', "_"));
                let vhost = XdsVirtualHost {
                    name: host.clone(),
                    domains: vec![host.clone()],
                    routes: vs
                        .http
                        .iter()
                        .enumerate()
                        .map(|(i, route)| {
                            let action = if let Some(dest) = route.route.first() {
                                XdsRouteAction::Route(XdsRouteActionRoute {
                                    cluster_specifier: if route.route.len() == 1 {
                                        XdsClusterSpecifier::Cluster(
                                            dest.destination.host.replace('.', "_"),
                                        )
                                    } else {
                                        XdsClusterSpecifier::WeightedClusters(
                                            route
                                                .route
                                                .iter()
                                                .map(|d| XdsWeightedCluster {
                                                    name: d.destination.host.replace('.', "_"),
                                                    weight: d.weight.unwrap_or(100),
                                                    request_headers_to_add: vec![],
                                                    response_headers_to_add: vec![],
                                                })
                                                .collect(),
                                        )
                                    },
                                    timeout_ms: route.timeout_ms,
                                    retry_policy: route.retries.as_ref().map(|r| XdsRetryPolicy {
                                        retry_on: r.retry_on.join(","),
                                        num_retries: r.attempts,
                                        per_try_timeout_ms: r.per_try_timeout_ms,
                                        retriable_status_codes: vec![],
                                    }),
                                    request_mirror_policies: route
                                        .mirror
                                        .as_ref()
                                        .map(|m| {
                                            vec![XdsRequestMirrorPolicy {
                                                cluster: m.host.replace('.', "_"),
                                                runtime_fraction: route.mirror_percentage,
                                            }]
                                        })
                                        .unwrap_or_default(),
                                    host_rewrite: route
                                        .rewrite
                                        .as_ref()
                                        .and_then(|rw| rw.authority.clone()),
                                    prefix_rewrite: route
                                        .rewrite
                                        .as_ref()
                                        .and_then(|rw| rw.uri.clone()),
                                    hash_policy: vec![],
                                })
                            } else {
                                XdsRouteAction::DirectResponse(XdsDirectResponseAction {
                                    status: 503,
                                    body: Some("no destination".to_string()),
                                })
                            };

                            let path_spec = route
                                .match_rules
                                .first()
                                .and_then(|m| m.uri.as_ref())
                                .map(|uri| match uri {
                                    crate::models::StringMatch::Exact(s) => {
                                        XdsPathSpecifier::Path(s.clone())
                                    }
                                    crate::models::StringMatch::Prefix(s) => {
                                        XdsPathSpecifier::Prefix(s.clone())
                                    }
                                    crate::models::StringMatch::Regex(s) => {
                                        XdsPathSpecifier::SafeRegex(s.clone())
                                    }
                                })
                                .unwrap_or(XdsPathSpecifier::Prefix("/".to_string()));

                            XdsRoute {
                                name: route.name.clone().or_else(|| Some(format!("route-{i}"))),
                                match_rule: XdsRouteMatch {
                                    path_specifier: path_spec,
                                    headers: vec![],
                                    query_parameters: vec![],
                                    case_sensitive: true,
                                },
                                action,
                                request_headers_to_add: vec![],
                                response_headers_to_add: vec![],
                            }
                        })
                        .collect(),
                    cors: None,
                };

                let rc = XdsRouteConfiguration {
                    name: rc_name.clone(),
                    virtual_hosts: vec![vhost],
                    request_headers_to_add: vec![],
                    response_headers_to_add: vec![],
                    request_headers_to_remove: vec![],
                    response_headers_to_remove: vec![],
                };
                snapshot.routes.insert(rc_name, rc);
            }
        }

        snapshot
    }
}

// ─────────────────────────────────────────────────────────────
// Validation types
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub resource_type: XdsResourceType,
    pub resource_name: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DeltaResponse {
    pub system_version: String,
    pub updated_resources: Vec<String>,
    pub removed_resources: Vec<String>,
}
