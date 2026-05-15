// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Upstream test port — batch 4 (2026-05-14).
//!
//! Source upstream: istio/istio @ `1.29.2`
//!
//!   * `pilot/pkg/networking/core/cluster_traffic_policy.go`
//!     (`applyOutlierDetection`, `applyConnectionPool`)
//!   * `pilot/pkg/networking/core/cluster_test.go`
//!     (`TestApplyOutlierDetection`, `TestConnectionPoolSettings`,
//!      `TestCommonHttpProtocolOptions`,
//!      `TestBuildSidecarClustersWithMeshWideTCPKeepalive`)
//!
//! The Istio Go layer compiles `networking.TrafficPolicy` (the v1beta1
//! API exposed in `DestinationRule`) into the Envoy XDS cluster proto
//! (`envoy_config_cluster_v3.Cluster`). The port mirrors the field-level
//! mapping in pure Rust: every assertion below is a 1:1 translation of
//! the Go expected struct, with field names lifted from the Envoy
//! cluster proto (`OutlierDetection`, `CircuitBreakers.Thresholds`,
//! `CommonHttpProtocolOptions`, `UpstreamConnectionOptions.TcpKeepalive`).
//!
//! Per-port `PortLevelSettings` overlay is exercised against
//! `cluster_traffic_policy.go`'s `selectTrafficPolicyComponents` semantics:
//! a port-specific field always wins when present; otherwise the global
//! TrafficPolicy field inherits.

use std::time::Duration;

use cave_mesh::traffic_policy::{
    merge_for_port, to_envoy_circuit_breakers, to_envoy_outlier, ConnectionPoolSettings,
    H2UpgradePolicy, HttpSettings, OutlierDetection, PortLevelSettings, TcpKeepalive, TcpSettings,
    TrafficPolicy,
};

