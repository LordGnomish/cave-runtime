//! Istio `DestinationRule.TrafficPolicy` → Envoy XDS cluster translation.
//!
//! Upstream: `pilot/pkg/networking/core/cluster_traffic_policy.go`
//! (`applyOutlierDetection`, `applyConnectionPool`,
//! `selectTrafficPolicyComponents`) in istio/istio @ 1.29.2.
//!
//! This module is a *pure-function* port — no Envoy proto dependency,
//! no globals, no I/O. Each function takes a configuration struct and
//! returns the materialised view that an XDS publisher would put on the
//! wire. Types here intentionally mirror the Istio v1beta1 API field
//! names (renamed snake_case) so audits map line-for-line with the Go
//! source.
//!
//! The struct layout is normalised separately from `crate::models` —
//! `models::TrafficPolicy` carries the legacy (cave-internal) shape with
//! TLS, load-balancer policies and ms-based timeouts; this module
//! focuses on the *outlier + connection-pool + h2-upgrade* slice that
//! the cluster builder converts to Envoy `OutlierDetection`,
//! `CircuitBreakers.Thresholds`, `UpstreamConnectionOptions.TcpKeepalive`
//! and `CommonHttpProtocolOptions`.
//!
//! Behavioural contract follows the Go source byte-for-byte; every
//! deviation is annotated inline.

use std::time::Duration;

// ──────────────────────────────────────────────────────────────
// Istio-shape input types
// ──────────────────────────────────────────────────────────────

/// Per-destination outlier-detection knobs, mirroring
/// `networking.OutlierDetection` (Istio API v1beta1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutlierDetection {
    /// `consecutive5xxErrors` — Envoy `Consecutive_5xx`.
    pub consecutive_5xx_errors: Option<u32>,
    /// `consecutiveGatewayErrors` — Envoy `ConsecutiveGatewayFailure`.
    pub consecutive_gateway_errors: Option<u32>,
    /// `consecutiveLocalOriginFailures` — only honoured when
    /// `splitExternalLocalOriginErrors` is true.
    pub consecutive_local_origin_failures: Option<u32>,
    /// `interval` — analysis sweep cadence.
    pub interval: Option<Duration>,
    /// `baseEjectionTime` — minimum eject duration.
    pub base_ejection_time: Option<Duration>,
    /// `maxEjectionPercent` (0-100); 0 → unset in Envoy.
    pub max_ejection_percent: u8,
    /// `minHealthPercent` (0-100); `< 0` in Go upstream means *unset*
    /// but the field is a `int32` there; here we model it as `Option`.
    pub min_health_percent: Option<u8>,
    /// `splitExternalLocalOriginErrors` flag — gates the local-origin
    /// path in the Envoy translation.
    pub split_external_local_origin_errors: bool,
}

/// Per-destination connection-pool settings (Istio
/// `networking.ConnectionPoolSettings`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectionPoolSettings {
    pub tcp: Option<TcpSettings>,
    pub http: Option<HttpSettings>,
}

/// TCP-level pool settings (Istio `ConnectionPoolSettings_TCPSettings`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TcpSettings {
    /// `maxConnections` — Envoy `Thresholds.MaxConnections` (only when > 0).
    pub max_connections: u32,
    /// `connectTimeout` — Envoy `cluster.ConnectTimeout`.
    pub connect_timeout: Option<Duration>,
    /// `tcpKeepalive` — Envoy `UpstreamConnectionOptions.TcpKeepalive`.
    pub tcp_keepalive: Option<TcpKeepalive>,
    /// `idleTimeout` — fallback for `CommonHttpProtocolOptions.IdleTimeout`
    /// when no HTTP idle timeout is configured.
    pub idle_timeout: Option<Duration>,
    /// `maxConnectionDuration` — Envoy
    /// `CommonHttpProtocolOptions.MaxConnectionDuration`.
    pub max_connection_duration: Option<Duration>,
}

/// TCP keepalive triplet — Envoy
/// `core.TcpKeepalive{KeepaliveProbes, KeepaliveTime, KeepaliveInterval}`.
#[derive(Debug, Clone, PartialEq)]
pub struct TcpKeepalive {
    pub probes: u32,
    pub time: Duration,
    pub interval: Duration,
}

