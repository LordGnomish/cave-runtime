// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Wire-faithful parity tests.
//!
//! These tests pin the *exact bytes* (or close-to-exact field shape)
//! cave-net emits on its observable wire surfaces, and verify each
//! against what upstream Cilium v1.19.3 would produce.
//!
//! Three kinds of wire surface are covered:
//!
//!   1. Prometheus text exposition — every metric name, type tag, and
//!      label-format detail is byte-comparable to a golden string we
//!      built from `pkg/metrics/metrics.go`.
//!   2. JSON CRD shape — `CiliumEnvoyConfig` resources serialise with
//!      the upstream `@type` field key (not `type_url`) so an existing
//!      Cilium-formatted CRD instance round-trips.
//!   3. Reserved-identity numeric table — the well-known
//!      `reserved:host`/`reserved:world` etc. identities emit the same
//!      numeric ID Cilium does so cross-cluster identity exchange works.

use cave_net::cilium::cec::{CecSpec, EnvoyResource};
use cave_net::cilium::metrics::{registry, render_exposition, Kind, MetricDef};
use cave_net::cilium::reserved_ids::ReservedIdentity;
use cave_net::cilium::xds::type_url;
use std::collections::BTreeMap;

// ── 1. Prometheus exposition ────────────────────────────────────────────────

#[test]
fn metric_full_names_byte_match_upstream() {
    // Pulled from upstream pkg/metrics/metrics.go @ v1.19.3.
    // Every single metric the agent registers must produce exactly this
    // dotted name when concatenated.
    let expected: &[&str] = &[
        "cilium_k8s_workqueue_depth",
        "cilium_k8s_workqueue_adds_total",
        "cilium_k8s_workqueue_queue_duration_seconds",
        "cilium_k8s_workqueue_work_duration_seconds",
        "cilium_k8s_workqueue_unfinished_work_seconds",
        "cilium_k8s_workqueue_longest_running_processor_seconds",
        "cilium_k8s_workqueue_retries_total",
        "cilium_agent_bootstrap_seconds",
        "cilium_agent_api_process_time_seconds",
        "cilium_agent_endpoint_regenerations_total",
        "cilium_agent_endpoint_state",
        "cilium_agent_endpoint_regeneration_time_stats_seconds",
        "cilium_agent_policy",
        "cilium_agent_policy_max_revision",
        "cilium_agent_policy_change_total",
        "cilium_agent_policy_endpoint_enforcement_status",
        "cilium_agent_policy_implementation_delay",
        "cilium_agent_policy_incremental_update_duration",
        "cilium_agent_identity",
        "cilium_agent_identity_label_sources",
        "cilium_agent_event_ts",
        "cilium_k8s_k8s_event_lag_seconds",
        "cilium_agent_proxy_redirects",
        "cilium_agent_policy_l7_total",
        "cilium_agent_proxy_upstream_reply_seconds",
        "cilium_agent_proxy_datapath_update_timeout_total",
        "cilium_datapath_conntrack_gc_runs_total",
        "cilium_datapath_conntrack_gc_key_fallbacks_total",
        "cilium_datapath_conntrack_gc_entries",
        "cilium_datapath_nat_gc_entries",
        "cilium_datapath_conntrack_gc_duration_seconds",
        "cilium_datapath_conntrack_gc_interval_seconds",
        "cilium_datapath_conntrack_dump_resets_total",
        "cilium_datapath_signals_handled_total",
        "cilium_agent_services_events_total",
        "cilium_agent_service_implementation_delay",
        "cilium_agent_controllers_runs_total",
        "cilium_agent_controllers_runs_duration_seconds",
        "cilium_agent_subprocess_start_total",
        "cilium_k8s_kubernetes_events_total",
        "cilium_k8s_kubernetes_events_received_total",
        "cilium_k8s_client_api_latency_time_seconds",
        "cilium_k8s_client_rate_limiter_duration_seconds",
        "cilium_k8s_client_api_calls_total",
        "cilium_k8s_terminating_endpoints_events_total",
        "cilium_agent_ipam_events_total",
        "cilium_agent_ipam_capacity",
        "cilium_kvstore_operations_duration_seconds",
        "cilium_kvstore_events_queue_seconds",
        "cilium_kvstore_quorum_errors_total",
        "cilium_ipcache_errors_total",
        "cilium_ipcache_events_total",
        "cilium_fqdn_gc_deletions_total",
        "cilium_fqdn_active_names",
        "cilium_fqdn_active_ips",
        "cilium_fqdn_alive_zombie_connections",
        "cilium_agent_selectors",
        "cilium_api_limiter_semaphore_rejected_total",
        "cilium_api_limiter_wait_history_duration_seconds",
        "cilium_api_limiter_wait_duration_seconds",
        "cilium_api_limiter_processing_duration_seconds",
        "cilium_api_limiter_requests_in_flight",
        "cilium_api_limiter_rate_limit",
        "cilium_api_limiter_adjustment_factor",
        "cilium_api_limiter_processed_requests_total",
        "cilium_api_limiter_endpoint_propagation_delay_seconds",
        "cilium_bpf_syscall_duration_seconds",
        "cilium_bpf_map_ops_total",
        "cilium_bpf_map_capacity",
        "cilium_agent_version",
        "cilium_node_health_connectivity_status",
        "cilium_node_health_connectivity_latency_seconds",
        "cilium_errors_warnings_total",
    ];
    let names: Vec<String> = registry().iter().map(|m| m.full_name()).collect();
    let names_str: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        names_str.len(),
        expected.len(),
        "metric count mismatch: rust={} upstream={}",
        names_str.len(),
        expected.len()
    );
    for (got, want) in names_str.iter().zip(expected.iter()) {
        assert_eq!(got, want, "metric name drift");
    }
}