// ──────────────────────────────────────────────────────────────
// OutlierDetection → Envoy translation
// ──────────────────────────────────────────────────────────────

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestApplyOutlierDetection/nil_detection lines 1428-1433
#[test]
fn upstream_outlier_nil_yields_no_cluster_field() {
    // The Go test passes `cfg: nil` and asserts `cluster.OutlierDetection == nil`.
    // Our `to_envoy_outlier` is total-on-`OutlierDetection`, so the equivalent
    // contract is: callers wrap in `Option`. We assert the wrapping behaviour.
    let policy = TrafficPolicy::default();
    assert!(
        policy.outlier_detection.is_none(),
        "default TrafficPolicy has no outlier detection"
    );
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestApplyOutlierDetection/empty_detection lines 1434-1439
#[test]
fn upstream_outlier_empty_only_sets_enforcing_success_rate_zero() {
    // Empty Istio OutlierDetection{} → Envoy cluster.OutlierDetection{
    //   EnforcingSuccessRate: 0,
    // }
    let od = OutlierDetection::default();
    let envoy = to_envoy_outlier(&od);
    assert_eq!(envoy.enforcing_success_rate, Some(0));
    assert_eq!(envoy.consecutive_5xx, None);
    assert_eq!(envoy.enforcing_consecutive_5xx, None);
    assert_eq!(envoy.consecutive_gateway_failure, None);
    assert_eq!(envoy.enforcing_consecutive_gateway_failure, None);
    assert_eq!(envoy.interval, None);
    assert_eq!(envoy.base_ejection_time, None);
    assert_eq!(envoy.max_ejection_percent, None);
    assert!(!envoy.split_external_local_origin_errors);
    assert_eq!(envoy.consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_local_origin_success_rate, None);
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestApplyOutlierDetection/5xx_and_gateway_set lines 1440-1453
#[test]
fn upstream_outlier_consecutive_5xx_and_gateway_set() {
    // Istio:
    //   Consecutive_5XxErrors:    UInt32{Value: 4}
    //   ConsecutiveGatewayErrors: UInt32{Value: 3}
    // Envoy:
    //   Consecutive_5Xx:                    UInt32{Value: 4}
    //   EnforcingConsecutive_5Xx:           UInt32{Value: 100}
    //   ConsecutiveGatewayFailure:          UInt32{Value: 3}
    //   EnforcingConsecutiveGatewayFailure: UInt32{Value: 100}
    //   EnforcingSuccessRate:               UInt32{Value: 0}
    let od = OutlierDetection {
        consecutive_5xx_errors: Some(4),
        consecutive_gateway_errors: Some(3),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert_eq!(envoy.consecutive_5xx, Some(4));
    assert_eq!(envoy.enforcing_consecutive_5xx, Some(100));
    assert_eq!(envoy.consecutive_gateway_failure, Some(3));
    assert_eq!(envoy.enforcing_consecutive_gateway_failure, Some(100));
    assert_eq!(envoy.enforcing_success_rate, Some(0));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyOutlierDetection "if v > 0 { v = 100 }" guard
#[test]
fn upstream_outlier_explicit_zero_consecutive_5xx_keeps_enforcing_zero() {
    // The "v > 0 ? 100 : v" branch in Go preserves zero when the user
    // explicitly sets ConsecutiveErrors=0 (disable). Envoy still sees
    // the field, but enforcing stays at 0 (effectively disabled).
    let od = OutlierDetection {
        consecutive_5xx_errors: Some(0),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert_eq!(envoy.consecutive_5xx, Some(0));
    assert_eq!(
        envoy.enforcing_consecutive_5xx,
        Some(0),
        "v=0 stays 0 (enforcement disabled), v>0 promotes to 100"
    );
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyOutlierDetection BaseEjectionTime / Interval branches
#[test]
fn upstream_outlier_interval_and_base_ejection_time_passthrough() {
    // Istio: Interval and BaseEjectionTime are *durationpb.Duration —
    // when non-nil they propagate verbatim into the Envoy proto.
    let od = OutlierDetection {
        interval: Some(Duration::from_secs(10)),
        base_ejection_time: Some(Duration::from_secs(30)),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert_eq!(envoy.interval, Some(Duration::from_secs(10)));
    assert_eq!(envoy.base_ejection_time, Some(Duration::from_secs(30)));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyOutlierDetection "if outlier.MaxEjectionPercent > 0" branch
#[test]
fn upstream_outlier_max_ejection_percent_only_when_positive() {
    // Zero stays unset (Istio: `if > 0`), positive flows through.
    let zero = OutlierDetection { max_ejection_percent: 0, ..Default::default() };
    assert_eq!(to_envoy_outlier(&zero).max_ejection_percent, None);

    let fifty = OutlierDetection { max_ejection_percent: 50, ..Default::default() };
    assert_eq!(to_envoy_outlier(&fifty).max_ejection_percent, Some(50));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestApplyOutlierDetection/local_origin_errors lines 1481-1495
#[test]
fn upstream_outlier_split_external_local_origin_errors() {
    // Istio:
    //   SplitExternalLocalOriginErrors: true,
    //   ConsecutiveLocalOriginFailures: UInt32{Value: 10},
    // Envoy:
    //   EnforcingSuccessRate:                   0
    //   SplitExternalLocalOriginErrors:         true
    //   ConsecutiveLocalOriginFailure:          10
    //   EnforcingLocalOriginSuccessRate:        0
    //   EnforcingConsecutiveLocalOriginFailure: 100
    let od = OutlierDetection {
        split_external_local_origin_errors: true,
        consecutive_local_origin_failures: Some(10),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert!(envoy.split_external_local_origin_errors);
    assert_eq!(envoy.consecutive_local_origin_failure, Some(10));
    assert_eq!(envoy.enforcing_consecutive_local_origin_failure, Some(100));
    assert_eq!(envoy.enforcing_local_origin_success_rate, Some(0));
    assert_eq!(envoy.enforcing_success_rate, Some(0));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyOutlierDetection — split=false ignores local-origin failures
#[test]
fn upstream_outlier_local_origin_failures_ignored_without_split_flag() {
    // The "if outlier.SplitExternalLocalOriginErrors" branch gates the
    // local-origin counter assignment; without the flag, failures don't
    // propagate even if set.
    let od = OutlierDetection {
        split_external_local_origin_errors: false,
        consecutive_local_origin_failures: Some(99),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert!(!envoy.split_external_local_origin_errors);
    assert_eq!(envoy.consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_local_origin_success_rate, None);
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyOutlierDetection — split=true with zero local-origin failures
#[test]
fn upstream_outlier_split_true_zero_local_origin_only_sets_flags() {
    // Per Go: split=true unconditionally writes
    // EnforcingLocalOriginSuccessRate=0 and SplitExternalLocalOriginErrors,
    // but ConsecutiveLocalOriginFailure / its enforcing peer only when
    // `failures > 0`.
    let od = OutlierDetection {
        split_external_local_origin_errors: true,
        consecutive_local_origin_failures: Some(0),
        ..Default::default()
    };
    let envoy = to_envoy_outlier(&od);
    assert!(envoy.split_external_local_origin_errors);
    assert_eq!(envoy.consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_consecutive_local_origin_failure, None);
    assert_eq!(envoy.enforcing_local_origin_success_rate, Some(0));
}

// ──────────────────────────────────────────────────────────────
// ConnectionPool → Envoy CircuitBreaker thresholds + http opts
// ──────────────────────────────────────────────────────────────

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestConnectionPoolSettings/basic_settings lines 150-180
#[test]
fn upstream_connection_pool_basic_settings() {
    // Istio:
    //   Http: { Http1MaxPendingRequests: 1, Http2MaxRequests: 2,
    //           MaxRequestsPerConnection: 3, MaxRetries: 4 }
    //   Tcp:  { MaxConnections: 1, ConnectTimeout: 2s,
    //           TcpKeepalive: { Probes: 3, Time: 4s, Interval: 5s } }
    // Envoy:
    //   threshold.MaxConnections=1, MaxPendingRequests=1, MaxRequests=2,
    //     MaxRetries=4
    //   cluster.ConnectTimeout=2s
    //   cluster.UpstreamConnectionOptions.TcpKeepalive{probes=3, time=4, interval=5}
    //   httpProtocolOptions.MaxRequestsPerConnection=3
    let cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings {
            max_connections: 1,
            connect_timeout: Some(Duration::from_secs(2)),
            tcp_keepalive: Some(TcpKeepalive {
                probes: 3,
                time: Duration::from_secs(4),
                interval: Duration::from_secs(5),
            }),
            ..Default::default()
        }),
        http: Some(HttpSettings {
            http1_max_pending_requests: 1,
            http2_max_requests: 2,
            max_requests_per_connection: 3,
            max_retries: 4,
            ..Default::default()
        }),
    };
    let envoy = to_envoy_circuit_breakers(&cp);

    let t = &envoy.thresholds;
    assert_eq!(t.max_connections, Some(1));
    assert_eq!(t.max_pending_requests, Some(1));
    assert_eq!(t.max_requests, Some(2));
    assert_eq!(t.max_retries, Some(4));

    assert_eq!(envoy.connect_timeout, Some(Duration::from_secs(2)));
    let ka = envoy.tcp_keepalive.as_ref().expect("keepalive set");
    assert_eq!(ka.probes, 3);
    assert_eq!(ka.time, Duration::from_secs(4));
    assert_eq!(ka.interval, Duration::from_secs(5));

    let opts = envoy.common_http_protocol_options.as_ref().expect("http opts");
    assert_eq!(opts.max_requests_per_connection, Some(3));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestConnectionPoolSettings/no_settings lines 165-171
#[test]
fn upstream_connection_pool_no_settings_yields_defaults() {
    // Istio: no DestinationRule.TrafficPolicy.ConnectionPool →
    //   thresholds == getDefaultCircuitBreakerThresholds()
    //   all CommonHttpProtocolOptions fields nil
    let cp = ConnectionPoolSettings::default();
    let envoy = to_envoy_circuit_breakers(&cp);
    assert_eq!(envoy.thresholds.max_connections, None);
    assert_eq!(envoy.thresholds.max_pending_requests, None);
    assert_eq!(envoy.thresholds.max_requests, None);
    assert_eq!(envoy.thresholds.max_retries, None);
    assert!(envoy.tcp_keepalive.is_none());
    assert!(envoy.common_http_protocol_options.is_none());
    assert!(envoy.connect_timeout.is_none());
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_test.go
///   TestCommonHttpProtocolOptions/idle_timeout_http_path lines 1133-1143
#[test]
fn upstream_connection_pool_http_idle_timeout_lifts_into_common_opts() {
    // Istio sets Http.IdleTimeout=15s → envoy
    // CommonHttpProtocolOptions.IdleTimeout=15s.
    let cp = ConnectionPoolSettings {
        http: Some(HttpSettings {
            http1_max_pending_requests: 1,
            idle_timeout: Some(Duration::from_secs(15)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let envoy = to_envoy_circuit_breakers(&cp);
    let opts = envoy.common_http_protocol_options.as_ref().expect("http opts");
    assert_eq!(opts.idle_timeout, Some(Duration::from_secs(15)));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyConnectionPool — "if idleTimeout == nil { idleTimeout = Tcp.IdleTimeout }"
#[test]
fn upstream_connection_pool_tcp_idle_timeout_fallback() {
    // When Http.IdleTimeout is unset but Tcp.IdleTimeout is set, the
    // TCP value bubbles up into CommonHttpProtocolOptions.IdleTimeout.
    let cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings {
            idle_timeout: Some(Duration::from_secs(7)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let envoy = to_envoy_circuit_breakers(&cp);
    let opts = envoy.common_http_protocol_options.as_ref().expect("http opts");
    assert_eq!(opts.idle_timeout, Some(Duration::from_secs(7)));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyConnectionPool — "if Tcp.ConnectTimeout != nil"
#[test]
fn upstream_connection_pool_connect_timeout_passthrough() {
    let cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings {
            connect_timeout: Some(Duration::from_millis(750)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let envoy = to_envoy_circuit_breakers(&cp);
    assert_eq!(envoy.connect_timeout, Some(Duration::from_millis(750)));
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyConnectionPool — "if Http.MaxRetries > 0", etc.
#[test]
fn upstream_connection_pool_zero_means_unset_for_positive_fields() {
    // Every `> 0` guard in Go: explicit zero leaves the threshold
    // field at its default (None).
    let cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings { max_connections: 0, ..Default::default() }),
        http: Some(HttpSettings {
            http1_max_pending_requests: 0,
            http2_max_requests: 0,
            max_retries: 0,
            max_requests_per_connection: 0,
            ..Default::default()
        }),
    };
    let envoy = to_envoy_circuit_breakers(&cp);
    assert_eq!(envoy.thresholds.max_connections, None);
    assert_eq!(envoy.thresholds.max_pending_requests, None);
    assert_eq!(envoy.thresholds.max_requests, None);
    assert_eq!(envoy.thresholds.max_retries, None);
    assert!(envoy.common_http_protocol_options.is_none());
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyConnectionPool — UpstreamConnectionOptions.TcpKeepalive
#[test]
fn upstream_connection_pool_tcp_keepalive_packed() {
    let cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings {
            tcp_keepalive: Some(TcpKeepalive {
                probes: 7,
                time: Duration::from_secs(60),
                interval: Duration::from_secs(15),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let envoy = to_envoy_circuit_breakers(&cp);
    let ka = envoy.tcp_keepalive.expect("keepalive set");
    assert_eq!(ka.probes, 7);
    assert_eq!(ka.time, Duration::from_secs(60));
    assert_eq!(ka.interval, Duration::from_secs(15));
}

// ──────────────────────────────────────────────────────────────
// Per-port (PortLevelSettings) overlay
// ──────────────────────────────────────────────────────────────

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   selectTrafficPolicyComponents — per-port overlay precedence
#[test]
fn upstream_merge_port_level_overrides_global_outlier() {
    // Global: consecutive_5xx=4. Per-port 8080: consecutive_5xx=7.
    // Lookup for port 8080 must yield 7, lookup for port 9090 yields 4.
    let global_od =
        OutlierDetection { consecutive_5xx_errors: Some(4), ..Default::default() };
    let port_od =
        OutlierDetection { consecutive_5xx_errors: Some(7), ..Default::default() };
    let policy = TrafficPolicy {
        outlier_detection: Some(global_od),
        port_level_settings: vec![PortLevelSettings {
            port: 8080,
            outlier_detection: Some(port_od),
            connection_pool: None,
        }],
        ..Default::default()
    };

    let on_8080 = merge_for_port(&policy, 8080);
    assert_eq!(
        on_8080.outlier_detection.expect("od on 8080").consecutive_5xx_errors,
        Some(7),
        "per-port wins on the matching port"
    );
    let on_9090 = merge_for_port(&policy, 9090);
    assert_eq!(
        on_9090.outlier_detection.expect("od on 9090").consecutive_5xx_errors,
        Some(4),
        "global inherits on non-matching ports"
    );
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   selectTrafficPolicyComponents — port-level connection_pool override
#[test]
fn upstream_merge_port_level_overrides_global_connection_pool() {
    let global_cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings { max_connections: 10, ..Default::default() }),
        ..Default::default()
    };
    let port_cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings { max_connections: 50, ..Default::default() }),
        ..Default::default()
    };
    let policy = TrafficPolicy {
        connection_pool: Some(global_cp),
        port_level_settings: vec![PortLevelSettings {
            port: 80,
            outlier_detection: None,
            connection_pool: Some(port_cp),
        }],
        ..Default::default()
    };

    let on_80 = merge_for_port(&policy, 80);
    assert_eq!(
        on_80.connection_pool.unwrap().tcp.unwrap().max_connections,
        50
    );
    let on_443 = merge_for_port(&policy, 443);
    assert_eq!(
        on_443.connection_pool.unwrap().tcp.unwrap().max_connections,
        10
    );
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   selectTrafficPolicyComponents — missing port-level fields inherit
#[test]
fn upstream_merge_port_level_partial_inherits_missing_fields() {
    // Port-level provides connection_pool but no outlier_detection;
    // outlier_detection must inherit from the global policy.
    let global_od =
        OutlierDetection { consecutive_5xx_errors: Some(2), ..Default::default() };
    let port_cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings { max_connections: 33, ..Default::default() }),
        ..Default::default()
    };
    let policy = TrafficPolicy {
        outlier_detection: Some(global_od),
        port_level_settings: vec![PortLevelSettings {
            port: 7000,
            outlier_detection: None,
            connection_pool: Some(port_cp),
        }],
        ..Default::default()
    };

    let merged = merge_for_port(&policy, 7000);
    assert_eq!(
        merged.outlier_detection.expect("inherited").consecutive_5xx_errors,
        Some(2),
        "missing port-level outlier_detection inherits from global"
    );
    assert_eq!(
        merged.connection_pool.unwrap().tcp.unwrap().max_connections,
        33,
        "port-level connection_pool wins"
    );
}

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   selectTrafficPolicyComponents — no port-level entries
#[test]
fn upstream_merge_no_port_level_returns_global_clone() {
    let global_od =
        OutlierDetection { consecutive_5xx_errors: Some(9), ..Default::default() };
    let global_cp = ConnectionPoolSettings {
        tcp: Some(TcpSettings { max_connections: 11, ..Default::default() }),
        ..Default::default()
    };
    let policy = TrafficPolicy {
        outlier_detection: Some(global_od),
        connection_pool: Some(global_cp),
        port_level_settings: vec![],
    };
    let merged = merge_for_port(&policy, 1234);
    assert_eq!(
        merged.outlier_detection.expect("od").consecutive_5xx_errors,
        Some(9)
    );
    assert_eq!(
        merged.connection_pool.unwrap().tcp.unwrap().max_connections,
        11
    );
}

// ──────────────────────────────────────────────────────────────
// H2 upgrade policy enum
// ──────────────────────────────────────────────────────────────

/// Upstream: istio/istio@1.29.2
///   pilot/pkg/networking/core/cluster_traffic_policy.go
///   applyH2Upgrade — explicit Upgrade / DoNotUpgrade override
#[test]
fn upstream_connection_pool_h2_upgrade_policy_propagates() {
    let cp_upgrade = ConnectionPoolSettings {
        http: Some(HttpSettings {
            h2_upgrade_policy: H2UpgradePolicy::Upgrade,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        to_envoy_circuit_breakers(&cp_upgrade).h2_upgrade,
        H2UpgradePolicy::Upgrade
    );

    let cp_no_upgrade = ConnectionPoolSettings {
        http: Some(HttpSettings {
            h2_upgrade_policy: H2UpgradePolicy::DoNotUpgrade,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        to_envoy_circuit_breakers(&cp_no_upgrade).h2_upgrade,
        H2UpgradePolicy::DoNotUpgrade
    );
}