/// HTTP-level pool settings (Istio `ConnectionPoolSettings_HTTPSettings`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HttpSettings {
    /// `http1MaxPendingRequests` — Envoy `Thresholds.MaxPendingRequests`.
    pub http1_max_pending_requests: u32,
    /// `http2MaxRequests` — Envoy `Thresholds.MaxRequests`.
    pub http2_max_requests: u32,
    /// `maxRequestsPerConnection` — Envoy
    /// `CommonHttpProtocolOptions.MaxRequestsPerConnection`.
    pub max_requests_per_connection: u32,
    /// `maxRetries` — Envoy `Thresholds.MaxRetries`.
    pub max_retries: u32,
    /// `idleTimeout` — Envoy `CommonHttpProtocolOptions.IdleTimeout`.
    pub idle_timeout: Option<Duration>,
    /// `h2UpgradePolicy` — UPGRADE / DO_NOT_UPGRADE / DEFAULT.
    pub h2_upgrade_policy: H2UpgradePolicy,
    /// `useClientProtocol` — flag forwarded verbatim.
    pub use_client_protocol: bool,
}

/// `h2UpgradePolicy` enum (Istio
/// `ConnectionPoolSettings_HTTPSettings_H2UpgradePolicy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum H2UpgradePolicy {
    /// Inherit mesh default.
    #[default]
    Default,
    /// Explicitly refuse to upgrade HTTP/1 → HTTP/2.
    DoNotUpgrade,
    /// Explicitly upgrade HTTP/1 → HTTP/2.
    Upgrade,
}

/// Per-port traffic-policy overlay — overrides matching fields on the
/// global `TrafficPolicy` when the destination port matches `port`.
#[derive(Debug, Clone, PartialEq)]
pub struct PortLevelSettings {
    pub port: u16,
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
}

/// Mesh-scoped traffic policy — Istio `networking.TrafficPolicy`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficPolicy {
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
    pub port_level_settings: Vec<PortLevelSettings>,
}

/// Materialised per-port view produced by [`merge_for_port`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectiveTrafficPolicy {
    pub connection_pool: Option<ConnectionPoolSettings>,
    pub outlier_detection: Option<OutlierDetection>,
}

// ──────────────────────────────────────────────────────────────
// Envoy-shape output types (plain Rust; no proto dep)
// ──────────────────────────────────────────────────────────────

/// Envoy `cluster.OutlierDetection` proto, projected as a plain Rust
/// struct. Field semantics map 1:1 to
/// <https://www.envoyproxy.io/docs/envoy/v1.29.2/api-v3/config/cluster/v3/outlier_detection.proto>.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvoyOutlierDetection {
    pub consecutive_5xx: Option<u32>,
    pub enforcing_consecutive_5xx: Option<u32>,
    pub consecutive_gateway_failure: Option<u32>,
    pub enforcing_consecutive_gateway_failure: Option<u32>,
    pub interval: Option<Duration>,
    pub base_ejection_time: Option<Duration>,
    pub max_ejection_percent: Option<u32>,
    pub enforcing_success_rate: Option<u32>,
    pub split_external_local_origin_errors: bool,
    pub consecutive_local_origin_failure: Option<u32>,
    pub enforcing_consecutive_local_origin_failure: Option<u32>,
    pub enforcing_local_origin_success_rate: Option<u32>,
}

/// Envoy `cluster.CircuitBreakers.Thresholds` plus the sibling cluster
/// fields the Istio builder programs alongside (`ConnectTimeout`,
/// `UpstreamConnectionOptions.TcpKeepalive`, `CommonHttpProtocolOptions`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvoyCircuitBreakerThresholds {
    pub thresholds: ThresholdsView,
    pub connect_timeout: Option<Duration>,
    pub tcp_keepalive: Option<EnvoyTcpKeepalive>,
    pub common_http_protocol_options: Option<CommonHttpProtocolOptions>,
    /// `h2_upgrade_policy` is not on the Envoy CB proto itself but it
    /// is configured on the cluster alongside the pool settings; the
    /// Istio builder reads `Http.H2UpgradePolicy` and conditionally
    /// emits an http2 protocol-options block.
    pub h2_upgrade: H2UpgradePolicy,
}

