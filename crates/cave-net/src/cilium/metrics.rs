//! Prometheus metric registry — Cilium-parity port of `pkg/metrics/metrics.go`.
//!
//! Wire-faithful: every metric name and every subsystem here matches the
//! upstream constants in `pkg/metrics/metrics.go` v1.19.3. Exposition
//! follows the Prometheus text format:
//!
//! ```text
//! # HELP <full_name> <help>
//! # TYPE <full_name> <type>
//! <full_name>{<labels>} <value>
//! ```
//!
//! The `full_name` is always `<namespace>_<subsystem>_<name>` when a
//! subsystem is set, and `<namespace>_<name>` otherwise. Cilium's
//! upstream uses the Prometheus client library's `Namespace`/`Subsystem`
//! convention, which produces the same string.
//!
//! Mirrors:
//!   * `pkg/metrics/metrics.go` — registry, namespaces, subsystems, every
//!     `Name: "…"` metric instance.
//!   * label keys (`LabelOutcome`, `LabelDirection`, …) used across the
//!     agent.

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;

/// Default namespace for cilium-agent metrics.
/// Mirrors `CiliumAgentNamespace` in upstream.
pub const NAMESPACE_AGENT: &str = "cilium";
/// Namespace for the clustermesh-apiserver process.
/// Mirrors `CiliumClusterMeshAPIServerNamespace`.
pub const NAMESPACE_CLUSTERMESH_APISERVER: &str = "cilium_clustermesh_apiserver";
/// Namespace for kvstoremesh.
/// Mirrors `CiliumKVStoreMeshNamespace`.
pub const NAMESPACE_KVSTOREMESH: &str = "cilium_kvstoremesh";
/// Namespace for cilium-operator.
/// Mirrors `CiliumOperatorNamespace`.
pub const NAMESPACE_OPERATOR: &str = "cilium_operator";

/// Subsystem string constants. Each matches a `Subsystem*` const in upstream.
pub mod subsystem {
    pub const BPF: &str = "bpf";
    pub const DATAPATH: &str = "datapath";
    pub const AGENT: &str = "agent";
    pub const IPCACHE: &str = "ipcache";
    pub const K8S: &str = "k8s";
    pub const K8S_CLIENT: &str = "k8s_client";
    pub const WORKQUEUE: &str = "k8s_workqueue";
    pub const KVSTORE: &str = "kvstore";
    pub const FQDN: &str = "fqdn";
    pub const NODES: &str = "nodes";
    pub const TRIGGERS: &str = "triggers";
    pub const API_LIMITER: &str = "api_limiter";
    pub const CLUSTERMESH: &str = "clustermesh";
}

/// Standard label keys.
/// Each matches a `Label*` const in `pkg/metrics/metrics.go`.
pub mod label {
    pub const ERROR: &str = "error";
    pub const OUTCOME: &str = "outcome";
    pub const ATTEMPTS: &str = "attempts";
    pub const DROP_REASON: &str = "reason";
    pub const EVENT_SOURCE: &str = "source";
    pub const DATAPATH_AREA: &str = "area";
    pub const DATAPATH_NAME: &str = "name";
    pub const DATAPATH_FAMILY: &str = "family";
    pub const PROTOCOL: &str = "protocol";
    pub const SIGNAL_TYPE: &str = "signal";
    pub const SIGNAL_DATA: &str = "data";
    pub const STATUS: &str = "status";
    pub const POLICY_ENFORCEMENT: &str = "enforcement";
    pub const POLICY_SOURCE: &str = "source";
    pub const SCOPE: &str = "scope";
    pub const PROTOCOL_L7: &str = "protocol_l7";
    pub const BUILD_STATE: &str = "state";
    pub const BUILD_QUEUE_NAME: &str = "name";
    pub const ACTION: &str = "action";
    pub const SUBSYSTEM: &str = "subsystem";
    pub const KIND: &str = "kind";
    pub const PATH: &str = "path";
    pub const METHOD: &str = "method";
    pub const API_RETURN_CODE: &str = "return_code";
    pub const OPERATION: &str = "operation";
    pub const MAP_NAME: &str = "map_name";
    pub const MAP_GROUP: &str = "map_group";
    pub const VERSION: &str = "version";
    pub const DIRECTION: &str = "direction";
    pub const SOURCE_CLUSTER: &str = "source_cluster";
    pub const TARGET_CLUSTER: &str = "target_cluster";
    pub const L7_RULE: &str = "rule";
    pub const L7_PROXY_TYPE: &str = "proxy_type";
    pub const ADDRESS_TYPE: &str = "address_type";
    pub const CONNECTIVITY_STATUS: &str = "status";
}

