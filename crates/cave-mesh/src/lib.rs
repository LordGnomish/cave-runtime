<<<<<<< HEAD
//! CAVE Service Mesh — full Istio-parity control plane.
//!
//! Provides:
//!  * Service discovery & registry
//!  * VirtualService / DestinationRule / Gateway / ServiceEntry
//!  * Traffic splitting (canary, blue-green, A/B)
//!  * Fault injection (delays + aborts)
//!  * Retries and timeouts per route
//!  * Circuit breaking (Closed → Open → HalfOpen)
//!  * mTLS via PeerAuthentication (STRICT / PERMISSIVE / DISABLE)
//!  * JWT validation via RequestAuthentication
//!  * AuthorizationPolicy (ALLOW / DENY rules)
//!  * Rate limiting (token-bucket per service)
//!  * Prometheus metrics
//!  * Distributed tracing (W3C Trace Context propagation)
//!  * Admin API (full CRUD for all resources)
//!  * cave-db integration for persistent storage

pub mod auth;
pub mod circuit;
pub mod error;
pub mod metrics;
pub mod models;
pub mod mtls;
pub mod rate_limit;
pub mod registry;
pub mod routes;
pub mod store;
pub mod traffic;

pub use auth::AuthEngine;
pub use circuit::CircuitBreaker;
pub use error::{MeshError, MeshResult};
pub use metrics::MeshMetrics;
pub use models::*;
pub use mtls::MtlsManager;
pub use rate_limit::RateLimiter;
pub use registry::ServiceRegistry;
pub use traffic::TrafficManager;

use axum::Router;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

pub const MODULE_NAME: &str = "mesh";

// ─────────────────────────────────────────────────────────────
// MeshState
// ─────────────────────────────────────────────────────────────

/// Shared state for the CAVE service mesh.
#[derive(Clone)]
pub struct MeshState {
    pub registry: Arc<ServiceRegistry>,
    pub traffic: Arc<TrafficManager>,
    pub circuit: Arc<CircuitBreaker>,
    pub mtls: Arc<MtlsManager>,
    pub auth: Arc<AuthEngine>,
    pub metrics: Arc<MeshMetrics>,
    pub rate_limiter: Arc<RateLimiter>,
    /// Gateway resources (keyed by "namespace/name")
    pub gateways: Arc<RwLock<HashMap<String, Gateway>>>,
    /// ServiceEntry resources (keyed by "namespace/name")
    pub service_entries: Arc<RwLock<HashMap<String, ServiceEntry>>>,
}

impl MeshState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ServiceRegistry::new()),
            traffic: Arc::new(TrafficManager::new()),
            circuit: Arc::new(CircuitBreaker::new()),
            mtls: Arc::new(MtlsManager::new()),
            auth: Arc::new(AuthEngine::default()),
            metrics: Arc::new(MeshMetrics::new()),
            rate_limiter: Arc::new(RateLimiter::new()),
            gateways: Arc::new(RwLock::new(HashMap::new())),
            service_entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
=======
//! CAVE Mesh — Service mesh replacing Istio + Linkerd.
//!
//! Replaces: Istio, Linkerd
//! Features: mTLS, traffic splitting, canary routing, circuit breaking,
//!           fault injection, traffic mirroring, golden-signal observability.

pub mod models;
pub mod mtls;
pub mod observability;
pub mod proxy;
pub mod routes;
pub mod traffic;

use axum::Router;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// Shared in-memory state for the mesh module.
/// Each collection is independently locked to reduce contention.
pub struct MeshState {
    pub services: Mutex<HashMap<Uuid, models::Service>>,
    pub instances: Mutex<HashMap<Uuid, models::ServiceInstance>>,
    pub virtual_services: Mutex<HashMap<Uuid, models::VirtualService>>,
    pub traffic_policies: Mutex<HashMap<Uuid, models::TrafficPolicy>>,
    pub destination_rules: Mutex<HashMap<Uuid, models::DestinationRule>>,
    pub service_entries: Mutex<HashMap<Uuid, models::ServiceEntry>>,
    pub sidecars: Mutex<HashMap<Uuid, models::SidecarConfig>>,
    pub circuit_breakers: Mutex<HashMap<Uuid, proxy::CircuitBreakerState>>,
    pub metrics: Mutex<HashMap<Uuid, observability::ServiceMetrics>>,
    pub certs: Mutex<HashMap<Uuid, mtls::CertRecord>>,
>>>>>>> claude/peaceful-lederberg
}