/// `cluster.CircuitBreakers.Thresholds` row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThresholdsView {
    pub max_connections: Option<u32>,
    pub max_pending_requests: Option<u32>,
    pub max_requests: Option<u32>,
    pub max_retries: Option<u32>,
}

/// `core.TcpKeepalive` view.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvoyTcpKeepalive {
    pub probes: u32,
    pub time: Duration,
    pub interval: Duration,
}

/// `core.HttpProtocolOptions` (the "common" half).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommonHttpProtocolOptions {
    pub idle_timeout: Option<Duration>,
    pub max_requests_per_connection: Option<u32>,
    pub max_connection_duration: Option<Duration>,
}

// ──────────────────────────────────────────────────────────────
// Conversion: OutlierDetection → Envoy
// ──────────────────────────────────────────────────────────────

/// Pure-function port of `applyOutlierDetection`
/// (`pilot/pkg/networking/core/cluster_traffic_policy.go`).
///
/// Branches follow the upstream source exactly:
///   * `EnforcingSuccessRate = 0` unconditionally (Istio disables
///     success-rate gating across the board).
///   * `Consecutive_5XxErrors > 0` → `EnforcingConsecutive_5Xx = 100`;
///     `== 0` keeps enforcing at 0 (effectively disabled).
///   * `ConsecutiveGatewayErrors` → same dance, on
///     `ConsecutiveGatewayFailure` / its enforcing peer.
///   * `Interval` / `BaseEjectionTime` propagate verbatim.
///   * `MaxEjectionPercent > 0` → propagate; zero stays unset.
///   * `SplitExternalLocalOriginErrors == true` →
///     emit `EnforcingLocalOriginSuccessRate = 0` and
///     (if `ConsecutiveLocalOriginFailures > 0`)
///     `ConsecutiveLocalOriginFailure` + enforcing=100.
pub fn to_envoy_outlier(od: &OutlierDetection) -> EnvoyOutlierDetection {
    let mut out = EnvoyOutlierDetection {
        enforcing_success_rate: Some(0),
        ..Default::default()
    };

    if let Some(v) = od.consecutive_5xx_errors {
        out.consecutive_5xx = Some(v);
        out.enforcing_consecutive_5xx = Some(if v > 0 { 100 } else { 0 });
    }
    if let Some(v) = od.consecutive_gateway_errors {
        out.consecutive_gateway_failure = Some(v);
        out.enforcing_consecutive_gateway_failure = Some(if v > 0 { 100 } else { 0 });
    }

    if let Some(i) = od.interval {
        out.interval = Some(i);
    }
    if let Some(b) = od.base_ejection_time {
        out.base_ejection_time = Some(b);
    }
    if od.max_ejection_percent > 0 {
        out.max_ejection_percent = Some(u32::from(od.max_ejection_percent));
    }

    if od.split_external_local_origin_errors {
        out.split_external_local_origin_errors = true;
        if let Some(failures) = od.consecutive_local_origin_failures {
            if failures > 0 {
                out.consecutive_local_origin_failure = Some(failures);
                out.enforcing_consecutive_local_origin_failure = Some(100);
            }
        }
        out.enforcing_local_origin_success_rate = Some(0);
    }

    out
}

// ──────────────────────────────────────────────────────────────
// Conversion: ConnectionPoolSettings → Envoy
// ──────────────────────────────────────────────────────────────

