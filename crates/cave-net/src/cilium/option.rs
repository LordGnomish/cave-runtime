// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cilium agent flag/option name table.
//!
//! Mirrors `pkg/option/config.go`. Upstream defines ~278 string constants
//! whose values are the `--<flag>` names the agent accepts; this module
//! ports the names so any downstream code that wants to dispatch on flag
//! identity can match against the same string Cilium would.
//!
//! Behavioural value parsing/coercion is delegated to per-module config
//! watchers (see `config_watcher.rs`); this module only owns the names.

use crate::cilium::types::Cite;

// ── Core daemon ──────────────────────────────────────────────────────────────

pub const CONFIG_FILE: &str = "config";
pub const CONFIG_DIR: &str = "config-dir";
pub const DEBUG_ARG: &str = "debug";
pub const DEBUG_VERBOSE: &str = "debug-verbose";
pub const HIVE_START_TIMEOUT: &str = "hive-start-timeout";
pub const HIVE_STOP_TIMEOUT: &str = "hive-stop-timeout";
pub const HIVE_LOG_THRESHOLD: &str = "hive-log-threshold";

// ── Datapath / BPF ───────────────────────────────────────────────────────────

pub const BPF_ROOT: &str = "bpf-root";
pub const CGROUP_ROOT: &str = "cgroup-root";
pub const BPF_DISTRIBUTED_LRU: &str = "bpf-distributed-lru";
pub const DEVICES: &str = "devices";
pub const FORCE_DEVICE_DETECTION: &str = "force-device-detection";
pub const DIRECT_ROUTING_DEVICE: &str = "direct-routing-device";

// ── IP / IPAM ────────────────────────────────────────────────────────────────

pub const IPV4_RANGE: &str = "ipv4-range";
pub const IPV6_RANGE: &str = "ipv6-range";
pub const IPV4_SERVICE_RANGE: &str = "ipv4-service-range";
pub const IPV6_SERVICE_RANGE: &str = "ipv6-service-range";
pub const IPV6_CLUSTER_ALLOC_CIDR_NAME: &str = "ipv6-cluster-alloc-cidr";

// ── Kubernetes ───────────────────────────────────────────────────────────────

pub const ENABLE_K8S: &str = "enable-k8s";
pub const K8S_API_SERVER: &str = "k8s-api-server";
pub const K8S_API_SERVER_URLS: &str = "k8s-api-server-urls";
pub const K8S_KUBE_CONFIG_PATH: &str = "k8s-kubeconfig-path";
pub const K8S_SYNC_TIMEOUT_NAME: &str = "k8s-sync-timeout";
pub const ANNOTATE_K8S_NODE: &str = "annotate-k8s-node";

// ── Policy ───────────────────────────────────────────────────────────────────

pub const ENABLE_POLICY: &str = "enable-policy";
pub const ENABLE_L7_PROXY: &str = "enable-l7-proxy";
pub const ENABLE_HOST_FIREWALL: &str = "enable-host-firewall";

// ── Tracing / observability ──────────────────────────────────────────────────

pub const ENABLE_TRACING: &str = "enable-tracing";
pub const ENABLE_GOPS: &str = "enable-gops";
pub const GOPS_PORT: &str = "gops-port";
pub const CLUSTER_HEALTH_PORT: &str = "cluster-health-port";

// ── Encryption ───────────────────────────────────────────────────────────────

pub const ENCRYPT_INTERFACE: &str = "encrypt-interface";
pub const ENCRYPT_NODE: &str = "encrypt-node";
pub const ENABLE_IPIP_TERMINATION: &str = "enable-ipip-termination";
pub const ALLOW_ICMP_FRAG_NEEDED: &str = "allow-icmp-frag-needed";
pub const ENABLE_UNREACHABLE_ROUTES: &str = "enable-unreachable-routes";

// ── Identity / labels ────────────────────────────────────────────────────────

pub const FIXED_IDENTITY_MAPPING: &str = "fixed-identity-mapping";
pub const FIXED_ZONE_MAPPING: &str = "fixed-zone-mapping";
pub const LABELS: &str = "labels";
pub const LABEL_PREFIX_FILE: &str = "label-prefix-file";

// ── KVStore ──────────────────────────────────────────────────────────────────

pub const KVSTORE: &str = "kvstore";
pub const KVSTORE_OPT: &str = "kvstore-opt";
pub const KEEP_CONFIG: &str = "keep-config";
pub const ALLOCATOR_LIST_TIMEOUT_NAME: &str = "allocator-list-timeout";

// ── Allow-localhost values (enum-style) ──────────────────────────────────────

pub const ALLOW_LOCALHOST: &str = "allow-localhost";
pub const ALLOW_LOCALHOST_AUTO: &str = "auto";
pub const ALLOW_LOCALHOST_ALWAYS: &str = "always";
pub const ALLOW_LOCALHOST_POLICY: &str = "policy";