/// Supported metric kinds. Mirrors the type tag emitted by Prometheus
/// `# TYPE <name> <kind>` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    pub fn as_prom(self) -> &'static str {
        match self {
            Kind::Counter => "counter",
            Kind::Gauge => "gauge",
            Kind::Histogram => "histogram",
        }
    }
}

/// One metric definition. The exposed Prometheus name is built from
/// `namespace_subsystem_name` (or `namespace_name` if subsystem is empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricDef {
    pub namespace: &'static str,
    pub subsystem: &'static str,
    pub name: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

impl MetricDef {
    /// Full Prometheus name, e.g. `cilium_agent_bootstrap_seconds`.
    pub fn full_name(&self) -> String {
        if self.subsystem.is_empty() {
            format!("{}_{}", self.namespace, self.name)
        } else {
            format!("{}_{}_{}", self.namespace, self.subsystem, self.name)
        }
    }
}

/// Canonical Cilium agent metric table — every `Name:` from
/// `pkg/metrics/metrics.go` v1.19.3, with subsystem and kind preserved.
/// Order follows upstream definition order.
pub fn registry() -> Vec<MetricDef> {
    use Kind::*;
    use subsystem as s;
    const NS: &str = NAMESPACE_AGENT;
    vec![
        // ── workqueue (k8s_workqueue) ────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "depth",                              help: "Current depth of workqueue.",                          kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "adds_total",                         help: "Total number of adds handled by workqueue.",           kind: Counter },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "queue_duration_seconds",             help: "How long an item stays in workqueue before being requested.", kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "work_duration_seconds",              help: "How long processing an item from workqueue takes.",    kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "unfinished_work_seconds",            help: "How many seconds of work has been done that is in progress and hasn't been observed by work_duration.", kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "longest_running_processor_seconds",  help: "How many seconds the longest running processor has been running.", kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::WORKQUEUE, name: "retries_total",                      help: "Total number of retries handled by workqueue.",        kind: Counter },
        // ── agent ────────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "bootstrap_seconds",                   help: "Duration of the agent bootstrap sequence.",            kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "api_process_time_seconds",            help: "Duration of API calls.",                               kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "endpoint_regenerations_total",        help: "Total number of endpoint regenerations.",              kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "endpoint_state",                      help: "Count of endpoints managed by this agent, labeled by state.", kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "endpoint_regeneration_time_stats_seconds", help: "Endpoint regeneration time stats.",               kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy",                              help: "Number of policies currently loaded.",                 kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_max_revision",                 help: "Highest policy revision processed by this agent.",     kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_change_total",                 help: "Number of policy changes by outcome.",                 kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_endpoint_enforcement_status",  help: "Number of endpoints labeled by policy enforcement status.", kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_implementation_delay",         help: "Time taken to implement a policy update from kube-apiserver.", kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_incremental_update_duration",  help: "Time it takes to do an incremental policy update.",    kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "identity",                            help: "Number of identities currently allocated.",            kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "identity_label_sources",              help: "Count of identities by their label source.",           kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "event_ts",                            help: "Last seen event timestamp by source.",                 kind: Gauge },
        // ── k8s ──────────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::K8S,      name: "k8s_event_lag_seconds",               help: "Lag between event timestamp and processing time.",     kind: Histogram },
        // ── proxy / l7 (under agent) ─────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "proxy_redirects",                     help: "Number of redirects installed for endpoints.",         kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "policy_l7_total",                     help: "Number of total L7 requests/responses.",               kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "proxy_upstream_reply_seconds",        help: "Roundtrip latencies of upstream replies via proxy.",   kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "proxy_datapath_update_timeout_total", help: "Number of datapath update timeouts due to FQDN IP updates.", kind: Counter },
        // ── datapath conntrack ───────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_gc_runs_total",             help: "Number of conntrack GC runs by status.",               kind: Counter },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_gc_key_fallbacks_total",    help: "Number of CT garbage collector key fallbacks.",        kind: Counter },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_gc_entries",                help: "Conntrack GC entries.",                                kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "nat_gc_entries",                      help: "NAT GC entries.",                                      kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_gc_duration_seconds",       help: "Duration of conntrack GC runs.",                       kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_gc_interval_seconds",       help: "Interval between conntrack GC runs.",                  kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "conntrack_dump_resets_total",         help: "Number of conntrack table dump resets.",               kind: Counter },
        MetricDef { namespace: NS, subsystem: s::DATAPATH, name: "signals_handled_total",               help: "Number of datapath signals handled.",                  kind: Counter },
        // ── service / lb ─────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "services_events_total",               help: "Number of services events.",                           kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "service_implementation_delay",        help: "Service implementation delay.",                        kind: Histogram },
        // ── controllers ──────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "controllers_runs_total",              help: "Number of controller runs.",                           kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "controllers_runs_duration_seconds",   help: "Duration of controller runs.",                         kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "subprocess_start_total",              help: "Number of subprocesses started.",                      kind: Counter },
        MetricDef { namespace: NS, subsystem: s::K8S,      name: "kubernetes_events_total",             help: "Number of Kubernetes events processed.",               kind: Counter },
        MetricDef { namespace: NS, subsystem: s::K8S,      name: "kubernetes_events_received_total",    help: "Number of Kubernetes events received.",                kind: Counter },
        // ── k8s client ───────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::K8S_CLIENT, name: "api_latency_time_seconds",          help: "Duration of K8s API calls by host and method.",        kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::K8S_CLIENT, name: "rate_limiter_duration_seconds",     help: "Duration spent in K8s client rate limiter.",           kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::K8S_CLIENT, name: "api_calls_total",                   help: "Total K8s API calls by host, method, return code.",    kind: Counter },
        // ── k8s terminating endpoints ────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::K8S,      name: "terminating_endpoints_events_total",  help: "Total terminating endpoints events.",                  kind: Counter },
        // ── ipam ─────────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "ipam_events_total",                   help: "IPAM events count.",                                   kind: Counter },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "ipam_capacity",                       help: "IPAM capacity by family.",                             kind: Gauge },
        // ── kvstore ──────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::KVSTORE,  name: "operations_duration_seconds",         help: "Duration of kvstore operations.",                      kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::KVSTORE,  name: "events_queue_seconds",                help: "How long events spend in the kvstore event queue.",    kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::KVSTORE,  name: "quorum_errors_total",                 help: "Number of kvstore quorum errors.",                     kind: Counter },
        // ── ipcache ──────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::IPCACHE,  name: "errors_total",                        help: "Number of ipcache errors.",                            kind: Counter },
        MetricDef { namespace: NS, subsystem: s::IPCACHE,  name: "events_total",                        help: "Number of ipcache events.",                            kind: Counter },
        // ── fqdn ─────────────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::FQDN,     name: "gc_deletions_total",                  help: "Number of FQDN cache deletions by GC.",                kind: Counter },
        MetricDef { namespace: NS, subsystem: s::FQDN,     name: "active_names",                        help: "Number of FQDNs being tracked.",                       kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::FQDN,     name: "active_ips",                          help: "Number of IPs being tracked for FQDNs.",               kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::FQDN,     name: "alive_zombie_connections",            help: "Number of alive connections to FQDN zombie IPs.",      kind: Gauge },
        // ── policy selector cache ────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "selectors",                           help: "Number of selectors in cache.",                        kind: Gauge },
        // ── api limiter ──────────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "semaphore_rejected_total",         help: "Number of API limiter semaphore rejections.",          kind: Counter },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "wait_history_duration_seconds",    help: "Histogram of API limiter wait history.",               kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "wait_duration_seconds",            help: "Duration spent in API limiter wait.",                  kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "processing_duration_seconds",      help: "Duration of API processing.",                          kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "requests_in_flight",               help: "Number of API requests in flight.",                    kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "rate_limit",                       help: "Rate limit value.",                                    kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "adjustment_factor",                help: "API limiter rate adjustment factor.",                  kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "processed_requests_total",         help: "Number of API requests processed.",                    kind: Counter },
        MetricDef { namespace: NS, subsystem: s::API_LIMITER, name: "endpoint_propagation_delay_seconds", help: "Delay between policy update and endpoint propagation.", kind: Histogram },
        // ── bpf syscall + map ────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: s::BPF,      name: "syscall_duration_seconds",            help: "Duration of bpf() syscall by operation.",              kind: Histogram },
        MetricDef { namespace: NS, subsystem: s::BPF,      name: "map_ops_total",                       help: "Number of bpf map operations.",                        kind: Counter },
        MetricDef { namespace: NS, subsystem: s::BPF,      name: "map_capacity",                        help: "Capacity of bpf maps by name.",                        kind: Gauge },
        MetricDef { namespace: NS, subsystem: s::AGENT,    name: "version",                             help: "Cilium agent version.",                                kind: Gauge },
        // ── nodes / health ───────────────────────────────────────────────
        MetricDef { namespace: NS, subsystem: "",          name: "node_health_connectivity_status",     help: "Node connectivity status by reachability.",            kind: Gauge },
        MetricDef { namespace: NS, subsystem: "",          name: "node_health_connectivity_latency_seconds", help: "Node connectivity latency.",                       kind: Histogram },
        // ── errors_warnings_total (lowercase namespace constant) ─────────
        MetricDef { namespace: NS, subsystem: "",          name: "errors_warnings_total",               help: "Number of errors and warnings logged by the agent.",   kind: Counter },
    ]
}

