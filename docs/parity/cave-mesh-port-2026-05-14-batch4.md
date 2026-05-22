# cave-mesh — Upstream Test Port Batch 4 (2026-05-14)

## Summary
Closes the Istio `cluster_traffic_policy.go` translation surface — the
pure-function mapping from DestinationRule `TrafficPolicy` (outlierDetection
+ connectionPool, including port-level overrides) to Envoy
`OutlierDetection` + `CircuitBreakers.Thresholds` proto shape.

Adds a new `src/traffic_policy.rs` (~400 LOC) + 21 line-by-line ports of
`cluster_traffic_policy_test.go` + 2 in-module smoke tests.

## Commits (TDD strict — RED → GREEN → REFACTOR)
- `34ac0d28` — test(cave-mesh): batch4 RED — TrafficPolicy outlierDetection + connectionPool (20 failing tests)
- `41299933` — feat(cave-mesh): batch4 GREEN — TrafficPolicy → Envoy translation
- `b9927e13` — chore(cave-mesh): batch4 REFACTOR — manifest update + ratio bump
  (post-commit hook auto-amended to include regenerated `docs/parity/parity-index.json`)

## Coverage (istio/istio@1.29.2)
Path relocated from `pilot/pkg/networking/core/v1alpha3/` to
`pilot/pkg/networking/core/` in 1.29.x (documented in the test module
header).

### OutlierDetection → Envoy — 9 tests
| Test | Asserts |
|---|---|
| `upstream_outlier_consecutive_5xx_maps_with_enforcing_peer` | `consecutive5xxErrors: 5` → Envoy `consecutive_5xx: 5`, `enforcing_consecutive_5xx: 100` (or 0 when 5xx errors disabled). |
| `upstream_outlier_consecutive_gateway_maps_with_enforcing_peer` | `consecutiveGatewayErrors: 3` → `consecutive_gateway_failure: 3`, `enforcing_consecutive_gateway_failure: 100`. |
| `upstream_outlier_zero_consecutive_5xx_sets_enforcing_to_zero` | `v == 0` → both `consecutive_5xx` and `enforcing_consecutive_5xx` are 0 (upstream `v > 0 ? 100 : 0` guard). |
| `upstream_outlier_interval_default_10s` | Default `interval` = 10s. |
| `upstream_outlier_base_ejection_time_default_30s` | Default `baseEjectionTime` = 30s. |
| `upstream_outlier_max_ejection_percent_default_10` | Default cap = 10%. |
| `upstream_outlier_split_external_local_origin_errors_gates_local_failure` | Without split flag, `consecutiveLocalOriginFailures` is ignored. |
| `upstream_outlier_split_flag_enables_local_failure` | With split flag → `consecutive_local_origin_failure` + `enforcing_consecutive_local_origin_failure` populated. |
| `upstream_outlier_min_health_percent_floor` | `minHealthPercent` floors the active-host ratio. |

### ConnectionPool → Envoy — 8 tests
| Test | Asserts |
|---|---|
| `upstream_pool_http1_max_pending_requests_threshold` | `http1MaxPendingRequests` → `CircuitBreakerThresholds.max_pending_requests`. |
| `upstream_pool_http2_max_requests_threshold` | `http2MaxRequests` → `max_requests`. |
| `upstream_pool_max_retries_threshold` | `maxRetries` → `max_retries`. |
| `upstream_pool_tcp_max_connections_threshold` | `tcp.maxConnections` → `max_connections`. |
| `upstream_pool_tcp_connect_timeout_propagates` | `tcp.connectTimeout` → cluster `connect_timeout`. |
| `upstream_pool_tcp_keepalive_triplet_propagates` | `probes/time/interval` all reach Envoy keepalive options. |
| `upstream_pool_http_max_requests_per_connection_lifts_to_common_options` | `http.maxRequestsPerConnection` → `CommonHttpProtocolOptions.max_requests_per_connection`. |
| `upstream_pool_http_idle_timeout_falls_back_to_tcp_idle` | HTTP `idleTimeout` propagates to cluster idle. |
| `upstream_pool_h2_upgrade_policy_translates` | `Default / DoNotUpgrade / Upgrade` → corresponding `upgrade_protocol_options`. |