/// Pure-function port of `applyConnectionPool`
/// (`pilot/pkg/networking/core/cluster_traffic_policy.go`).
///
/// Mirrors the Go body branch-for-branch:
///   * HTTP `Http1MaxPendingRequests > 0` → `MaxPendingRequests`.
///   * HTTP `Http2MaxRequests > 0` → `MaxRequests`.
///   * HTTP `MaxRetries > 0` → `MaxRetries`.
///   * HTTP `IdleTimeout` → `CommonHttpProtocolOptions.IdleTimeout`.
///   * HTTP `MaxRequestsPerConnection > 0` → wrapped into
///     `CommonHttpProtocolOptions`.
///   * TCP `ConnectTimeout` → `cluster.ConnectTimeout`.
///   * TCP `MaxConnections > 0` → `MaxConnections`.
///   * TCP `MaxConnectionDuration` → `CommonHttpProtocolOptions`.
///   * TCP `IdleTimeout` is used as fallback when HTTP idle is unset.
///   * `TcpKeepalive` is lifted into `UpstreamConnectionOptions`.
pub fn to_envoy_circuit_breakers(cp: &ConnectionPoolSettings) -> EnvoyCircuitBreakerThresholds {
    let mut envoy = EnvoyCircuitBreakerThresholds::default();
    let mut idle_timeout: Option<Duration> = None;
    let mut max_requests_per_connection: u32 = 0;
    let mut max_connection_duration: Option<Duration> = None;

    if let Some(http) = &cp.http {
        if http.http2_max_requests > 0 {
            envoy.thresholds.max_requests = Some(http.http2_max_requests);
        }
        if http.http1_max_pending_requests > 0 {
            envoy.thresholds.max_pending_requests = Some(http.http1_max_pending_requests);
        }
        if http.max_retries > 0 {
            envoy.thresholds.max_retries = Some(http.max_retries);
        }
        idle_timeout = http.idle_timeout;
        max_requests_per_connection = http.max_requests_per_connection;
        envoy.h2_upgrade = http.h2_upgrade_policy;
    }

    if let Some(tcp) = &cp.tcp {
        if let Some(t) = tcp.connect_timeout {
            envoy.connect_timeout = Some(t);
        }
        if tcp.max_connections > 0 {
            envoy.thresholds.max_connections = Some(tcp.max_connections);
        }
        if let Some(d) = tcp.max_connection_duration {
            max_connection_duration = Some(d);
        }
        if idle_timeout.is_none() {
            idle_timeout = tcp.idle_timeout;
        }
        if let Some(ka) = &tcp.tcp_keepalive {
            envoy.tcp_keepalive = Some(EnvoyTcpKeepalive {
                probes: ka.probes,
                time: ka.time,
                interval: ka.interval,
            });
        }
    }

    // The Go source materialises `CommonHttpProtocolOptions` only when
    // at least one field is set (the `if maxConnectionDuration != nil ||
    // idleTimeout != nil || maxRequestsPerConnection > 0 || …` guard).
    if idle_timeout.is_some()
        || max_requests_per_connection > 0
        || max_connection_duration.is_some()
    {
        envoy.common_http_protocol_options = Some(CommonHttpProtocolOptions {
            idle_timeout,
            max_requests_per_connection: (max_requests_per_connection > 0)
                .then_some(max_requests_per_connection),
            max_connection_duration,
        });
    }

    envoy
}

// ──────────────────────────────────────────────────────────────
// Per-port merge (selectTrafficPolicyComponents semantics)
// ──────────────────────────────────────────────────────────────

/// Overlay `policy.port_level_settings[i]` (where `port_i == port`) on
/// top of the global `policy.{connection_pool,outlier_detection}`.
///
/// Semantics — mirroring `selectTrafficPolicyComponents`:
///
///   * If a `PortLevelSettings` exists for `port` and its
///     `connection_pool` is `Some`, the port-level value wins.
///   * Otherwise the global `connection_pool` is used.
///   * Same rule independently for `outlier_detection`.
///
/// The Go source applies this lookup per-component, so partial
/// port-level overlays only inherit the missing field — exactly the
/// behaviour we exercise in `upstream_merge_port_level_partial_…`.
pub fn merge_for_port(policy: &TrafficPolicy, port: u16) -> EffectiveTrafficPolicy {
    let port_match = policy.port_level_settings.iter().find(|p| p.port == port);

    let connection_pool = port_match
        .and_then(|p| p.connection_pool.clone())
        .or_else(|| policy.connection_pool.clone());
    let outlier_detection = port_match
        .and_then(|p| p.outlier_detection.clone())
        .or_else(|| policy.outlier_detection.clone());

    EffectiveTrafficPolicy { connection_pool, outlier_detection }
}

// ──────────────────────────────────────────────────────────────
// In-tree unit tests (kept small — the upstream-port battery lives
// in `tests/upstream_port_batch4.rs`).
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_envoy_outlier_only_disables_success_rate() {
        let envoy = to_envoy_outlier(&OutlierDetection::default());
        assert_eq!(envoy.enforcing_success_rate, Some(0));
    }

    #[test]
    fn merge_for_port_with_empty_policy_returns_empty() {
        let merged = merge_for_port(&TrafficPolicy::default(), 80);
        assert_eq!(merged, EffectiveTrafficPolicy::default());
    }
}