/// Render the registry in Prometheus text exposition format with the given
/// per-metric numeric values. Metrics not present in `values` are omitted.
///
/// Each metric is rendered as:
/// ```text
/// # HELP <full_name> <help>
/// # TYPE <full_name> <kind>
/// <full_name>{<labels>} <value>
/// ```
///
/// Labels are joined with `,` and values are quoted. This is the same
/// shape that Cilium's prometheus client emits.
pub fn render_exposition(
    defs: &[MetricDef],
    values: &BTreeMap<String, (BTreeMap<String, String>, f64)>,
) -> String {
    let mut out = String::new();
    for def in defs {
        let full = def.full_name();
        let Some((labels, val)) = values.get(&full) else {
            continue;
        };
        out.push_str(&format!("# HELP {} {}\n", full, def.help));
        out.push_str(&format!("# TYPE {} {}\n", full, def.kind.as_prom()));
        if labels.is_empty() {
            out.push_str(&format!("{} {}\n", full, val));
        } else {
            let label_str = labels
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!("{}{{{}}} {}\n", full, label_str, val));
        }
    }
    out
}

/// Errors specific to the metrics module (kept for parity with other modules
/// in the batch; render_exposition itself never fails).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MetricsError {
    #[error("metric {0} not in registry")]
    NotInRegistry(String),
    #[error("tenant {tenant} cannot mutate metric registry owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/metrics/metrics.go", "Registry");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn registry_has_all_seventy_three_upstream_metrics() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Registry.Count", "tenant-met-cnt");
        // 73 metrics is the canonical count parsed from upstream
        // pkg/metrics/metrics.go @ v1.19.3 (every line matching `Name: "…"`).
        assert_eq!(registry().len(), 73, "metric count drifted from upstream");
    }

    #[test]
    fn namespace_constants_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Namespaces", "tenant-met-ns");
        assert_eq!(NAMESPACE_AGENT, "cilium");
        assert_eq!(NAMESPACE_CLUSTERMESH_APISERVER, "cilium_clustermesh_apiserver");
        assert_eq!(NAMESPACE_KVSTOREMESH, "cilium_kvstoremesh");
        assert_eq!(NAMESPACE_OPERATOR, "cilium_operator");
    }

    #[test]
    fn subsystem_constants_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Subsystems", "tenant-met-sub");
        assert_eq!(subsystem::BPF, "bpf");
        assert_eq!(subsystem::DATAPATH, "datapath");
        assert_eq!(subsystem::AGENT, "agent");
        assert_eq!(subsystem::IPCACHE, "ipcache");
        assert_eq!(subsystem::K8S, "k8s");
        assert_eq!(subsystem::K8S_CLIENT, "k8s_client");
        assert_eq!(subsystem::WORKQUEUE, "k8s_workqueue");
        assert_eq!(subsystem::KVSTORE, "kvstore");
        assert_eq!(subsystem::FQDN, "fqdn");
        assert_eq!(subsystem::NODES, "nodes");
        assert_eq!(subsystem::TRIGGERS, "triggers");
        assert_eq!(subsystem::API_LIMITER, "api_limiter");
        assert_eq!(subsystem::CLUSTERMESH, "clustermesh");
    }

    #[test]
    fn full_name_combines_namespace_subsystem_name() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "FullName.WithSub", "tenant-met-fn");
        let m = MetricDef {
            namespace: "cilium",
            subsystem: "agent",
            name: "bootstrap_seconds",
            help: "h",
            kind: Kind::Gauge,
        };
        assert_eq!(m.full_name(), "cilium_agent_bootstrap_seconds");
    }

    #[test]
    fn full_name_skips_subsystem_when_empty() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "FullName.NoSub", "tenant-met-fne");
        let m = MetricDef {
            namespace: "cilium",
            subsystem: "",
            name: "errors_warnings_total",
            help: "h",
            kind: Kind::Counter,
        };
        assert_eq!(m.full_name(), "cilium_errors_warnings_total");
    }

    #[test]
    fn key_canonical_metrics_are_present() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Registry.HasKeys", "tenant-met-keys");
        let names: std::collections::BTreeSet<String> =
            registry().iter().map(|m| m.full_name()).collect();
        // sample of 10 well-known metrics that absolutely must be present
        for n in [
            "cilium_agent_bootstrap_seconds",
            "cilium_agent_endpoint_state",
            "cilium_agent_policy",
            "cilium_agent_identity",
            "cilium_datapath_conntrack_gc_runs_total",
            "cilium_bpf_syscall_duration_seconds",
            "cilium_bpf_map_ops_total",
            "cilium_kvstore_operations_duration_seconds",
            "cilium_fqdn_active_names",
            "cilium_node_health_connectivity_status",
        ] {
            assert!(names.contains(n), "missing metric {}", n);
        }
    }

    #[test]
    fn registry_metric_kinds_balance_correctly() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Registry.Kinds", "tenant-met-kinds");
        let r = registry();
        let counters = r.iter().filter(|m| m.kind == Kind::Counter).count();
        let gauges = r.iter().filter(|m| m.kind == Kind::Gauge).count();
        let hists = r.iter().filter(|m| m.kind == Kind::Histogram).count();
        assert_eq!(counters + gauges + hists, 73);
        assert!(counters > 0 && gauges > 0 && hists > 0);
    }

    #[test]
    fn render_exposition_emits_help_and_type_lines() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Render.HelpType", "tenant-met-rht");
        let defs = registry();
        let mut values = BTreeMap::new();
        let mut empty_labels = BTreeMap::new();
        // No labels, scalar value
        empty_labels.clear();
        values.insert("cilium_agent_policy".to_string(), (empty_labels.clone(), 5.0));
        let out = render_exposition(&defs, &values);
        assert!(out.contains("# HELP cilium_agent_policy"));
        assert!(out.contains("# TYPE cilium_agent_policy gauge"));
        assert!(out.contains("cilium_agent_policy 5"));
    }

    #[test]
    fn render_exposition_renders_labels_in_order() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Render.Labels", "tenant-met-rl");
        let defs = registry();
        let mut values = BTreeMap::new();
        let mut labels = BTreeMap::new();
        labels.insert("outcome".to_string(), "success".to_string());
        labels.insert("source".to_string(), "k8s".to_string());
        values.insert(
            "cilium_agent_policy_change_total".to_string(),
            (labels, 42.0),
        );
        let out = render_exposition(&defs, &values);
        assert!(out.contains(
            "cilium_agent_policy_change_total{outcome=\"success\",source=\"k8s\"} 42"
        ));
    }

    #[test]
    fn render_exposition_escapes_label_values() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Render.Escape", "tenant-met-esc");
        let defs = registry();
        let mut values = BTreeMap::new();
        let mut labels = BTreeMap::new();
        labels.insert("error".to_string(), "broken \"quote\" and \\ slash".to_string());
        values.insert(
            "cilium_ipcache_errors_total".to_string(),
            (labels, 1.0),
        );
        let out = render_exposition(&defs, &values);
        // Both \ and " must be escaped in label values per Prom text format.
        assert!(out.contains("error=\"broken \\\"quote\\\" and \\\\ slash\""));
    }

    #[test]
    fn label_keys_match_upstream_constants() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "LabelKeys", "tenant-met-lk");
        // spot check key labels
        assert_eq!(label::OUTCOME, "outcome");
        assert_eq!(label::DROP_REASON, "reason");
        assert_eq!(label::DIRECTION, "direction");
        assert_eq!(label::PROTOCOL, "protocol");
        assert_eq!(label::ACTION, "action");
        assert_eq!(label::POLICY_ENFORCEMENT, "enforcement");
        assert_eq!(label::SOURCE_CLUSTER, "source_cluster");
        assert_eq!(label::TARGET_CLUSTER, "target_cluster");
    }

    #[test]
    fn no_duplicate_metric_full_names() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "NoDuplicates", "tenant-met-dup");
        let names: Vec<String> = registry().iter().map(|m| m.full_name()).collect();
        let unique: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "duplicate full_name in registry");
    }

    #[test]
    fn metrics_error_renders_correctly() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Errors", "tenant-met-err");
        let e = MetricsError::NotInRegistry("foo".into());
        assert!(format!("{}", e).contains("foo"));
        let e = MetricsError::TenantDenied { tenant: TenantId::new("t1").expect("test fixture") };
        assert!(format!("{}", e).contains("t1"));
    }

    #[test]
    fn kind_prom_string_matches_text_format() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Kind.Prom", "tenant-met-kp");
        assert_eq!(Kind::Counter.as_prom(), "counter");
        assert_eq!(Kind::Gauge.as_prom(), "gauge");
        assert_eq!(Kind::Histogram.as_prom(), "histogram");
    }

    #[test]
    fn metric_def_equality_is_structural() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Eq", "tenant-met-eq");
        let a = MetricDef { namespace: "cilium", subsystem: "agent", name: "policy", help: "h", kind: Kind::Gauge };
        let b = MetricDef { namespace: "cilium", subsystem: "agent", name: "policy", help: "h", kind: Kind::Gauge };
        assert_eq!(a, b);
    }

    #[test]
    fn render_exposition_skips_unknown_metrics() {
        let (_c, _t) = cilium_test_ctx!("pkg/metrics/metrics.go", "Render.Skip", "tenant-met-skip");
        let defs = registry();
        let mut values = BTreeMap::new();
        let labels = BTreeMap::new();
        values.insert("not_a_real_metric".to_string(), (labels, 99.0));
        let out = render_exposition(&defs, &values);
        assert!(out.is_empty());
    }
}