impl Default for MeshState {
    fn default() -> Self {
<<<<<<< HEAD
        Self::new()
    }
}

/// Build the axum router for the mesh admin API.
=======
        Self {
            services: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            virtual_services: Mutex::new(HashMap::new()),
            traffic_policies: Mutex::new(HashMap::new()),
            destination_rules: Mutex::new(HashMap::new()),
            service_entries: Mutex::new(HashMap::new()),
            sidecars: Mutex::new(HashMap::new()),
            circuit_breakers: Mutex::new(HashMap::new()),
            metrics: Mutex::new(HashMap::new()),
            certs: Mutex::new(HashMap::new()),
        }
    }
}

/// Create the axum router for the mesh module.
>>>>>>> claude/peaceful-lederberg
pub fn router(state: Arc<MeshState>) -> Router {
    routes::create_router(state)
}

<<<<<<< HEAD
// ─────────────────────────────────────────────────────────────
// Tests (≥ 20 covering every major feature)
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::AuthEngine,
        circuit::{BreakerConfig, CircuitBreaker},
        models::*,
        mtls::MtlsManager,
        rate_limit::RateLimiter,
        registry::ServiceRegistry,
        traffic::TrafficManager,
    };
    use chrono::Utc;
    use std::{collections::HashMap, time::Duration};

    // ── Helpers ──────────────────────────────────────────────

    fn test_service() -> ServiceMeta {
        ServiceMeta {
            name: "reviews".to_string(),
            namespace: "prod".to_string(),
            labels: [("app".to_string(), "reviews".to_string())].into(),
            created_at: Utc::now(),
        }
    }

    fn test_endpoint(addr: &str, healthy: bool) -> Endpoint {
        Endpoint {
            address: addr.to_string(),
            port: 9080,
            health: if healthy {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            },
            weight: 100,
            labels: HashMap::new(),
            last_checked: Utc::now(),
        }
    }

    fn vs_with_exact_uri(host: &str, uri: &str, dest_host: &str) -> VirtualService {
        VirtualService {
            name: "test-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec![host.to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![HttpMatchRequest {
                    uri: Some(StringMatch::Exact(uri.to_string())),
                    ..Default::default()
                }],
                route: vec![HttpRouteDestination {
                    destination: Destination {
                        host: dest_host.to_string(),
                        subset: None,
                        port: None,
                    },
                    weight: Some(100),
                    headers: None,
                }],
                fault: None,
                retries: None,
                timeout_ms: None,
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ════════════════════════════════════════════════════════
    // 1. Service Registry — register and resolve
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_registry_register_and_resolve() {
        let reg = ServiceRegistry::new();
        reg.register(test_service(), test_endpoint("10.0.0.1", true));

        let endpoints = reg.resolve("reviews");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].address, "10.0.0.1");
    }

    // ════════════════════════════════════════════════════════
    // 2. Health update filters unhealthy endpoints
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_registry_health_filter() {
        let reg = ServiceRegistry::new();
        reg.register(test_service(), test_endpoint("10.0.0.1", true));
        reg.register(test_service(), test_endpoint("10.0.0.2", false));

        let healthy = reg.resolve("reviews");
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].address, "10.0.0.1");

        let all = reg.resolve_all("reviews");
        assert_eq!(all.len(), 2);
    }

    // ════════════════════════════════════════════════════════
    // 3. Health status update transitions endpoint correctly
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_registry_health_update() {
        let reg = ServiceRegistry::new();
        reg.register(test_service(), test_endpoint("10.0.0.1", true));

        reg.update_health("prod", "reviews", "10.0.0.1", 9080, HealthStatus::Unhealthy);
        let healthy = reg.resolve("reviews");
        assert!(healthy.is_empty(), "should have no healthy endpoints");
    }

    // ════════════════════════════════════════════════════════
    // 4. Deregister removes endpoint
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_registry_deregister() {
        let reg = ServiceRegistry::new();
        reg.register(test_service(), test_endpoint("10.0.0.1", true));
        reg.register(test_service(), test_endpoint("10.0.0.2", true));

        reg.deregister("prod", "reviews", "10.0.0.1", 9080);
        let endpoints = reg.resolve("reviews");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].address, "10.0.0.2");
    }

    // ════════════════════════════════════════════════════════
    // 5. VirtualService — exact URI match
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_exact_uri_match() {
        let tm = TrafficManager::new();
        tm.upsert_virtual_service(vs_with_exact_uri("reviews", "/v1/product", "reviews-v1"));

        let req = IncomingRequest {
            uri: "/v1/product".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        let decision = tm.resolve_route("reviews", &req);
        assert!(decision.is_some());
        assert_eq!(decision.unwrap().destination_host, "reviews-v1");
    }

    // ════════════════════════════════════════════════════════
    // 6. VirtualService — prefix URI match
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_prefix_uri_match() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "prefix-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["api".to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![HttpMatchRequest {
                    uri: Some(StringMatch::Prefix("/api/v2".to_string())),
                    ..Default::default()
                }],
                route: vec![HttpRouteDestination {
                    destination: Destination {
                        host: "api-v2".to_string(),
                        subset: None,
                        port: None,
                    },
                    weight: Some(100),
                    headers: None,
                }],
                fault: None,
                retries: None,
                timeout_ms: None,
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        let req = IncomingRequest {
            uri: "/api/v2/users".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        let decision = tm.resolve_route("api", &req).unwrap();
        assert_eq!(decision.destination_host, "api-v2");
    }

    // ════════════════════════════════════════════════════════
    // 7. VirtualService — no match returns None
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_no_match() {
        let tm = TrafficManager::new();
        tm.upsert_virtual_service(vs_with_exact_uri("reviews", "/v1/product", "reviews-v1"));

        let req = IncomingRequest {
            uri: "/v2/other".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        assert!(tm.resolve_route("reviews", &req).is_none());
    }

    // ════════════════════════════════════════════════════════
    // 8. A/B routing — header-based canary
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_header_ab_routing() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "ab-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["frontend".to_string()],
            gateways: vec![],
            http: vec![
                // Route users with header "x-canary: true" to canary
                HttpRoute {
                    name: Some("canary".to_string()),
                    match_rules: vec![HttpMatchRequest {
                        headers: [(
                            "x-canary".to_string(),
                            StringMatch::Exact("true".to_string()),
                        )]
                        .into(),
                        ..Default::default()
                    }],
                    route: vec![HttpRouteDestination {
                        destination: Destination {
                            host: "frontend-canary".to_string(),
                            subset: None,
                            port: None,
                        },
                        weight: Some(100),
                        headers: None,
                    }],
                    fault: None,
                    retries: None,
                    timeout_ms: None,
                    mirror: None,
                    headers: None,
                },
                // Everyone else → stable
                HttpRoute {
                    name: Some("stable".to_string()),
                    match_rules: vec![],
                    route: vec![HttpRouteDestination {
                        destination: Destination {
                            host: "frontend-stable".to_string(),
                            subset: None,
                            port: None,
                        },
                        weight: Some(100),
                        headers: None,
                    }],
                    fault: None,
                    retries: None,
                    timeout_ms: None,
                    mirror: None,
                    headers: None,
                },
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        let canary_req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            headers: [("x-canary".to_string(), "true".to_string())].into(),
            ..Default::default()
        };
        let stable_req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };

        let canary_dec = tm.resolve_route("frontend", &canary_req).unwrap();
        assert_eq!(canary_dec.destination_host, "frontend-canary");

        let stable_dec = tm.resolve_route("frontend", &stable_req).unwrap();
        assert_eq!(stable_dec.destination_host, "frontend-stable");
    }

    // ════════════════════════════════════════════════════════
    // 9. Weighted routing (blue-green 50/50) — both destinations reachable
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_weighted_routing() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "bluegreen-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["checkout".to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![],
                route: vec![
                    HttpRouteDestination {
                        destination: Destination {
                            host: "checkout-blue".to_string(),
                            subset: None,
                            port: None,
                        },
                        weight: Some(50),
                        headers: None,
                    },
                    HttpRouteDestination {
                        destination: Destination {
                            host: "checkout-green".to_string(),
                            subset: None,
                            port: None,
                        },
                        weight: Some(50),
                        headers: None,
                    },
                ],
                fault: None,
                retries: None,
                timeout_ms: None,
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        // Run many iterations; both destinations should appear
        let mut blue = 0u32;
        let mut green = 0u32;
        let req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        for _ in 0..200 {
            let dec = tm.resolve_route("checkout", &req).unwrap();
            match dec.destination_host.as_str() {
                "checkout-blue" => blue += 1,
                "checkout-green" => green += 1,
                _ => {}
            }
        }
        assert!(blue > 0, "blue destination never chosen");
        assert!(green > 0, "green destination never chosen");
    }

    // ════════════════════════════════════════════════════════
    // 10. Fault injection — abort 100% of requests
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_fault_abort_100pct() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "fault-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["faulty".to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![],
                route: vec![HttpRouteDestination {
                    destination: Destination {
                        host: "backend".to_string(),
                        subset: None,
                        port: None,
                    },
                    weight: Some(100),
                    headers: None,
                }],
                fault: Some(HttpFaultInjection {
                    delay: None,
                    abort: Some(HttpAbort {
                        percent: 100.0,
                        http_status: 503,
                    }),
                }),
                retries: None,
                timeout_ms: None,
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        let req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        let dec = tm.resolve_route("faulty", &req).unwrap();
        assert!(
            matches!(dec.fault, Some(FaultEffect::Abort(503))),
            "Expected Abort(503), got {:?}",
            dec.fault
        );
    }

    // ════════════════════════════════════════════════════════
    // 11. Fault injection — delay 100% of requests
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_fault_delay_100pct() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "delay-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["slow".to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![],
                route: vec![HttpRouteDestination {
                    destination: Destination {
                        host: "backend".to_string(),
                        subset: None,
                        port: None,
                    },
                    weight: Some(100),
                    headers: None,
                }],
                fault: Some(HttpFaultInjection {
                    delay: Some(FixedDelay {
                        percent: 100.0,
                        fixed_delay_ms: 500,
                    }),
                    abort: None,
                }),
                retries: None,
                timeout_ms: None,
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        let req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        let dec = tm.resolve_route("slow", &req).unwrap();
        assert!(
            matches!(dec.fault, Some(FaultEffect::Delay(500))),
            "Expected Delay(500), got {:?}",
            dec.fault
        );
    }

    // ════════════════════════════════════════════════════════
    // 12. Retry config is propagated to decision
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traffic_retry_config() {
        let tm = TrafficManager::new();
        let vs = VirtualService {
            name: "retry-vs".to_string(),
            namespace: "default".to_string(),
            hosts: vec!["api".to_string()],
            gateways: vec![],
            http: vec![HttpRoute {
                name: None,
                match_rules: vec![],
                route: vec![HttpRouteDestination {
                    destination: Destination {
                        host: "api-backend".to_string(),
                        subset: None,
                        port: None,
                    },
                    weight: Some(100),
                    headers: None,
                }],
                fault: None,
                retries: Some(HttpRetry {
                    attempts: 3,
                    per_try_timeout_ms: Some(2000),
                    retry_on: vec!["5xx".to_string()],
                }),
                timeout_ms: Some(10000),
                mirror: None,
                headers: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_virtual_service(vs);

        let req = IncomingRequest {
            uri: "/".to_string(),
            method: "GET".to_string(),
            ..Default::default()
        };
        let dec = tm.resolve_route("api", &req).unwrap();
        let retry = dec.retry.unwrap();
        assert_eq!(retry.attempts, 3);
        assert_eq!(dec.timeout_ms, Some(10000));
    }

    // ════════════════════════════════════════════════════════
    // 13. Circuit breaker — opens after consecutive errors
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_circuit_opens_after_errors() {
        let cb = CircuitBreaker::new();
        cb.configure(
            "reviews",
            None,
            BreakerConfig {
                consecutive_errors: 3,
                ..Default::default()
            },
        );

        assert!(!cb.is_open("reviews", None));
        cb.record_failure("reviews", None);
        cb.record_failure("reviews", None);
        assert!(!cb.is_open("reviews", None)); // not yet
        cb.record_failure("reviews", None);     // 3rd → open
        assert!(cb.is_open("reviews", None));
    }

    // ════════════════════════════════════════════════════════
    // 14. Circuit breaker — success resets the counter
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_circuit_success_resets_counter() {
        let cb = CircuitBreaker::new();
        cb.configure(
            "payments",
            None,
            BreakerConfig {
                consecutive_errors: 3,
                ..Default::default()
            },
        );

        cb.record_failure("payments", None);
        cb.record_failure("payments", None);
        cb.record_success("payments", None); // resets to 0
        cb.record_failure("payments", None);
        cb.record_failure("payments", None);
        assert!(!cb.is_open("payments", None)); // only 2 consecutive after reset
    }

    // ════════════════════════════════════════════════════════
    // 15. Circuit breaker — transitions to HalfOpen after ejection time
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_circuit_half_open_after_ejection() {
        let cb = CircuitBreaker::new();
        cb.configure(
            "inventory",
            None,
            BreakerConfig {
                consecutive_errors: 1,
                base_ejection_time: Duration::from_millis(1), // very short
                ..Default::default()
            },
        );

        cb.record_failure("inventory", None); // opens
        assert!(cb.is_open("inventory", None));

        // Wait for ejection window to expire
        std::thread::sleep(Duration::from_millis(10));

        // Now should transition to HalfOpen and allow a probe
        assert!(!cb.is_open("inventory", None));
        assert_eq!(cb.state_label("inventory", None), "half_open");
    }

    // ════════════════════════════════════════════════════════
    // 16. Circuit breaker — HalfOpen closes on success
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_circuit_half_open_closes_on_success() {
        let cb = CircuitBreaker::new();
        cb.configure(
            "catalog",
            None,
            BreakerConfig {
                consecutive_errors: 1,
                base_ejection_time: Duration::from_millis(1),
                ..Default::default()
            },
        );

        cb.record_failure("catalog", None);
        std::thread::sleep(Duration::from_millis(10));
        cb.is_open("catalog", None); // triggers HalfOpen
        cb.record_success("catalog", None);
        assert_eq!(cb.state_label("catalog", None), "closed");
    }

    // ════════════════════════════════════════════════════════
    // 17. mTLS — STRICT mode rejects plaintext
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_mtls_strict_rejects_plaintext() {
        let mtls = MtlsManager::new();
        mtls.upsert_policy(PeerAuthentication {
            name: "default".to_string(),
            namespace: "prod".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Strict },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        let ctx = TlsContext { peer_principal: None, is_mtls: false };
        let result = mtls.validate_peer("prod", &HashMap::new(), &ctx);
        assert!(result.is_err());
    }

    // ════════════════════════════════════════════════════════
    // 18. mTLS — STRICT mode accepts mTLS
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_mtls_strict_accepts_mtls() {
        let mtls = MtlsManager::new();
        mtls.upsert_policy(PeerAuthentication {
            name: "default".to_string(),
            namespace: "prod".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Strict },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        let ctx = TlsContext {
            peer_principal: Some("spiffe://cluster.local/ns/prod/sa/reviews".to_string()),
            is_mtls: true,
        };
        assert!(mtls.validate_peer("prod", &HashMap::new(), &ctx).is_ok());
    }

    // ════════════════════════════════════════════════════════
    // 19. mTLS — PERMISSIVE mode accepts plaintext
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_mtls_permissive_accepts_plaintext() {
        let mtls = MtlsManager::new();
        mtls.upsert_policy(PeerAuthentication {
            name: "permissive".to_string(),
            namespace: "staging".to_string(),
            selector: None,
            mtls: MtlsConfig { mode: MtlsMode::Permissive },
            port_level_mtls: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        let ctx = TlsContext { peer_principal: None, is_mtls: false };
        assert!(mtls.validate_peer("staging", &HashMap::new(), &ctx).is_ok());
    }

    // ════════════════════════════════════════════════════════
    // 20. AuthorizationPolicy — ALLOW rule permits matching request
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_authz_allow_rule_permits() {
        let engine = AuthEngine::default();
        engine.upsert_authz_policy(AuthorizationPolicy {
            name: "allow-get".to_string(),
            namespace: "default".to_string(),
            selector: None,
            action: AuthzAction::Allow,
            rules: vec![AuthzRule {
                from: vec![],
                to: vec![Operation {
                    methods: vec!["GET".to_string()],
                    paths: vec!["/api/*".to_string()],
                    ..Default::default()
                }],
                when: vec![],
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        let ctx = RequestContext {
            method: "GET".to_string(),
            path: "/api/users".to_string(),
            host: "api-service".to_string(),
            ..Default::default()
        };
        assert!(engine
            .check_authz("default", &HashMap::new(), &ctx)
            .is_ok());
    }

    // ════════════════════════════════════════════════════════
    // 21. AuthorizationPolicy — DENY rule blocks matching request
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_authz_deny_rule_blocks() {
        let engine = AuthEngine::default();
        engine.upsert_authz_policy(AuthorizationPolicy {
            name: "deny-delete".to_string(),
            namespace: "default".to_string(),
            selector: None,
            action: AuthzAction::Deny,
            rules: vec![AuthzRule {
                from: vec![],
                to: vec![Operation {
                    methods: vec!["DELETE".to_string()],
                    ..Default::default()
                }],
                when: vec![],
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        let ctx = RequestContext {
            method: "DELETE".to_string(),
            path: "/api/users/42".to_string(),
            host: "api-service".to_string(),
            ..Default::default()
        };
        assert!(engine
            .check_authz("default", &HashMap::new(), &ctx)
            .is_err());
    }

    // ════════════════════════════════════════════════════════
    // 22. AuthorizationPolicy — default allow when no policies
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_authz_default_allow_no_policies() {
        let engine = AuthEngine::default();
        let ctx = RequestContext {
            method: "GET".to_string(),
            path: "/health".to_string(),
            host: "svc".to_string(),
            ..Default::default()
        };
        // No policies at all → allow
        assert!(engine
            .check_authz("default", &HashMap::new(), &ctx)
            .is_ok());
    }

    // ════════════════════════════════════════════════════════
    // 23. Rate limiter — allows within limit
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_rate_limit_allows_within_limit() {
        let rl = RateLimiter::with_policy("svc-a", 10); // 10 RPS → bucket capacity 20
        // Should be allowed (bucket starts full)
        let result = rl.check_and_consume("svc-a");
        assert!(matches!(result, crate::rate_limit::RateLimitDecision::Allowed));
    }

    // ════════════════════════════════════════════════════════
    // 24. Rate limiter — blocks after exceeding burst capacity
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_rate_limit_blocks_after_burst() {
        // 1 RPS → capacity = 2 tokens
        let rl = RateLimiter::with_policy("svc-b", 1);

        // Drain all tokens
        let _r1 = rl.check_and_consume("svc-b");
        let _r2 = rl.check_and_consume("svc-b");
        // 3rd should be denied
        let r3 = rl.check_and_consume("svc-b");
        assert!(
            matches!(r3, crate::rate_limit::RateLimitDecision::Denied { .. }),
            "Expected denied on 3rd request"
        );
    }

    // ════════════════════════════════════════════════════════
    // 25. Rate limiter — no policy means always allowed
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_rate_limit_no_policy_allows() {
        let rl = RateLimiter::new();
        let result = rl.check_and_consume("unknown-svc");
        assert!(matches!(result, crate::rate_limit::RateLimitDecision::Allowed));
    }

    // ════════════════════════════════════════════════════════
    // 26. Metrics — request counter increments
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_metrics_request_counter() {
        let m = MeshMetrics::new();
        m.record_request("svc-a", "svc-b", "GET", 200, 1024);
        m.record_request("svc-a", "svc-b", "GET", 200, 512);
        m.record_request("svc-a", "svc-b", "GET", 503, 0);

        let output = m.export();
        assert!(output.contains("cave_mesh_requests_total"));
        assert!(output.contains("cave_mesh_errors_total"));
    }

    // ════════════════════════════════════════════════════════
    // 27. Traceparent propagation — generates new span-id
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traceparent_propagation_preserves_trace_id() {
        let original = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let propagated = traffic::propagate_traceparent(Some(original));

        let orig_parts: Vec<&str> = original.splitn(4, '-').collect();
        let prop_parts: Vec<&str> = propagated.splitn(4, '-').collect();

        assert_eq!(prop_parts[0], "00");
        assert_eq!(prop_parts[1], orig_parts[1], "trace-id must be preserved");
        assert_ne!(
            prop_parts[2], orig_parts[2],
            "parent-id (span-id) must be new"
        );
        assert_eq!(prop_parts[3], "01");
    }

    // ════════════════════════════════════════════════════════
    // 28. Traceparent generation — fresh ID when none incoming
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_traceparent_fresh_generation() {
        let tp = traffic::propagate_traceparent(None);
        let parts: Vec<&str> = tp.splitn(4, '-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert!(!parts[1].is_empty());
        assert!(!parts[2].is_empty());
    }

    // ════════════════════════════════════════════════════════
    // 29. StringMatch — regex matching
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_string_match_regex() {
        let m = StringMatch::Regex(r"/api/v\d+/.*".to_string());
        assert!(m.matches("/api/v1/users"));
        assert!(m.matches("/api/v2/items"));
        assert!(!m.matches("/health"));
    }

    // ════════════════════════════════════════════════════════
    // 30. DestinationRule — round-robin endpoint selection
    // ════════════════════════════════════════════════════════
    #[test]
    fn test_destination_rule_round_robin() {
        let tm = TrafficManager::new();
        let dr = DestinationRule {
            name: "reviews-dr".to_string(),
            namespace: "default".to_string(),
            host: "reviews".to_string(),
            traffic_policy: Some(TrafficPolicy {
                load_balancer: Some(LoadBalancerSettings {
                    mode: LoadBalancerMode::RoundRobin,
                    consistent_hash_header: None,
                }),
                connection_pool: None,
                outlier_detection: None,
            }),
            subsets: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        tm.upsert_destination_rule(dr);

        // With 3 endpoints, round-robin should cycle through 0→1→2→0
        let idx0 = tm.select_endpoint_index("reviews", None, 3);
        let idx1 = tm.select_endpoint_index("reviews", None, 3);
        let idx2 = tm.select_endpoint_index("reviews", None, 3);
        let idx3 = tm.select_endpoint_index("reviews", None, 3); // wraps

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(idx3, 0);
    }
}
=======
pub const MODULE_NAME: &str = "mesh";
>>>>>>> claude/peaceful-lederberg
