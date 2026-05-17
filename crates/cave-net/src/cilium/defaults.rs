// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cilium agent default-value constants.
//!
//! Mirrors `pkg/defaults/defaults.go`. Every const here has the same name
//! and value as upstream so any agent-side code that references "the
//! default tunnel protocol" or "the default runtime path" produces the same
//! string Cilium would.

use crate::cilium::types::Cite;

// ── Filesystem paths ─────────────────────────────────────────────────────────

/// Directory where the agent stores its runtime sockets.
pub const RUNTIME_PATH: &str = "/var/run/cilium";
/// State subdirectory beneath [`RUNTIME_PATH`].
pub const STATE_DIR: &str = "state";
/// Templates subdirectory beneath [`RUNTIME_PATH`].
pub const TEMPLATES_DIR: &str = "templates";
/// Template-ID stamp file.
pub const TEMPLATE_ID_PATH: &str = "template.txt";
/// BPF subdir beneath the agent library path.
pub const BPF_DIR: &str = "bpf";
/// Default library path.
pub const LIBRARY_PATH: &str = "/var/lib/cilium";
/// Environment variable holding the agent unix socket.
pub const SOCK_PATH_ENV: &str = "CILIUM_SOCK";
/// BPF-fs root directory.
pub const BPFFS_ROOT: &str = "/sys/fs/bpf";
/// Fallback BPF-fs root.
pub const BPFFS_ROOT_FALLBACK: &str = "/run/cilium/bpffs";
/// Path under BPFFS where TC globals are pinned.
pub const TC_GLOBALS_PATH: &str = "tc/globals";
/// Default cgroup-v2 root.
pub const DEFAULT_CGROUP_ROOT: &str = "/run/cilium/cgroupv2";
/// Netns directory.
pub const NETNS_PATH: &str = "/var/run/cilium/netns";

// ── IP / CIDR ────────────────────────────────────────────────────────────────

/// Base prefix for the IPv6 cluster allocator.
pub const IPV6_CLUSTER_ALLOC_CIDR_BASE: &str = "f00d::";
/// NAT46x64 base prefix.
pub const IPV6_NAT46X64_CIDR_BASE: &str = "64:ff9b::";

// ── BPF object names ─────────────────────────────────────────────────────────

/// Filename for the BPF struct alignment checker object.
pub const ALIGN_CHECKER_NAME: &str = "bpf_alignchecker.o";

// ── IPAM ─────────────────────────────────────────────────────────────────────

/// Default IPAM pool name.
pub const IPAM_DEFAULT_IP_POOL: &str = "default";
/// ENI GC tag for the "managed" key.
pub const ENI_GC_TAG_MANAGED_NAME: &str = "io.cilium/cilium-managed";
/// ENI GC tag value for the "managed" key.
pub const ENI_GC_TAG_MANAGED_VALUE: &str = "true";
/// ENI GC tag for the cluster name.
pub const ENI_GC_TAG_CLUSTER_NAME: &str = "io.cilium/cluster-name";

// ── Datapath ─────────────────────────────────────────────────────────────────

/// SRv6 encap mode default.
pub const SRV6_ENCAP_MODE: &str = "reduced";
/// Default datapath mode.
pub const DATAPATH_MODE: &str = "veth";
/// Routing mode default.
pub const ROUTING_MODE: &str = "tunnel";
/// Default tunnel protocol.
pub const TUNNEL_PROTOCOL: &str = "vxlan";
/// Default tunnel source-port range ("0-0" = unset).
pub const TUNNEL_SOURCE_PORT_RANGE: &str = "0-0";
/// Default underlay IP family.
pub const UNDERLAY_PROTOCOL: &str = "ipv4";

// ── Restore (sysctl-style state keys) ────────────────────────────────────────

/// Sysctl key holding the IPv4 internal restore addr (value).
pub const RESTORE_V4_ADDR: &str = "cilium.v4.internal.raw ";
/// Sysctl key holding the IPv6 internal restore addr (value).
pub const RESTORE_V6_ADDR: &str = "cilium.v6.internal.raw ";

// ── Service / policy responses ───────────────────────────────────────────────

/// "no backend" response (reject vs drop).
pub const SERVICE_NO_BACKEND_RESPONSE: &str = "reject";
/// Policy deny response default.
pub const POLICY_DENY_RESPONSE: &str = "none";
/// "auto" container-ip reserved-ports value.
pub const CONTAINER_IP_LOCAL_RESERVED_PORTS_AUTO: &str = "auto";

