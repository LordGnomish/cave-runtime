//! CAVE Service Mesh — full Istio-parity control plane.
//!
//! Modules:
//!   models       — all Istio-equivalent resource types
//!   registry     — service discovery (endpoints, health, locality)
//!   traffic      — VirtualService routing, fault injection, LB
//!   circuit      — circuit breaker (Closed/Open/HalfOpen)
//!   rate_limit   — token-bucket rate limiting
//!   auth         — JWT validation + AuthorizationPolicy engine
//!   mtls         — PeerAuthentication, auto-mTLS, SPIFFE validation
//!   metrics      — Prometheus automatic request metrics
//!   observability — per-service latency histograms, golden signals
//!   xds          — xDS v3 snapshot + delta (LDS/RDS/CDS/EDS/SDS)
//!   spiffe       — SPIFFE/SVID identity, internal CA, cert rotation
//!   telemetry    — Telemetry API (per-workload metrics/logs/tracing)
//!   multicluster — cross-cluster discovery, federation, trust domain
//!   sidecar      — Sidecar, EnvoyFilter, WorkloadGroup managers
//!   store        — persistence (cave-db) for all resource types
//!   routes       — Axum admin REST API (~45 endpoints)
//!   error        — MeshError, MeshResult

pub mod auth;
pub mod circuit;
pub mod error;
pub mod metrics;
pub mod models;
pub mod mtls;
pub mod multicluster;
pub mod observability;
pub mod rate_limit;
pub mod registry;
pub mod routes;
pub mod sidecar;
pub mod spiffe;
pub mod store;
pub mod telemetry;
pub mod traffic;
pub mod xds;

/// Ambient-mode parity batch (ztunnel L4 mTLS, waypoint L7, AuthZ, VS/DR,
/// SPIFFE SVID, telemetry). Pinned to istio/istio v1.29.2.
pub mod ambient;

// Public re-exports most frequently needed by callers.
pub use auth::AuthEngine;
pub use circuit::{BreakerConfig, CircuitBreaker};
pub use error::{MeshError, MeshResult};
pub use metrics::MeshMetrics;
pub use models::*;
pub use mtls::MtlsManager;
pub use multicluster::MultiClusterRegistry;
pub use observability::ObservabilityStore;
pub use rate_limit::RateLimiter;
pub use registry::ServiceRegistry;
pub use sidecar::{EnvoyFilterManager, SidecarManager, WorkloadGroupManager};
pub use spiffe::{CertRotationManager, InternalCa, TrustDomainRegistry};
pub use telemetry::TelemetryManager;
pub use traffic::TrafficManager;
pub use xds::XdsManager;

use axum::Router;
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────
// MeshState — shared state injected into every route handler
// ─────────────────────────────────────────────────────────────

/// Central application state for the CAVE service mesh control plane.
///
/// All fields are cheaply cloneable (`Arc`-backed), so the struct can be
/// wrapped in a single `Arc<MeshState>` and passed to Axum via `.with_state()`.
#[derive(Clone)]
pub struct MeshState {
    // ── Service discovery ──────────────────────────────────
    pub registry: Arc<ServiceRegistry>,

    // ── Traffic management ─────────────────────────────────
    pub traffic: Arc<TrafficManager>,

    // ── Gateway + ServiceEntry (owned by MeshState) ────────
    pub gateways: Arc<RwLock<HashMap<String, Gateway>>>,
    pub service_entries: Arc<RwLock<HashMap<String, ServiceEntry>>>,

    // ── Reliability ────────────────────────────────────────
    pub circuit: Arc<CircuitBreaker>,
    pub rate_limiter: Arc<RateLimiter>,

    // ── Security ───────────────────────────────────────────
    pub mtls: Arc<MtlsManager>,
    pub auth: Arc<AuthEngine>,

    // ── Extended resource managers ─────────────────────────
    pub sidecar_mgr: Arc<SidecarManager>,
    pub envoy_filter_mgr: Arc<EnvoyFilterManager>,
    pub workload_group_mgr: Arc<WorkloadGroupManager>,
    pub telemetry_mgr: Arc<TelemetryManager>,

    // ── xDS control plane ──────────────────────────────────
    pub xds: Arc<XdsManager>,

    // ── Multi-cluster federation ───────────────────────────
    pub multicluster: Arc<MultiClusterRegistry>,

    // ── Observability ──────────────────────────────────────
    pub metrics: Arc<MeshMetrics>,
    pub obs: Arc<ObservabilityStore>,
}

impl Default for MeshState {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ServiceRegistry::new()),
            traffic: Arc::new(TrafficManager::new()),
            gateways: Arc::new(RwLock::new(HashMap::new())),
            service_entries: Arc::new(RwLock::new(HashMap::new())),
            circuit: Arc::new(CircuitBreaker::new()),
            rate_limiter: Arc::new(RateLimiter::new()),
            mtls: Arc::new(MtlsManager::new()),
            auth: Arc::new(AuthEngine::new("cave-mesh-dev-secret")),
            sidecar_mgr: Arc::new(SidecarManager::new()),
            envoy_filter_mgr: Arc::new(EnvoyFilterManager::new()),
            workload_group_mgr: Arc::new(WorkloadGroupManager::new()),
            telemetry_mgr: Arc::new(TelemetryManager::new()),
            xds: Arc::new(XdsManager::new()),
            multicluster: Arc::new(MultiClusterRegistry::new("local")),
            metrics: Arc::new(MeshMetrics::new()),
            obs: Arc::new(ObservabilityStore::new()),
        }
    }

    /// Create the Axum router with all mesh endpoints.
    pub fn router(self: Arc<Self>) -> Router {
        routes::create_router(self)
    }
}

// ─────────────────────────────────────────────────────────────
// Convenience constructor for callers
// ─────────────────────────────────────────────────────────────

