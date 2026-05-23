// SPDX-License-Identifier: AGPL-3.0-or-later
//! Domain model — mirrors Kong entities (route/service/upstream/target/consumer/plugin)
//! with Envoy `RouteConfiguration` / `Cluster` / `LbPolicy` cross-references.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol { Http, Https, Http2, Grpc, Grpcs, Ws, Wss, Tcp, Tls, Udp }

impl Protocol {
    pub fn is_tls(self) -> bool { matches!(self, Self::Https | Self::Grpcs | Self::Wss | Self::Tls) }
    pub fn is_stream(self) -> bool { matches!(self, Self::Tcp | Self::Tls | Self::Udp) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpstreamAlgorithm { RoundRobin, LeastConnections, ConsistentHashing, Ewma, Random }
impl Default for UpstreamAlgorithm { fn default() -> Self { Self::RoundRobin } }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashOn { None, Consumer, Ip, Header(String), Cookie(String), Path, QueryArg(String) }
impl Default for HashOn { fn default() -> Self { Self::None } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveHealthCheck {
    pub enabled: bool, pub http_path: String, pub interval_s: u32, pub timeout_s: u32,
    pub healthy_threshold: u32, pub unhealthy_threshold: u32, pub http_statuses: Vec<u16>,
}
impl Default for ActiveHealthCheck {
    fn default() -> Self {
        Self { enabled: false, http_path: "/".into(), interval_s: 5, timeout_s: 1,
            healthy_threshold: 2, unhealthy_threshold: 3, http_statuses: vec![200, 201, 204] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassiveHealthCheck {
    pub enabled: bool, pub healthy_threshold: u32, pub unhealthy_threshold: u32,
    pub http_statuses: Vec<u16>, pub tcp_failures: u32, pub timeouts: u32,
}
impl Default for PassiveHealthCheck {
    fn default() -> Self {
        Self { enabled: true, healthy_threshold: 1, unhealthy_threshold: 5,
            http_statuses: vec![200, 201, 204, 301, 302, 304, 307, 404], tcp_failures: 5, timeouts: 5 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target { pub id: Uuid, pub host: String, pub port: u16, pub weight: u32, pub tags: Vec<String> }
impl Target {
    pub fn new(host: impl Into<String>, port: u16, weight: u32) -> Self {
        Self { id: Uuid::new_v4(), host: host.into(), port, weight, tags: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub id: Uuid, pub name: String, pub algorithm: UpstreamAlgorithm,
    pub hash_on: HashOn, pub hash_fallback: HashOn, pub slots: u32,
    pub healthchecks_active: ActiveHealthCheck, pub healthchecks_passive: PassiveHealthCheck,
    pub targets: Vec<Target>, pub tags: Vec<String>,
}
impl Upstream {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), name: name.into(), algorithm: UpstreamAlgorithm::default(),
            hash_on: HashOn::default(), hash_fallback: HashOn::default(), slots: 10_000,
            healthchecks_active: Default::default(), healthchecks_passive: Default::default(),
            targets: vec![], tags: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub id: Uuid, pub name: String, pub protocol: Protocol, pub host: String, pub port: u16,
    pub path: Option<String>, pub retries: u32, pub connect_timeout_ms: u64,
    pub write_timeout_ms: u64, pub read_timeout_ms: u64,
    pub tls_verify: Option<bool>, pub tls_verify_depth: Option<u32>,
    pub ca_certificates: Vec<Uuid>, pub client_certificate: Option<Uuid>,
    pub tags: Vec<String>, pub enabled: bool,
}
impl Service {
    pub fn new(name: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self { id: Uuid::new_v4(), name: name.into(), protocol: Protocol::Http,
            host: host.into(), port, path: None, retries: 5, connect_timeout_ms: 60_000,
            write_timeout_ms: 60_000, read_timeout_ms: 60_000, tls_verify: None,
            tls_verify_depth: None, ca_certificates: vec![], client_certificate: None,
            tags: vec![], enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderMatch { pub name: String, pub values: Vec<String> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathHandling { V0, V1 }
impl Default for PathHandling { fn default() -> Self { Self::V1 } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid, pub name: String, pub protocols: Vec<Protocol>,
    pub methods: Vec<String>, pub hosts: Vec<String>, pub paths: Vec<String>,
    pub headers: Vec<HeaderMatch>, pub https_redirect_status_code: u16,
    pub regex_priority: i32, pub strip_path: bool, pub preserve_host: bool,
    pub path_handling: PathHandling, pub snis: Vec<String>,
    pub sources: Vec<String>, pub destinations: Vec<String>,
    pub service_id: Option<Uuid>, pub tags: Vec<String>,
}
impl Route {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), name: name.into(),
            protocols: vec![Protocol::Http, Protocol::Https],
            methods: vec![], hosts: vec![], paths: vec![], headers: vec![],
            https_redirect_status_code: 426, regex_priority: 0, strip_path: true,
            preserve_host: false, path_handling: PathHandling::default(),
            snis: vec![], sources: vec![], destinations: vec![],
            service_id: None, tags: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer { pub id: Uuid, pub username: String, pub custom_id: Option<String>, pub tags: Vec<String> }
impl Consumer {
    pub fn new(username: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), username: username.into(), custom_id: None, tags: vec![] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    KeyAuth, Jwt, Oauth2, Mtls, Ldap,
    RateLimiting, ProxyCache, RequestTransformer, ResponseTransformer,
    Cors, BotDetection, IpRestriction, CircuitBreaker, Retry, RequestTermination,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub id: Uuid, pub name: String, pub kind: PluginKind, pub enabled: bool,
    pub config: serde_json::Value,
    pub route_id: Option<Uuid>, pub service_id: Option<Uuid>, pub consumer_id: Option<Uuid>,
    pub tags: Vec<String>,
}
impl Plugin {
    pub fn new(name: impl Into<String>, kind: PluginKind) -> Self {
        Self { id: Uuid::new_v4(), name: name.into(), kind, enabled: true,
            config: serde_json::json!({}), route_id: None, service_id: None,
            consumer_id: None, tags: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GwConfig {
    pub listen_http: String, pub listen_https: String, pub listen_admin: String,
    pub workers: u32, pub access_log_path: Option<String>, pub error_log_path: Option<String>,
    pub trusted_ips: Vec<String>, pub real_ip_header: String,
    pub pqc_hybrid_enabled: bool, pub default_protocol: Protocol,
    pub annotations: HashMap<String, String>,
}
impl Default for GwConfig {
    fn default() -> Self {
        Self { listen_http: "0.0.0.0:8000".into(), listen_https: "0.0.0.0:8443".into(),
            listen_admin: "127.0.0.1:8001".into(),
            workers: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1),
            access_log_path: Some("/dev/stdout".into()),
            error_log_path: Some("/dev/stderr".into()),
            trusted_ips: vec![], real_ip_header: "X-Forwarded-For".into(),
            pqc_hybrid_enabled: false, default_protocol: Protocol::Http,
            annotations: HashMap::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn tls_detection() {
        assert!(Protocol::Https.is_tls()); assert!(Protocol::Grpcs.is_tls());
        assert!(Protocol::Wss.is_tls()); assert!(Protocol::Tls.is_tls());
        assert!(!Protocol::Http.is_tls()); assert!(!Protocol::Http2.is_tls());
    }
    #[test] fn stream_detection() {
        assert!(Protocol::Tcp.is_stream()); assert!(Protocol::Tls.is_stream());
        assert!(Protocol::Udp.is_stream()); assert!(!Protocol::Http.is_stream());
    }
    #[test] fn service_defaults() {
        let s = Service::new("foo", "example.com", 80);
        assert_eq!(s.retries, 5); assert_eq!(s.connect_timeout_ms, 60_000); assert!(s.enabled);
    }
    #[test] fn route_defaults() {
        let r = Route::new("foo");
        assert!(r.strip_path); assert!(!r.preserve_host);
        assert_eq!(r.path_handling, PathHandling::V1); assert_eq!(r.protocols.len(), 2);
    }
    #[test] fn upstream_defaults() {
        let u = Upstream::new("api");
        assert_eq!(u.slots, 10_000); assert_eq!(u.algorithm, UpstreamAlgorithm::RoundRobin);
        assert_eq!(u.hash_on, HashOn::None);
    }
    #[test] fn passive_hc_defaults() {
        let phc = PassiveHealthCheck::default();
        assert!(phc.enabled); assert!(phc.http_statuses.contains(&200));
        assert!(phc.http_statuses.contains(&404)); assert_eq!(phc.unhealthy_threshold, 5);
    }
    #[test] fn config_listen() {
        let c = GwConfig::default();
        assert_eq!(c.listen_http, "0.0.0.0:8000"); assert_eq!(c.listen_admin, "127.0.0.1:8001");
    }
    #[test] fn plugin_kind_serde() {
        let k = PluginKind::KeyAuth;
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"key-auth\"");
        let k2: PluginKind = serde_json::from_str(&s).unwrap();
        assert_eq!(k, k2);
    }
}