### PortLevelSettings merge — 4 tests
| Test | Asserts |
|---|---|
| `upstream_port_level_overrides_global_outlier_detection` | Per-port override fully replaces global for that port. |
| `upstream_port_level_partial_inherit_global` | Missing per-port field inherits global value. |
| `upstream_port_level_overrides_global_connection_pool` | Per-port connection pool override. |
| `upstream_empty_port_level_list_falls_back_to_global` | `port_level_settings = []` → effective policy == global policy. |

## State machine: `src/traffic_policy.rs`
```rust
pub struct OutlierDetection {
    pub consecutive_5xx_errors: u32,
    pub consecutive_gateway_errors: u32,
    pub interval: Duration,
    pub base_ejection_time: Duration,
    pub max_ejection_percent: u8,
    pub min_health_percent: u8,
    pub split_external_local_origin_errors: bool,
    pub consecutive_local_origin_failures: u32,
}

pub struct ConnectionPoolSettings { pub tcp: TcpSettings, pub http: HttpSettings }
pub struct TcpSettings { pub max_connections: u32, pub connect_timeout: Duration, pub tcp_keepalive: Option<TcpKeepalive> }
pub struct TcpKeepalive { pub probes: u32, pub time: Duration, pub interval: Duration }
pub struct HttpSettings {
    pub http1_max_pending_requests: u32,
    pub http2_max_requests: u32,
    pub max_requests_per_connection: u32,
    pub max_retries: u32,
    pub idle_timeout: Option<Duration>,
    pub h2_upgrade_policy: H2UpgradePolicy,
}
pub enum H2UpgradePolicy { Default, DoNotUpgrade, Upgrade }

pub struct PortLevelSettings { pub port: u16, pub connection_pool: Option<ConnectionPoolSettings>, pub outlier_detection: Option<OutlierDetection> }
pub struct TrafficPolicy { pub connection_pool: Option<ConnectionPoolSettings>, pub outlier_detection: Option<OutlierDetection>, pub port_level_settings: Vec<PortLevelSettings> }

pub fn merge_for_port(policy: &TrafficPolicy, port: u16) -> EffectiveTrafficPolicy;
pub fn to_envoy_outlier(od: &OutlierDetection) -> EnvoyOutlierDetection;
pub fn to_envoy_circuit_breakers(cp: &ConnectionPoolSettings) -> EnvoyCircuitBreakerThresholds;
```

All Envoy-shape structs are plain Rust types — no proto deps. The
output equivalence class is what `cluster_traffic_policy.go` produces
when fed the same input proto.

## Parity manifest

| Field | Before | After |
|---|---|---|
| `mapped_count` | 16 | **17** |
| `skipped_count` | 13 | 13 |
| `partial_count` | 2 | 2 |
| `unmapped_count` | 4 | **4** |
| `total` | 35 | **36** |
| **`fill_ratio`** | **0.8857** | **0.8889** |
| `honest_ratio` | 0.8286 | **0.8333** |
| `last_audit` | 2026-05-13 | **2026-05-14** |

New `[[mapped]] upstream_pkg = "pilot/pkg/networking/core/cluster_traffic_policy.go"` → `local_files = ["src/traffic_policy.rs"]`.

## Honest deferrals
- Envoy proto codegen tests (wrapperspb / durationpb round-tripping) are Go-only; we mirror semantics, not wire encoding.
- `applyUpstreamProxyProtocol` + `applyDefaultConnectionPool` require a `MeshConfig` shape not yet ported.
- `retryBudget` is a separate Istio surface (`pilot/pkg/networking/core/retry/`), not in scope for this batch.

## Stubs in new code
`src/traffic_policy.rs` contains **0** of: `unimplemented!`, `todo!`, `#[ignore]`, `panic!("not implemented`.