/// Build and return the CAVE mesh Axum router.
pub fn router(state: Arc<MeshState>) -> Router {
    routes::create_router(state)
}

// ─────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    // ── Helpers ────────────────────────────────────────────

    fn simple_labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn make_endpoint(addr: &str, port: u16, health: HealthStatus) -> Endpoint {
        Endpoint {
            address: addr.to_string(), port, health, weight: 100,
            labels: HashMap::new(), last_checked: Utc::now(),
            locality: None, network: None,
        }
    }

    fn make_meta(namespace: &str, name: &str) -> ServiceMeta {
        ServiceMeta {
            name: name.to_string(), namespace: namespace.to_string(),
            labels: HashMap::new(), created_at: Utc::now(),
        }
    }

    fn make_vs(name: &str, host: &str, routes: Vec<HttpRoute>) -> VirtualService {
        VirtualService {
            name: name.to_string(), namespace: "default".to_string(),
            hosts: vec![host.to_string()], gateways: vec![],
            http: routes, tcp: vec![], tls: vec![], export_to: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        }
    }

    fn make_req(uri: &str, method: &str) -> IncomingRequest {
        IncomingRequest {
            uri: uri.to_string(),
            method: method.to_string(),
            authority: None,
            headers: HashMap::new(),
            query_params: HashMap::new(),
            source_labels: HashMap::new(),
            source_namespace: None,
            traceparent: None,
            tracestate: None,
            gateway: None,
        }
    }

    // ═══════════════════════════════════════════════════════
    // 1 — ServiceRegistry
    // ═══════════════════════════════════════════════════════

    #[test]
    fn registry_register_and_list() {
        let reg = ServiceRegistry::new();
        let ep = make_endpoint("10.0.0.1", 8080, HealthStatus::Healthy);
        reg.register(make_meta("ns", "svc"), ep);
        let svcs = reg.list_services();
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs[0].name, "svc");
    }

    #[test]
    fn registry_healthy_endpoints_only() {
        let reg = ServiceRegistry::new();
        reg.register(make_meta("ns", "svc"), make_endpoint("10.0.0.1", 8080, HealthStatus::Healthy));
        reg.register(make_meta("ns", "svc"), make_endpoint("10.0.0.2", 8080, HealthStatus::Unhealthy));
        let eps = reg.resolve("ns/svc");
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].address, "10.0.0.1");
    }

    #[test]
    fn registry_health_update() {
        let reg = ServiceRegistry::new();
        reg.register(make_meta("ns", "s"), make_endpoint("1.2.3.4", 80, HealthStatus::Unknown));
        reg.update_health("ns", "s", "1.2.3.4", 80, HealthStatus::Healthy);
        let eps = reg.resolve("ns/s");
        // Unknown is not Unhealthy — both Unknown and Healthy are "healthy" in resolve()
        assert!(eps.len() >= 1);
    }

    #[test]
    fn registry_deregister_endpoint() {
        let reg = ServiceRegistry::new();
        reg.register(make_meta("ns", "svc"), make_endpoint("1.2.3.4", 80, HealthStatus::Healthy));
        assert!(!reg.list_services().is_empty());
        reg.deregister("ns", "svc", "1.2.3.4", 80);
        // Service record still exists but no endpoints
        let eps = reg.resolve("ns/svc");
        assert!(eps.is_empty());
    }

    #[test]
    fn registry_subset_filtering() {
        let reg = ServiceRegistry::new();
        let mut ep1 = make_endpoint("10.0.0.1", 80, HealthStatus::Healthy);
        ep1.labels = simple_labels(&[("version", "v1")]);
        let mut ep2 = make_endpoint("10.0.0.2", 80, HealthStatus::Healthy);
        ep2.labels = simple_labels(&[("version", "v2")]);
        reg.register(make_meta("ns", "svc"), ep1);
        reg.register(make_meta("ns", "svc"), ep2);
        let v1 = reg.resolve_subset("ns/svc", &simple_labels(&[("version", "v1")]));
        assert_eq!(v1.len(), 1);
        assert_eq!(v1[0].address, "10.0.0.1");
    }

    #[test]
    fn registry_locality_filtering() {
        let reg = ServiceRegistry::new();
        let mut ep1 = make_endpoint("10.0.0.1", 80, HealthStatus::Healthy);
        ep1.locality = Some(Locality::new("us-east-1").with_zone("us-east-1a"));
        let mut ep2 = make_endpoint("10.0.0.2", 80, HealthStatus::Healthy);
        ep2.locality = Some(Locality::new("us-west-2").with_zone("us-west-2a"));
        reg.register(make_meta("ns", "svc"), ep1);
        reg.register(make_meta("ns", "svc"), ep2);
        let local = reg.resolve_locality(
            "ns/svc", &Locality::new("us-east-1").with_zone("us-east-1a"));
        assert!(local.iter().any(|e| e.address == "10.0.0.1"));
    }

    // ═══════════════════════════════════════════════════════
    // 2 — TrafficManager
    // ═══════════════════════════════════════════════════════

    fn route_dest(host: &str, weight: u32) -> HttpRouteDestination {
        HttpRouteDestination {
            destination: Destination {
                host: host.to_string(), subset: None, port: None,
            },
            weight: Some(weight),
            headers: None,
        }
    }

    #[test]
    fn traffic_basic_route() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "svc.ns.svc.cluster.local", vec![
            HttpRoute {
                name: None, match_rules: vec![],
                route: vec![route_dest("backend", 100)],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None,
                headers: None, redirect: None,
                direct_response: None, rewrite: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);
        let req = make_req("/api", "GET");
        let decision = tm.resolve_route("svc.ns.svc.cluster.local", &req).unwrap();
        assert_eq!(decision.destination_host, "backend");
    }

    #[test]
    fn traffic_redirect() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "old.svc", vec![
            HttpRoute {
                name: None, match_rules: vec![],
                route: vec![],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None, headers: None,
                redirect: Some(HttpRedirect {
                    uri: Some("/new".to_string()),
                    authority: Some("new.svc".to_string()),
                    redirect_code: 301,
                    port: None, scheme: None,
                }),
                direct_response: None, rewrite: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);
        let req = make_req("/old", "GET");
        let decision = tm.resolve_route("old.svc", &req).unwrap();
        assert!(decision.redirect.is_some());
        assert_eq!(decision.redirect.unwrap().redirect_code, 301);
    }

    #[test]
    fn traffic_rewrite() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "rewrite.svc", vec![
            HttpRoute {
                name: None, match_rules: vec![],
                route: vec![route_dest("backend", 100)],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None, headers: None,
                redirect: None,
                rewrite: Some(HttpRewrite { uri: Some("/v2".to_string()), authority: None }),
                direct_response: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);
        let decision = tm.resolve_route("rewrite.svc", &make_req("/v1", "GET")).unwrap();
        assert!(decision.rewrite.is_some());
    }

    #[test]
    fn traffic_header_match() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "header.svc", vec![
            HttpRoute {
                name: None,
                match_rules: vec![HttpMatchRequest {
                    headers: {
                        let mut m = HashMap::new();
                        m.insert("x-env".to_string(), StringMatch::Exact("canary".to_string()));
                        m
                    },
                    name: None, uri: None, method: None, authority: None,
                    query_params: HashMap::new(), gateways: vec![],
                    source_namespace: None, without_headers: HashMap::new(),
                    port: None, ignore_uri_case: false,
                    source_labels: HashMap::new(),
                }],
                route: vec![route_dest("canary", 100)],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None, headers: None,
                redirect: None, direct_response: None, rewrite: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);

        let mut req = make_req("/", "GET");
        req.headers.insert("x-env".to_string(), "canary".to_string());
        let decision = tm.resolve_route("header.svc", &req).unwrap();
        assert_eq!(decision.destination_host, "canary");
    }

    #[test]
    fn traffic_header_match_no_match() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "hm.svc", vec![
            HttpRoute {
                name: None,
                match_rules: vec![HttpMatchRequest {
                    name: None,
                    headers: {
                        let mut m = HashMap::new();
                        m.insert("x-flag".to_string(), StringMatch::Exact("yes".to_string()));
                        m
                    },
                    uri: None, method: None, authority: None,
                    query_params: HashMap::new(), gateways: vec![],
                    source_namespace: None, without_headers: HashMap::new(),
                    port: None, ignore_uri_case: false, source_labels: HashMap::new(),
                }],
                route: vec![route_dest("special", 100)],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None, headers: None,
                redirect: None, direct_response: None, rewrite: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);
        // No matching header → no decision
        let decision = tm.resolve_route("hm.svc", &make_req("/", "GET"));
        assert!(decision.is_none());
    }

    #[test]
    fn traffic_without_header_excludes() {
        let tm = TrafficManager::new();
        let vs = make_vs("vs", "excl.svc", vec![
            HttpRoute {
                name: None,
                match_rules: vec![HttpMatchRequest {
                    name: None, headers: HashMap::new(), uri: None, method: None, authority: None,
                    query_params: HashMap::new(), gateways: vec![],
                    source_namespace: None,
                    without_headers: {
                        let mut m = HashMap::new();
                        m.insert("x-internal".to_string(),
                            StringMatch::Exact("true".to_string()));
                        m
                    },
                    port: None, ignore_uri_case: false, source_labels: HashMap::new(),
                }],
                route: vec![route_dest("public", 100)],
                timeout_ms: None, retries: None, fault: None,
                mirror: None, mirror_percentage: None, headers: None,
                redirect: None, direct_response: None, rewrite: None, cors_policy: None,
            }
        ]);
        tm.upsert_virtual_service(vs);

        // Request with excluded header should NOT match
        let mut req = make_req("/", "GET");
        req.headers.insert("x-internal".to_string(), "true".to_string());
        let decision = tm.resolve_route("excl.svc", &req);
        assert!(decision.is_none());

        // Request without that header should match
        let decision = tm.resolve_route("excl.svc", &make_req("/", "GET"));
        assert!(decision.is_some());
        assert_eq!(decision.unwrap().destination_host, "public");
    }

    #[test]
    fn traffic_consistent_hash_endpoint_index_stable() {
        let tm = TrafficManager::new();
        let dr = DestinationRule {
            name: "dr".to_string(), namespace: "default".to_string(),
            host: "hash.svc".to_string(),
            traffic_policy: Some(TrafficPolicy {
                load_balancer: Some(LoadBalancerSettings {
                    mode: LoadBalancerMode::ConsistentHash,
                    consistent_hash: Some(ConsistentHashLb {
                        key_type: ConsistentHashKey::HttpHeaderName("x-user-id".to_string()),
                        minimum_ring_size: 1024,
                    }),
                    locality_lb_setting: None,
                    warmup_duration_secs: None,
                }),
                connection_pool: None, outlier_detection: None, tls: None,
                port_level_settings: vec![],
            }),
            subsets: vec![], export_to: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        tm.upsert_destination_rule(dr);
        // Same key → same index (deterministic)
        let idx1 = tm.select_endpoint_index("hash.svc", None, 5, Some("user-42"));
        let idx2 = tm.select_endpoint_index("hash.svc", None, 5, Some("user-42"));
        assert_eq!(idx1, idx2);
        // Different key → potentially different index
        let idx3 = tm.select_endpoint_index("hash.svc", None, 5, Some("user-99"));
        // Just verify it's in range
        assert!(idx3 < 5);
    }

    // ═══════════════════════════════════════════════════════
    // 3 — CircuitBreaker
    // ═══════════════════════════════════════════════════════

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new();
        cb.configure("svc", None, BreakerConfig {
            consecutive_errors: 3, ..BreakerConfig::default()
        });
        assert!(!cb.is_open("svc", None));
        cb.record_failure("svc", None);
        cb.record_failure("svc", None);
        cb.record_failure("svc", None);
        // After 3 consecutive failures circuit should open
        assert!(cb.is_open("svc", None));
    }

    #[test]
    fn circuit_breaker_success_resets_errors() {
        let cb = CircuitBreaker::new();
        cb.configure("svc", None, BreakerConfig {
            consecutive_errors: 5, ..BreakerConfig::default()
        });
        cb.record_failure("svc", None);
        cb.record_failure("svc", None);
        cb.record_success("svc", None); // resets error counter
        // 2 errors < 5 threshold, no opening expected
        assert!(!cb.is_open("svc", None));
    }

    #[test]
    fn circuit_breaker_state_label() {
        let cb = CircuitBreaker::new();
        let label = cb.state_label("new-svc", None);
        assert_eq!(label, "closed");
    }

    // ═══════════════════════════════════════════════════════
    // 4 — RateLimiter
    // ═══════════════════════════════════════════════════════

    #[test]
    fn rate_limiter_allows_within_quota() {
        let rl = RateLimiter::with_policy("svc", 100);
        let decision = rl.check_and_consume("svc");
        assert!(matches!(decision, rate_limit::RateLimitDecision::Allowed));
    }

    #[test]
    fn rate_limiter_no_policy_allows() {
        let rl = RateLimiter::new();
        // No policy → always allowed
        let decision = rl.check_and_consume("unknown-svc");
        assert!(matches!(decision, rate_limit::RateLimitDecision::Allowed));
    }

    #[test]
    fn rate_limiter_with_policy_helper() {
        let rl = RateLimiter::with_policy("api", 50);
        let policies = rl.list_policies();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name, "api");
    }

    // ═══════════════════════════════════════════════════════
    // 5 — mTLS
    // ═══════════════════════════════════════════════════════

    #[test]
    fn mtls_strict_rejects_no_cert() {
        let mgr = MtlsManager::new();
        let pa = PeerAuthentication {
            name: "pa".to_string(), namespace: "ns".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Strict },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert_policy(pa);
        let ctx = TlsContext { peer_principal: None, is_mtls: false, peer_cert_san: vec![] };
        let result = mgr.validate_peer("ns", &HashMap::new(), &ctx, None);
        assert!(result.is_err());
    }

    #[test]
    fn mtls_permissive_allows_no_cert() {
        let mgr = MtlsManager::new();
        let pa = PeerAuthentication {
            name: "pa".to_string(), namespace: "ns".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Permissive },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert_policy(pa);
        let ctx = TlsContext { peer_principal: None, is_mtls: false, peer_cert_san: vec![] };
        assert!(mgr.validate_peer("ns", &HashMap::new(), &ctx, None).is_ok());
    }

    #[test]
    fn mtls_disable_allows_without_cert() {
        let mgr = MtlsManager::new();
        let pa = PeerAuthentication {
            name: "pa".to_string(), namespace: "ns".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Disable },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert_policy(pa);
        let ctx = TlsContext { peer_principal: None, is_mtls: false, peer_cert_san: vec![] };
        assert!(mgr.validate_peer("ns", &HashMap::new(), &ctx, None).is_ok());
    }

    #[test]
    fn mtls_auto_mtls_enables_strict() {
        let mgr = MtlsManager::new();
        mgr.set_auto_mtls(true);
        assert!(mgr.auto_mtls_enabled());
        let mode = mgr.effective_mode("ns", &HashMap::new(), None);
        assert_eq!(mode, MtlsMode::Strict);
    }

    #[test]
    fn mtls_per_port_override() {
        let mgr = MtlsManager::new();
        let mut port_map: HashMap<u16, MtlsConfig> = HashMap::new();
        port_map.insert(8080u16, MtlsConfig { mode: MtlsMode::Disable });
        let pa = PeerAuthentication {
            name: "pa".to_string(), namespace: "ns".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Strict },
            port_level_mtls: port_map,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert_policy(pa);
        let mode = mgr.effective_mode("ns", &HashMap::new(), Some(8080));
        assert_eq!(mode, MtlsMode::Disable);
    }

    // ═══════════════════════════════════════════════════════
    // 6 — AuthEngine
    // ═══════════════════════════════════════════════════════

    fn make_req_ctx(method: &str, path: &str) -> RequestContext {
        RequestContext {
            method: method.to_string(),
            path: path.to_string(),
            host: "svc".to_string(),
            source_principal: None,
            source_namespace: None,
            source_ip: None,
            remote_ip: None,
            port: None,
            jwt_claims: None,
            request_principal: None,
        }
    }

    #[test]
    fn auth_deny_overrides_allow() {
        let engine = AuthEngine::new("secret");
        let deny = AuthorizationPolicy {
            name: "deny-all".to_string(), namespace: "ns".to_string(),
            selector: None, action: AuthzAction::Deny,
            rules: vec![AuthzRule { from: vec![], to: vec![], when: vec![] }],
            provider: None,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        let allow = AuthorizationPolicy {
            name: "allow-all".to_string(), namespace: "ns".to_string(),
            selector: None, action: AuthzAction::Allow,
            rules: vec![AuthzRule { from: vec![], to: vec![], when: vec![] }],
            provider: None,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        engine.upsert_authz_policy(deny);
        engine.upsert_authz_policy(allow);
        let ctx = make_req_ctx("GET", "/api");
        assert!(engine.check_authz("ns", &HashMap::new(), &ctx).is_err());
    }

    #[test]
    fn auth_allow_with_no_deny() {
        let engine = AuthEngine::new("secret");
        let allow = AuthorizationPolicy {
            name: "allow-all".to_string(), namespace: "ns".to_string(),
            selector: None, action: AuthzAction::Allow,
            rules: vec![AuthzRule { from: vec![], to: vec![], when: vec![] }],
            provider: None,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        engine.upsert_authz_policy(allow);
        let ctx = make_req_ctx("GET", "/");
        assert!(engine.check_authz("ns", &HashMap::new(), &ctx).is_ok());
    }

    #[test]
    fn auth_no_policies_default_allow() {
        let engine = AuthEngine::new("secret");
        let ctx = make_req_ctx("GET", "/");
        assert!(engine.check_authz("ns", &HashMap::new(), &ctx).is_ok());
    }

    #[test]
    fn auth_remove_policy() {
        let engine = AuthEngine::new("secret");
        let deny = AuthorizationPolicy {
            name: "deny".to_string(), namespace: "ns".to_string(),
            selector: None, action: AuthzAction::Deny,
            rules: vec![AuthzRule { from: vec![], to: vec![], when: vec![] }],
            provider: None,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        engine.upsert_authz_policy(deny);
        engine.remove_authz_policy("ns", "deny");
        let ctx = make_req_ctx("GET", "/");
        assert!(engine.check_authz("ns", &HashMap::new(), &ctx).is_ok());
    }

    // ═══════════════════════════════════════════════════════
    // 7 — SPIFFE
    // ═══════════════════════════════════════════════════════

    #[test]
    fn spiffe_id_parse_valid() {
        let id = SpiffeId::parse("spiffe://cluster.local/ns/default/sa/my-service").unwrap();
        assert_eq!(id.trust_domain, "cluster.local");
        assert_eq!(id.path, "/ns/default/sa/my-service");
    }

    #[test]
    fn spiffe_id_to_uri_roundtrip() {
        let id = SpiffeId {
            trust_domain: "example.org".to_string(),
            path: "/ns/prod/sa/svc".to_string(),
        };
        let uri = id.to_uri();
        let parsed = SpiffeId::parse(&uri).unwrap();
        assert_eq!(parsed.trust_domain, "example.org");
    }

    #[test]
    fn spiffe_id_for_workload() {
        let id = SpiffeId::for_workload("cluster.local", "prod", "api");
        assert_eq!(id.trust_domain, "cluster.local");
        assert!(id.path.contains("prod"));
        assert!(id.path.contains("api"));
    }

    #[test]
    fn spiffe_id_parse_invalid_scheme() {
        assert!(SpiffeId::parse("https://not-spiffe/path").is_none());
    }

    #[test]
    fn spiffe_id_parse_empty_trust_domain() {
        assert!(SpiffeId::parse("spiffe://").is_none());
    }

    #[test]
    fn internal_ca_issues_svid() {
        let ca = spiffe::InternalCa::new("cave.local").unwrap();
        let svid = ca.issue_svid("default", "my-svc", 1).unwrap();
        assert!(!svid.cert_pem.is_empty());
        assert!(!svid.key_pem.is_empty());
        assert_eq!(svid.spiffe_id.trust_domain, "cave.local");
        assert!(!svid.is_expired());
    }

    #[test]
    fn cert_rotation_manager_stores_and_retrieves() {
        let ca = spiffe::InternalCa::new("cave.local").unwrap();
        let svid = ca.issue_svid("default", "svc", 1).unwrap();
        let spiffe_id = svid.spiffe_id.clone();
        let rm = spiffe::CertRotationManager::new(3600);
        rm.store(svid);
        let retrieved = rm.get(&spiffe_id).unwrap();
        assert_eq!(retrieved.spiffe_id.trust_domain, "cave.local");
    }

    #[test]
    fn cert_rotation_manager_revoke() {
        let ca = spiffe::InternalCa::new("cave.local").unwrap();
        let svid = ca.issue_svid("default", "svc", 1).unwrap();
        let spiffe_id = svid.spiffe_id.clone();
        let rm = spiffe::CertRotationManager::new(3600);
        rm.store(svid);
        rm.revoke(&spiffe_id);
        assert!(rm.get(&spiffe_id).is_none());
    }

    #[test]
    fn trust_domain_registry_trusted() {
        let tdr = spiffe::TrustDomainRegistry::new();
        let td = spiffe::TrustDomain::new("partner.org", "CERT-PEM");
        tdr.register(td);
        let partner_id = SpiffeId { trust_domain: "partner.org".to_string(), path: "/ns/x/sa/y".to_string() };
        let evil_id = SpiffeId { trust_domain: "evil.org".to_string(), path: "/ns/x/sa/y".to_string() };
        assert!(tdr.is_trusted(&partner_id));
        assert!(!tdr.is_trusted(&evil_id));
    }

    #[test]
    fn trust_domain_registry_remove() {
        let tdr = spiffe::TrustDomainRegistry::new();
        tdr.register(spiffe::TrustDomain::new("x.org", "CERT"));
        tdr.remove("x.org");
        let x_id = SpiffeId { trust_domain: "x.org".to_string(), path: "/ns/x/sa/y".to_string() };
        assert!(!tdr.is_trusted(&x_id));
    }

    // ═══════════════════════════════════════════════════════
    // 8 — Sidecar / EnvoyFilter / WorkloadGroup
    // ═══════════════════════════════════════════════════════

    fn make_sidecar(name: &str, ns: &str, selector: Option<HashMap<String, String>>) -> Sidecar {
        Sidecar {
            name: name.to_string(), namespace: ns.to_string(),
            selector,
            ingress: vec![], egress: vec![],
            outbound_traffic_policy: OutboundTrafficPolicy::AllowAny,
            created_at: Utc::now(), updated_at: Utc::now(),
        }
    }

    #[test]
    fn sidecar_effective_workload_specific() {
        let mgr = SidecarManager::new();
        mgr.upsert(make_sidecar("ns-wide", "ns", None));
        mgr.upsert(make_sidecar("workload", "ns",
            Some(simple_labels(&[("app", "frontend")]))));
        let eff = mgr.effective_sidecar("ns",
            &simple_labels(&[("app", "frontend")])).unwrap();
        assert_eq!(eff.name, "workload");
    }

    #[test]
    fn sidecar_namespace_fallback() {
        let mgr = SidecarManager::new();
        mgr.upsert(make_sidecar("ns-wide", "ns", None));
        let eff = mgr.effective_sidecar("ns", &HashMap::new()).unwrap();
        assert_eq!(eff.name, "ns-wide");
    }

    #[test]
    fn sidecar_accessible_hosts_from_egress() {
        let mgr = SidecarManager::new();
        let sc = Sidecar {
            name: "sc".to_string(), namespace: "ns".to_string(),
            selector: None, ingress: vec![],
            egress: vec![IstioEgressListener {
                port: None, bind: None,
                capture_mode: CaptureMode::Default,
                hosts: vec!["ns/payments.svc.cluster.local".to_string()],
            }],
            outbound_traffic_policy: OutboundTrafficPolicy::AllowAny,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert(sc);
        let hosts = mgr.accessible_hosts("ns", &HashMap::new());
        assert!(hosts.iter().any(|h| h.contains("payments")));
    }

    #[test]
    fn sidecar_default_all_hosts_when_none() {
        let mgr = SidecarManager::new();
        let hosts = mgr.accessible_hosts("ns", &HashMap::new());
        assert_eq!(hosts, vec!["*/*".to_string()]);
    }

    #[test]
    fn envoy_filter_priority_ordering() {
        let mgr = EnvoyFilterManager::new();
        let high = EnvoyFilter {
            name: "high".to_string(), namespace: "ns".to_string(),
            selector: None, priority: 1, config_patches: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        let low = EnvoyFilter {
            name: "low".to_string(), namespace: "ns".to_string(),
            selector: None, priority: 100, config_patches: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert(low);
        mgr.upsert(high);
        let filters = mgr.list();
        assert_eq!(filters[0].name, "high");
    }

    #[test]
    fn envoy_filter_workload_selector() {
        let mgr = EnvoyFilterManager::new();
        let matched = EnvoyFilter {
            name: "matched".to_string(), namespace: "ns".to_string(),
            selector: Some(simple_labels(&[("app", "api")])),
            priority: 0, config_patches: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        let unmatched = EnvoyFilter {
            name: "unmatched".to_string(), namespace: "ns".to_string(),
            selector: Some(simple_labels(&[("app", "other")])),
            priority: 0, config_patches: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert(matched);
        mgr.upsert(unmatched);
        let filters = mgr.filters_for_workload("ns", &simple_labels(&[("app", "api")]));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].name, "matched");
    }

    #[test]
    fn envoy_filter_remove() {
        let mgr = EnvoyFilterManager::new();
        mgr.upsert(EnvoyFilter {
            name: "ef".to_string(), namespace: "ns".to_string(),
            selector: None, priority: 0, config_patches: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        });
        mgr.remove("ns", "ef");
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn workload_group_entries_for_group() {
        let mgr = WorkloadGroupManager::new();
        let group = WorkloadGroup {
            name: "vm-group".to_string(), namespace: "ns".to_string(),
            selector: Some(simple_labels(&[("workload-type", "vm")])),
            metadata: WorkloadGroupMetadata::default(),
            template: WorkloadEntryTemplate {
                address: None, labels: HashMap::new(),
                service_account: None, network: None, locality: None,
                weight: 100, ports: HashMap::new(),
            },
            probe: None,
            created_at: Utc::now(), updated_at: Utc::now(),
        };
        mgr.upsert_group(group.clone());

        let vm_entry = WorkloadEntry {
            name: Some("vm-1".to_string()), namespace: Some("ns".to_string()),
            address: "192.168.1.100".to_string(),
            labels: simple_labels(&[("workload-type", "vm")]),
            ports: HashMap::new(), service_account: None, network: None,
            locality: None, weight: 100u32,
            created_at: Some(Utc::now()), updated_at: Some(Utc::now()),
        };
        let other = WorkloadEntry {
            name: Some("other-1".to_string()), namespace: Some("ns".to_string()),
            address: "192.168.1.200".to_string(),
            labels: simple_labels(&[("workload-type", "container")]),
            ports: HashMap::new(), service_account: None, network: None,
            locality: None, weight: 100u32,
            created_at: Some(Utc::now()), updated_at: Some(Utc::now()),
        };
        mgr.upsert_entry(vm_entry);
        mgr.upsert_entry(other);

        let matched = mgr.entries_for_group(&group);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].address, "192.168.1.100");
    }

    // ═══════════════════════════════════════════════════════
    // 9 — Telemetry
    // ═══════════════════════════════════════════════════════

    fn make_telemetry(name: &str, ns: &str, selector: Option<HashMap<String, String>>) -> Telemetry {
        Telemetry {
            name: name.to_string(), namespace: ns.to_string(),
            selector,
            tracing: vec![], metrics: vec![], access_logging: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        }
    }

    #[test]
    fn telemetry_workload_priority_over_namespace() {
        let mgr = TelemetryManager::new();
        mgr.upsert(make_telemetry("ns-tel", "ns", None));
        mgr.upsert(make_telemetry("wl-tel", "ns",
            Some(simple_labels(&[("app", "api")]))));
        let eff = mgr.effective_telemetry("ns",
            &simple_labels(&[("app", "api")])).unwrap();
        assert_eq!(eff.name, "wl-tel");
    }

    #[test]
    fn telemetry_root_namespace_fallback() {
        let mgr = TelemetryManager::new();
        mgr.upsert(make_telemetry("global", "istio-system", None));
        let eff = mgr.effective_telemetry("some-other-ns", &HashMap::new());
        assert!(eff.is_some());
        assert_eq!(eff.unwrap().name, "global");
    }

    #[test]
    fn telemetry_tracing_sampling_rate() {
        let mgr = TelemetryManager::new();
        mgr.upsert(Telemetry {
            name: "t".to_string(), namespace: "ns".to_string(),
            selector: None,
            tracing: vec![Tracing {
                providers: vec![], custom_tags: HashMap::new(),
                disable_span_reporting: None,
                random_sampling_percentage: Some(5.0),
                use_request_id_for_trace_sampling: None,
            }],
            metrics: vec![], access_logging: vec![],
            created_at: Utc::now(), updated_at: Utc::now(),
        });
        let rate = mgr.tracing_sampling_rate("ns", &HashMap::new());
        assert_eq!(rate, Some(5.0));
    }

    #[test]
    fn telemetry_remove() {
        let mgr = TelemetryManager::new();
        mgr.upsert(make_telemetry("t", "ns", None));
        mgr.remove("ns", "t");
        assert!(mgr.list().is_empty());
    }

    // ═══════════════════════════════════════════════════════
    // 10 — Multi-cluster
    // ═══════════════════════════════════════════════════════

    #[test]
    fn multicluster_register_and_list() {
        let reg = MultiClusterRegistry::new("local");
        reg.register_cluster(multicluster::RemoteCluster::new("remote1", "network1", "remote1.local"));
        let clusters = reg.list_clusters();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].name, "remote1");
    }

    #[test]
    fn multicluster_status_update() {
        let reg = MultiClusterRegistry::new("local");
        reg.register_cluster(multicluster::RemoteCluster::new("r1", "net", "r1.local"));
        reg.update_cluster_status("r1", multicluster::RemoteClusterStatus::Connected);
        let c = reg.get_cluster("r1").unwrap();
        assert_eq!(c.status, multicluster::RemoteClusterStatus::Connected);
        assert_eq!(reg.connected_clusters().len(), 1);
    }

    #[test]
    fn multicluster_export_and_visible_services() {
        let reg = MultiClusterRegistry::new("local");
        reg.register_cluster(multicluster::RemoteCluster::new("remote", "net", "remote.local"));
        let svc = multicluster::CrossClusterService {
            name: "payments".to_string(), namespace: "billing".to_string(),
            source_cluster: "remote".to_string(),
            host_fqdn: "payments.billing.svc.cluster.local".to_string(),
            ports: vec![], endpoints: vec![],
            export_to: vec!["*".to_string()],
            registered_at: Utc::now(), updated_at: Utc::now(),
        };
        reg.export_service(svc);
        assert_eq!(reg.visible_services().len(), 1);
    }

    #[test]
    fn multicluster_trust_federation() {
        let reg = MultiClusterRegistry::new("local");
        let fed = multicluster::TrustDomainFederation::new(
            "local", "remote.partner", "FAKE-CA-CERT");
        reg.federate(fed);
        assert!(reg.is_federated("remote.partner"));
        assert!(!reg.is_federated("unknown.org"));
        reg.remove_federation("local", "remote.partner");
        assert!(!reg.is_federated("remote.partner"));
    }

    #[test]
    fn multicluster_remove_cluster_clears_services() {
        let reg = MultiClusterRegistry::new("local");
        reg.register_cluster(multicluster::RemoteCluster::new("r", "net", "r.local"));
        reg.export_service(multicluster::CrossClusterService {
            name: "svc".to_string(), namespace: "ns".to_string(),
            source_cluster: "r".to_string(),
            host_fqdn: "svc.ns.svc".to_string(),
            ports: vec![], endpoints: vec![],
            export_to: vec!["*".to_string()],
            registered_at: Utc::now(), updated_at: Utc::now(),
        });
        assert_eq!(reg.visible_services().len(), 1);
        reg.remove_cluster("r");
        assert_eq!(reg.visible_services().len(), 0);
    }

    #[test]
    fn multicluster_federation_snapshot() {
        let reg = MultiClusterRegistry::new("local");
        reg.register_cluster(multicluster::RemoteCluster::new("r", "net", "r.local"));
        reg.update_cluster_status("r", multicluster::RemoteClusterStatus::Connected);
        let snap = reg.federation_snapshot();
        assert_eq!(snap.total_remote_clusters, 1);
        assert_eq!(snap.connected_clusters, 1);
        assert_eq!(snap.local_cluster, "local");
    }

    // ═══════════════════════════════════════════════════════
    // 11 — xDS
    // ═══════════════════════════════════════════════════════

    #[test]
    fn xds_set_and_get_snapshot() {
        let xds_mgr = XdsManager::new();
        let snap = xds::XdsSnapshot::empty();
        xds_mgr.set_snapshot("_default", snap);
        let got = xds_mgr.get_snapshot("_default");
        assert!(got.is_some());
    }

    #[test]
    fn xds_register_node() {
        let xds_mgr = XdsManager::new();
        let node = xds::NodeInfo {
            id: "sidecar~10.0.0.1~pod.default~default.svc".to_string(),
            cluster: "default".to_string(),
            locality: None,
            metadata: serde_json::Value::Null,
            user_agent_name: None,
            user_agent_version: None,
        };
        xds_mgr.register_node(node.clone());
        let nodes = xds_mgr.list_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node.id);
    }

    #[test]
    fn xds_resource_type_url_roundtrip() {
        use xds::XdsResourceType;
        for rt in [
            XdsResourceType::Lds, XdsResourceType::Rds,
            XdsResourceType::Cds, XdsResourceType::Eds, XdsResourceType::Sds,
        ] {
            let url = rt.type_url();
            let parsed = XdsResourceType::from_type_url(url).unwrap();
            assert_eq!(parsed, rt);
        }
    }

    #[test]
    fn xds_validate_empty_snapshot() {
        let snap = xds::XdsSnapshot::empty();
        let errors = xds::XdsManager::validate_snapshot(&snap);
        assert!(errors.is_empty());
    }

    #[test]
    fn xds_delta_state_nonce() {
        let xds_mgr = XdsManager::new();
        let mut state = xds_mgr.get_or_create_delta_state("node-1", xds::XdsResourceType::Cds);
        let nonce1 = state.new_nonce();
        let nonce2 = state.new_nonce();
        assert_ne!(nonce1, nonce2);
    }

    // ═══════════════════════════════════════════════════════
    // 12 — Observability store
    // ═══════════════════════════════════════════════════════

    #[test]
    fn obs_record_and_golden_signals() {
        let store = ObservabilityStore::new();
        let id = Uuid::new_v4();
        store.record_request(id, 42, true);
        store.record_request(id, 80, true);
        store.record_request(id, 120, false);
        let gs = store.golden_signals(id);
        assert_eq!(gs.traffic_total, 3);
        assert_eq!(gs.error_rate, 1.0 / 3.0);
        assert!(gs.latency_avg_ms > 0.0);
    }

    #[test]
    fn obs_error_rate_calculation() {
        let store = ObservabilityStore::new();
        let id = Uuid::new_v4();
        store.record_request(id, 10, true);
        store.record_request(id, 10, false);
        let rate = store.error_rate(id);
        assert!((rate - 0.5).abs() < 0.01);
    }

    #[test]
    fn obs_latency_histogram() {
        let store = ObservabilityStore::new();
        let id = Uuid::new_v4();
        for latency in [10u64, 50, 100, 200, 500, 1000, 2000] {
            store.record_request(id, latency, true);
        }
        let hist = store.latency_histogram(id).unwrap();
        // 100ms bucket should include the 10, 50, 100ms entries
        assert!(hist.le_100ms >= 3);
        // +Inf should include all 7
        assert_eq!(hist.le_inf, 7);
    }

    #[test]
    fn obs_no_data_returns_zero_error_rate() {
        let store = ObservabilityStore::new();
        let rate = store.error_rate(Uuid::new_v4());
        assert_eq!(rate, 0.0);
    }

    #[test]
    fn obs_all_service_ids() {
        let store = ObservabilityStore::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        store.record_request(id1, 10, true);
        store.record_request(id2, 20, false);
        let ids = store.all_service_ids();
        assert_eq!(ids.len(), 2);
    }

    // ═══════════════════════════════════════════════════════
    // 13 — Prometheus metrics
    // ═══════════════════════════════════════════════════════

    #[test]
    fn metrics_record_and_export() {
        let m = MeshMetrics::new();
        m.record_request("src", "dst", "GET", 200, 1024, 55);
        m.record_request("src", "dst", "GET", 503, 0, 10);
        let output = m.export();
        assert!(output.contains("cave_mesh_requests_total"));
        assert!(output.contains("cave_mesh_errors_total"));
    }

    #[test]
    fn metrics_circuit_trip_counter() {
        let m = MeshMetrics::new();
        m.record_circuit_trip("payments");
        let output = m.export();
        assert!(output.contains("cave_mesh_circuit_trips_total"));
    }

    #[test]
    fn metrics_active_connections_gauge() {
        let m = MeshMetrics::new();
        m.inc_connections("api");
        m.inc_connections("api");
        m.dec_connections("api");
        let output = m.export();
        assert!(output.contains("cave_mesh_active_connections"));
    }

    #[test]
    fn metrics_rate_limited_counter() {
        let m = MeshMetrics::new();
        m.record_rate_limited("throttled-svc");
        let output = m.export();
        assert!(output.contains("cave_mesh_rate_limited_total"));
    }

    #[test]
    fn metrics_fault_injected_counter() {
        let m = MeshMetrics::new();
        m.record_fault_injected("unstable-svc");
        let output = m.export();
        assert!(output.contains("cave_mesh_faults_injected_total"));
    }

    // ═══════════════════════════════════════════════════════
    // 14 — MeshState wiring
    // ═══════════════════════════════════════════════════════

    #[test]
    fn mesh_state_default_constructs() {
        let state = MeshState::new();
        assert_eq!(state.registry.list_services().len(), 0);
        assert_eq!(state.sidecar_mgr.list().len(), 0);
        assert_eq!(state.envoy_filter_mgr.list().len(), 0);
        assert_eq!(state.telemetry_mgr.list().len(), 0);
        assert_eq!(state.multicluster.list_clusters().len(), 0);
        assert_eq!(state.xds.list_nodes().len(), 0);
        assert_eq!(state.workload_group_mgr.list_groups().len(), 0);
    }

    #[test]
    fn mesh_state_clone_shares_state() {
        let state = Arc::new(MeshState::new());
        let state2 = state.clone();
        state.registry.register(
            make_meta("ns", "svc"),
            make_endpoint("1.2.3.4", 80, HealthStatus::Healthy),
        );
        assert_eq!(state2.registry.list_services().len(), 1);
    }

    #[test]
    fn access_log_format_default_json_has_trace_id() {
        let fmt = telemetry::AccessLogFormat::default_json();
        assert_eq!(fmt.format, telemetry::AccessLogFormatType::Json);
        assert!(fmt.fields.iter().any(|f| f.name == "trace_id"));
    }

    #[test]
    fn access_log_format_default_json_has_20_plus_fields() {
        let fmt = telemetry::AccessLogFormat::default_json();
        assert!(fmt.fields.len() >= 20);
    }
}
