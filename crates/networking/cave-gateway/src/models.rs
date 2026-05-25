// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data models — Kong-compatible entities.
//!
//! Services, Routes, Upstreams, Targets, Consumers, Plugins,
//! Certificates, SNIs, Tags — all mirroring the Kong Admin API schema.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Service ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: Uuid,
    pub name: Option<String>,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub path: Option<String>,
    pub retries: u32,
    pub connect_timeout: u64, // ms
    pub write_timeout: u64,   // ms
    pub read_timeout: u64,    // ms
    pub enabled: bool,
    pub tags: Vec<String>,
    pub tls_verify: Option<bool>,
    pub ca_certificates: Vec<Uuid>,
    pub client_certificate: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Service {
    pub fn new(host: String, port: u16, protocol: Protocol) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: None,
            protocol,
            host,
            port,
            path: None,
            retries: 5,
            connect_timeout: 60_000,
            write_timeout: 60_000,
            read_timeout: 60_000,
            enabled: true,
            tags: vec![],
            tls_verify: None,
            ca_certificates: vec![],
            client_certificate: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn upstream_url(&self) -> String {
        let scheme = match self.protocol {
            Protocol::Http => "http",
            Protocol::Https => "https",
            Protocol::Grpc => "http",
            Protocol::Grpcs => "https",
            Protocol::Tcp | Protocol::Tls | Protocol::Udp | Protocol::TlsPassthrough => "tcp",
            Protocol::Ws => "ws",
            Protocol::Wss => "wss",
        };
        let path = self.path.as_deref().unwrap_or("");
        format!("{}://{}:{}{}", scheme, self.host, self.port, path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
    Grpc,
    Grpcs,
    Tcp,
    Tls,
    Udp,
    TlsPassthrough,
    Ws,
    Wss,
}

// ── Route ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub name: Option<String>,
    pub service_id: Option<Uuid>,
    pub protocols: Vec<Protocol>,
    pub methods: Option<Vec<String>>,
    pub hosts: Option<Vec<String>>,
    pub paths: Option<Vec<String>>,
    pub headers: Option<HashMap<String, Vec<String>>>,
    pub https_redirect_status_code: u16,
    pub regex_priority: i64,
    pub strip_path: bool,
    pub preserve_host: bool,
    pub request_buffering: bool,
    pub response_buffering: bool,
    pub path_handling: PathHandling,
    pub snis: Option<Vec<String>>,
    pub sources: Option<Vec<CidrPort>>,
    pub destinations: Option<Vec<CidrPort>>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Route {
    pub fn new(service_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: None,
            service_id: Some(service_id),
            protocols: vec![Protocol::Http, Protocol::Https],
            methods: None,
            hosts: None,
            paths: None,
            headers: None,
            https_redirect_status_code: 426,
            regex_priority: 0,
            strip_path: true,
            preserve_host: false,
            request_buffering: true,
            response_buffering: true,
            path_handling: PathHandling::V0,
            snis: None,
            sources: None,
            destinations: None,
            tags: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PathHandling {
    V0,
    V1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidrPort {
    pub ip: Option<String>,
    pub port: Option<u16>,
}

// ── Upstream ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub id: Uuid,
    pub name: String,
    pub algorithm: LbAlgorithm,
    pub hash_on: HashOn,
    pub hash_fallback: HashOn,
    pub hash_on_header: Option<String>,
    pub hash_fallback_header: Option<String>,
    pub hash_on_cookie: Option<String>,
    pub hash_on_cookie_path: String,
    pub hash_on_query_arg: Option<String>,
    pub hash_on_uri_capture: Option<String>,
    pub slots: u32,
    pub healthchecks: HealthChecks,
    pub tags: Vec<String>,
    pub host_header: Option<String>,
    pub client_certificate: Option<Uuid>,
    pub use_srv_name: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Upstream {
    pub fn new(name: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            algorithm: LbAlgorithm::RoundRobin,
            hash_on: HashOn::None,
            hash_fallback: HashOn::None,
            hash_on_header: None,
            hash_fallback_header: None,
            hash_on_cookie: None,
            hash_on_cookie_path: "/".to_string(),
            hash_on_query_arg: None,
            hash_on_uri_capture: None,
            slots: 10000,
            healthchecks: HealthChecks::default(),
            tags: vec![],
            host_header: None,
            client_certificate: None,
            use_srv_name: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LbAlgorithm {
    RoundRobin,
    ConsistentHashing,
    LeastConnections,
    LatencyAware,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HashOn {
    None,
    Consumer,
    Ip,
    Header,
    Cookie,
    Path,
    QueryArg,
    UriCapture,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthChecks {
    pub active: ActiveHealthCheck,
    pub passive: PassiveHealthCheck,
    pub threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveHealthCheck {
    pub enabled: bool,
    pub interval: u64, // seconds
    pub http_path: String,
    pub https_sni: Option<String>,
    pub https_verify_certificate: bool,
    pub concurrency: u32,
    pub timeout: u64, // seconds
    pub healthy: HealthThreshold,
    pub unhealthy: HealthThreshold,
    pub r#type: HealthCheckType,
}

impl Default for ActiveHealthCheck {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: 0,
            http_path: "/".to_string(),
            https_sni: None,
            https_verify_certificate: true,
            concurrency: 10,
            timeout: 1,
            healthy: HealthThreshold {
                successes: 5,
                failures: 0,
                http_failures: 0,
                tcp_failures: 0,
                timeouts: 0,
                interval: 0,
                http_statuses: vec![200, 302],
            },
            unhealthy: HealthThreshold {
                successes: 0,
                failures: 5,
                http_failures: 5,
                tcp_failures: 0,
                interval: 0,
                http_statuses: vec![429, 404, 500, 501, 502, 503, 504, 505],
                timeouts: 0,
            },
            r#type: HealthCheckType::Http,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassiveHealthCheck {
    pub enabled: bool,
    pub r#type: HealthCheckType,
    pub healthy: HealthThreshold,
    pub unhealthy: HealthThreshold,
}

impl Default for PassiveHealthCheck {
    fn default() -> Self {
        Self {
            enabled: false,
            r#type: HealthCheckType::Http,
            healthy: HealthThreshold {
                successes: 0,
                failures: 0,
                http_failures: 0,
                tcp_failures: 0,
                timeouts: 0,
                interval: 0,
                http_statuses: vec![
                    200, 201, 202, 203, 204, 205, 206, 207, 208, 226, 300, 301, 302, 303, 304, 305,
                    306, 307, 308,
                ],
            },
            unhealthy: HealthThreshold {
                successes: 0,
                failures: 0,
                http_failures: 5,
                tcp_failures: 2,
                interval: 0,
                http_statuses: vec![429, 500, 503],
                timeouts: 5,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthThreshold {
    #[serde(default)]
    pub successes: u32,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub http_failures: u32,
    #[serde(default)]
    pub tcp_failures: u32,
    #[serde(default)]
    pub timeouts: u32,
    pub interval: u64,
    pub http_statuses: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthCheckType {
    Http,
    Https,
    Tcp,
    Grpc,
    Grpcs,
}

// ── Target ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: Uuid,
    pub upstream_id: Uuid,
    pub target: String, // "host:port"
    pub weight: u32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Target {
    pub fn new(upstream_id: Uuid, target: String, weight: u32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            upstream_id,
            target,
            weight,
            tags: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    pub fn host_port(&self) -> (&str, u16) {
        if let Some(pos) = self.target.rfind(':') {
            let host = &self.target[..pos];
            let port = self.target[pos + 1..].parse().unwrap_or(80);
            (host, port)
        } else {
            (self.target.as_str(), 80)
        }
    }
}

// ── Consumer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    pub id: Uuid,
    pub username: Option<String>,
    pub custom_id: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Consumer {
    pub fn new(username: Option<String>, custom_id: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            username,
            custom_id,
            tags: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

// ── Plugin ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub id: Uuid,
    pub name: String,
    pub service_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub consumer_id: Option<Uuid>,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub protocols: Vec<Protocol>,
    pub tags: Vec<String>,
    pub ordering: Option<PluginOrdering>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Plugin {
    pub fn new(name: String, config: serde_json::Value) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            service_id: None,
            route_id: None,
            consumer_id: None,
            enabled: true,
            config,
            protocols: vec![
                Protocol::Http,
                Protocol::Https,
                Protocol::Grpc,
                Protocol::Grpcs,
            ],
            tags: vec![],
            ordering: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginOrdering {
    pub before: Option<HashMap<String, Vec<String>>>,
    pub after: Option<HashMap<String, Vec<String>>>,
}

// ── Certificate ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    pub id: Uuid,
    pub cert: String,             // PEM
    pub key: String,              // PEM (private key)
    pub cert_alt: Option<String>, // ECDSA alt cert
    pub key_alt: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Certificate {
    pub fn new(cert: String, key: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            cert,
            key,
            cert_alt: None,
            key_alt: None,
            tags: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

// ── SNI ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sni {
    pub id: Uuid,
    pub name: String,
    pub certificate_id: Uuid,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Sni {
    pub fn new(name: String, certificate_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            certificate_id,
            tags: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

// ── Consumer credentials ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyAuthCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub key: String,
    pub tags: Vec<String>,
    pub ttl: Option<u64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub algorithm: JwtAlgorithm,
    pub key: String,
    pub rsa_public_key: Option<String>,
    pub secret: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JwtAlgorithm {
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    ES256,
    ES384,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAuthCredential {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub username: String,
    pub password: String, // stored as bcrypt hash
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclGroup {
    pub id: Uuid,
    pub consumer_id: Uuid,
    pub group: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// ── Workspace info ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: Uuid,
    pub name: String,
    pub comment: Option<String>,
    pub config: HashMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Pagination ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagedResponse<T> {
    pub data: Vec<T>,
    pub next: Option<String>,
    pub offset: Option<String>,
}

impl<T> PagedResponse<T> {
    pub fn new(data: Vec<T>) -> Self {
        Self {
            data,
            next: None,
            offset: None,
        }
    }
}

// ── Request/Response helpers ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateService {
    pub name: Option<String>,
    pub protocol: Option<Protocol>,
    pub host: String,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub retries: Option<u32>,
    pub connect_timeout: Option<u64>,
    pub write_timeout: Option<u64>,
    pub read_timeout: Option<u64>,
    pub enabled: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub tls_verify: Option<bool>,
    pub url: Option<String>, // convenience: parse host/port/protocol
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateRoute {
    pub name: Option<String>,
    pub service: Option<EntityRef>,
    pub protocols: Option<Vec<Protocol>>,
    pub methods: Option<Vec<String>>,
    pub hosts: Option<Vec<String>>,
    pub paths: Option<Vec<String>>,
    pub headers: Option<HashMap<String, Vec<String>>>,
    pub regex_priority: Option<i64>,
    pub strip_path: Option<bool>,
    pub preserve_host: Option<bool>,
    pub request_buffering: Option<bool>,
    pub response_buffering: Option<bool>,
    pub path_handling: Option<PathHandling>,
    pub snis: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: Option<Uuid>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateUpstream {
    pub name: String,
    pub algorithm: Option<LbAlgorithm>,
    pub hash_on: Option<HashOn>,
    pub slots: Option<u32>,
    pub healthchecks: Option<HealthChecks>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateTarget {
    pub target: String,
    pub weight: Option<u32>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateConsumer {
    pub username: Option<String>,
    pub custom_id: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlugin {
    pub name: String,
    pub service: Option<EntityRef>,
    pub route: Option<EntityRef>,
    pub consumer: Option<EntityRef>,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
    pub protocols: Option<Vec<Protocol>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCertificate {
    pub cert: String,
    pub key: String,
    pub cert_alt: Option<String>,
    pub key_alt: Option<String>,
    pub snis: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSni {
    pub name: String,
    pub certificate: EntityRef,
    pub tags: Option<Vec<String>>,
}