// ── FQDN ─────────────────────────────────────────────────────────────────────

/// Default `toFQDNsPreCache` setting (empty string = no precache).
pub const TO_FQDNS_PRECACHE: &str = "";

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/defaults/defaults.go", "Constants");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn runtime_path_matches_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "RuntimePath", "tenant-def-rp");
        assert_eq!(RUNTIME_PATH, "/var/run/cilium");
    }

    #[test]
    fn library_path_matches_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "LibraryPath", "tenant-def-lp");
        assert_eq!(LIBRARY_PATH, "/var/lib/cilium");
    }

    #[test]
    fn bpffs_root_is_sysfs() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "BPFFSRoot", "tenant-def-bfs");
        assert_eq!(BPFFS_ROOT, "/sys/fs/bpf");
        assert_eq!(BPFFS_ROOT_FALLBACK, "/run/cilium/bpffs");
    }

    #[test]
    fn ipv6_cluster_alloc_base_is_f00d() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "IPv6ClusterAlloc", "tenant-def-cl");
        assert_eq!(IPV6_CLUSTER_ALLOC_CIDR_BASE, "f00d::");
    }

    #[test]
    fn nat46x64_base_is_well_known_prefix() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "NAT46x64", "tenant-def-nat");
        // RFC 6052 well-known NAT64 prefix.
        assert_eq!(IPV6_NAT46X64_CIDR_BASE, "64:ff9b::");
    }

    #[test]
    fn align_checker_name_matches_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "AlignCheckerName", "tenant-def-ac");
        assert_eq!(ALIGN_CHECKER_NAME, "bpf_alignchecker.o");
    }

    #[test]
    fn ipam_default_pool_is_default() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "IPAMDefault", "tenant-def-ip");
        assert_eq!(IPAM_DEFAULT_IP_POOL, "default");
    }

    #[test]
    fn eni_gc_tag_matches_io_cilium_keys() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "ENIGCTags", "tenant-def-eni");
        assert!(ENI_GC_TAG_MANAGED_NAME.starts_with("io.cilium/"));
        assert!(ENI_GC_TAG_CLUSTER_NAME.starts_with("io.cilium/"));
        assert_eq!(ENI_GC_TAG_MANAGED_VALUE, "true");
    }

    #[test]
    fn tunnel_defaults_are_vxlan_over_ipv4() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "Tunnel", "tenant-def-tun");
        assert_eq!(ROUTING_MODE, "tunnel");
        assert_eq!(TUNNEL_PROTOCOL, "vxlan");
        assert_eq!(UNDERLAY_PROTOCOL, "ipv4");
    }

    #[test]
    fn datapath_default_is_veth() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "Datapath", "tenant-def-dp");
        assert_eq!(DATAPATH_MODE, "veth");
    }

    #[test]
    fn srv6_encap_mode_default_is_reduced() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "SRv6", "tenant-def-srv6");
        assert_eq!(SRV6_ENCAP_MODE, "reduced");
    }

    #[test]
    fn restore_keys_have_trailing_space() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "Restore", "tenant-def-rs");
        // Upstream defines them with a trailing space — important for
        // sysctl key matching. Don't normalise it.
        assert!(RESTORE_V4_ADDR.ends_with(' '));
        assert!(RESTORE_V6_ADDR.ends_with(' '));
    }

    #[test]
    fn service_no_backend_default_is_reject() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "NoBackend", "tenant-def-nb");
        assert_eq!(SERVICE_NO_BACKEND_RESPONSE, "reject");
    }

    #[test]
    fn policy_deny_default_is_none() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "DenyResponse", "tenant-def-dn");
        assert_eq!(POLICY_DENY_RESPONSE, "none");
    }

    #[test]
    fn netns_path_matches_runtime_path() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "NetNs", "tenant-def-nn");
        assert!(NETNS_PATH.starts_with(RUNTIME_PATH));
    }

    #[test]
    fn cgroup_root_is_run_cilium_subdir() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "CGroup", "tenant-def-cg");
        assert!(DEFAULT_CGROUP_ROOT.starts_with("/run/cilium/"));
    }

    #[test]
    fn cilium_sock_env_name_constant() {
        let (_c, _t) = cilium_test_ctx!("pkg/defaults/defaults.go", "SockEnv", "tenant-def-se");
        assert_eq!(SOCK_PATH_ENV, "CILIUM_SOCK");
    }
}