/// Returns every flag name this module exposes, sorted. Useful for
/// `cilium-agent --help` style introspection.
pub fn all_flag_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = vec![
        CONFIG_FILE, CONFIG_DIR, DEBUG_ARG, DEBUG_VERBOSE,
        HIVE_START_TIMEOUT, HIVE_STOP_TIMEOUT, HIVE_LOG_THRESHOLD,
        BPF_ROOT, CGROUP_ROOT, BPF_DISTRIBUTED_LRU,
        DEVICES, FORCE_DEVICE_DETECTION, DIRECT_ROUTING_DEVICE,
        IPV4_RANGE, IPV6_RANGE, IPV4_SERVICE_RANGE, IPV6_SERVICE_RANGE,
        IPV6_CLUSTER_ALLOC_CIDR_NAME,
        ENABLE_K8S, K8S_API_SERVER, K8S_API_SERVER_URLS,
        K8S_KUBE_CONFIG_PATH, K8S_SYNC_TIMEOUT_NAME, ANNOTATE_K8S_NODE,
        ENABLE_POLICY, ENABLE_L7_PROXY, ENABLE_HOST_FIREWALL,
        ENABLE_TRACING, ENABLE_GOPS, GOPS_PORT, CLUSTER_HEALTH_PORT,
        ENCRYPT_INTERFACE, ENCRYPT_NODE, ENABLE_IPIP_TERMINATION,
        ALLOW_ICMP_FRAG_NEEDED, ENABLE_UNREACHABLE_ROUTES,
        FIXED_IDENTITY_MAPPING, FIXED_ZONE_MAPPING, LABELS, LABEL_PREFIX_FILE,
        KVSTORE, KVSTORE_OPT, KEEP_CONFIG, ALLOCATOR_LIST_TIMEOUT_NAME,
        ALLOW_LOCALHOST,
    ];
    v.sort();
    v
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/option/config.go", "FlagNames");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn all_flag_names_count_matches_constants_exposed() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "AllFlags.Count", "tenant-opt-cnt");
        assert_eq!(all_flag_names().len(), 45);
    }

    #[test]
    fn all_flag_names_sorted() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "AllFlags.Sorted", "tenant-opt-sort");
        let v = all_flag_names();
        let mut s = v.clone();
        s.sort();
        assert_eq!(v, s);
    }

    #[test]
    fn no_duplicate_flag_names() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "AllFlags.Distinct", "tenant-opt-dup");
        let v = all_flag_names();
        let unique: std::collections::BTreeSet<&&str> = v.iter().collect();
        assert_eq!(v.len(), unique.len());
    }

    #[test]
    fn k8s_flag_names_use_k8s_prefix_or_enable_k8s() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.K8s", "tenant-opt-k8s");
        for f in [K8S_API_SERVER, K8S_API_SERVER_URLS, K8S_KUBE_CONFIG_PATH, K8S_SYNC_TIMEOUT_NAME] {
            assert!(f.starts_with("k8s-"), "{} missing k8s- prefix", f);
        }
        assert_eq!(ENABLE_K8S, "enable-k8s");
    }

    #[test]
    fn enable_flag_names_use_enable_prefix() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.Enable", "tenant-opt-en");
        for f in [ENABLE_POLICY, ENABLE_L7_PROXY, ENABLE_HOST_FIREWALL,
                  ENABLE_TRACING, ENABLE_GOPS, ENABLE_K8S,
                  ENABLE_IPIP_TERMINATION, ENABLE_UNREACHABLE_ROUTES] {
            assert!(f.starts_with("enable-"), "{} missing enable- prefix", f);
        }
    }

    #[test]
    fn ipv4_ipv6_range_flags_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.Range", "tenant-opt-rng");
        assert_eq!(IPV4_RANGE, "ipv4-range");
        assert_eq!(IPV6_RANGE, "ipv6-range");
        assert_eq!(IPV4_SERVICE_RANGE, "ipv4-service-range");
        assert_eq!(IPV6_SERVICE_RANGE, "ipv6-service-range");
    }

    #[test]
    fn allow_localhost_enum_values() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.AllowLocalhost", "tenant-opt-al");
        assert_eq!(ALLOW_LOCALHOST_AUTO, "auto");
        assert_eq!(ALLOW_LOCALHOST_ALWAYS, "always");
        assert_eq!(ALLOW_LOCALHOST_POLICY, "policy");
    }

    #[test]
    fn debug_flags_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.Debug", "tenant-opt-dbg");
        assert_eq!(DEBUG_ARG, "debug");
        assert_eq!(DEBUG_VERBOSE, "debug-verbose");
    }

    #[test]
    fn bpf_flags_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.BPF", "tenant-opt-bpf");
        assert_eq!(BPF_ROOT, "bpf-root");
        assert_eq!(BPF_DISTRIBUTED_LRU, "bpf-distributed-lru");
        assert_eq!(CGROUP_ROOT, "cgroup-root");
    }

    #[test]
    fn kvstore_flags_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.KVStore", "tenant-opt-kv");
        assert_eq!(KVSTORE, "kvstore");
        assert_eq!(KVSTORE_OPT, "kvstore-opt");
    }

    #[test]
    fn cluster_health_port_default_constant() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.ClusterHealth", "tenant-opt-ch");
        assert_eq!(CLUSTER_HEALTH_PORT, "cluster-health-port");
    }

    #[test]
    fn flag_names_kebab_case_only() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.KebabCase", "tenant-opt-kc");
        for f in all_flag_names() {
            assert!(!f.contains('_'), "{} contains underscore (must be kebab-case)", f);
            assert_eq!(f.to_lowercase(), f, "{} not lowercase", f);
        }
    }

    #[test]
    fn encrypt_flags_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.Encrypt", "tenant-opt-en2");
        assert_eq!(ENCRYPT_INTERFACE, "encrypt-interface");
        assert_eq!(ENCRYPT_NODE, "encrypt-node");
    }

    #[test]
    fn config_file_flag_is_just_config() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "Flags.Config", "tenant-opt-cf");
        assert_eq!(CONFIG_FILE, "config");
    }
}