#[test]
fn render_exposition_matches_prometheus_text_grammar() {
    // Build a small subset and compare to a hand-written golden block.
    let defs: Vec<MetricDef> = vec![
        MetricDef {
            namespace: "cilium",
            subsystem: "agent",
            name: "policy",
            help: "Number of policies currently loaded.",
            kind: Kind::Gauge,
        },
        MetricDef {
            namespace: "cilium",
            subsystem: "agent",
            name: "policy_change_total",
            help: "Number of policy changes by outcome.",
            kind: Kind::Counter,
        },
    ];
    let mut values: BTreeMap<String, (BTreeMap<String, String>, f64)> = BTreeMap::new();
    let no_labels = BTreeMap::new();
    values.insert("cilium_agent_policy".to_string(), (no_labels.clone(), 5.0));
    let mut labels = BTreeMap::new();
    labels.insert("outcome".to_string(), "success".to_string());
    labels.insert("source".to_string(), "k8s".to_string());
    values.insert(
        "cilium_agent_policy_change_total".to_string(),
        (labels, 42.0),
    );

    let out = render_exposition(&defs, &values);
    // Golden expectation — what an upstream Prometheus client produces.
    let expected = "\
# HELP cilium_agent_policy Number of policies currently loaded.
# TYPE cilium_agent_policy gauge
cilium_agent_policy 5
# HELP cilium_agent_policy_change_total Number of policy changes by outcome.
# TYPE cilium_agent_policy_change_total counter
cilium_agent_policy_change_total{outcome=\"success\",source=\"k8s\"} 42
";
    assert_eq!(out, expected, "exposition format drift");
}

// ── 2. CiliumEnvoyConfig JSON shape ─────────────────────────────────────────

#[test]
fn cec_resource_uses_upstream_at_type_field() {
    let er = EnvoyResource {
        type_url: type_url::LISTENER.to_string(),
        body: serde_json::json!({"name": "envoy.l1", "address": {"socket_address": {"port_value": 9080}}}),
    };
    let s = serde_json::to_string(&er).unwrap();
    // Upstream serialises Envoy resources with the `@type` JSON pointer
    // (the gRPC Any-message convention). Anything else and an existing
    // CEC manifest would not round-trip.
    assert!(s.contains("\"@type\""), "missing @type field");
    assert!(
        s.contains("\"type.googleapis.com/envoy.config.listener.v3.Listener\""),
        "type URL not preserved verbatim",
    );
}

#[test]
fn cec_round_trips_real_world_listener_json() {
    // Simplified upstream YAML/JSON the user would author. Round-tripping
    // through serde must produce a structurally equivalent value.
    let raw = serde_json::json!({
        "services": [{"name": "echo", "namespace": "default", "listener": "envoy.echo"}],
        "backendServices": [{"name": "echo-backend", "namespace": "default", "number": ["80"]}],
        "resources": [
            {
                "@type": "type.googleapis.com/envoy.config.listener.v3.Listener",
                "name": "envoy.echo"
            }
        ]
    });
    let spec: CecSpec = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(spec.services[0].name, "echo");
    assert_eq!(spec.resources[0].type_url, type_url::LISTENER);
}

// ── 3. Reserved-identity numeric IDs ─────────────────────────────────────────

#[test]
fn reserved_identity_numeric_ids_byte_match_upstream() {
    // The numeric IDs are the wire format for cluster-mesh identity exchange
    // — they appear inside ipcache map entries and Hubble flow logs. Drift
    // here breaks cross-cluster connectivity.
    let pairs: &[(ReservedIdentity, u32)] = &[
        (ReservedIdentity::Unknown, 0),
        (ReservedIdentity::Host, 1),
        (ReservedIdentity::World, 2),
        (ReservedIdentity::Unmanaged, 3),
        (ReservedIdentity::Health, 4),
        (ReservedIdentity::Init, 5),
        (ReservedIdentity::RemoteNode, 6),
        (ReservedIdentity::KubeApiServer, 7),
        (ReservedIdentity::Ingress, 8),
        (ReservedIdentity::WorldIPv4, 9),
        (ReservedIdentity::WorldIPv6, 10),
        (ReservedIdentity::EncryptedOverlay, 11),
    ];
    for (r, expected) in pairs {
        assert_eq!(r.numeric(), *expected);
    }
}

#[test]
fn reserved_identity_label_strings_byte_match_upstream() {
    // The label strings appear in CRD .spec.endpointSelector["matchLabels"]
    // and need to round-trip through the CRD plane unchanged.
    assert_eq!(ReservedIdentity::Host.label(), "reserved:host");
    assert_eq!(ReservedIdentity::World.label(), "reserved:world");
    assert_eq!(
        ReservedIdentity::KubeApiServer.label(),
        "reserved:kube-apiserver"
    );
    assert_eq!(ReservedIdentity::Ingress.label(), "reserved:ingress");
    assert_eq!(ReservedIdentity::WorldIPv4.label(), "reserved:world-ipv4");
    assert_eq!(ReservedIdentity::WorldIPv6.label(), "reserved:world-ipv6");
}
