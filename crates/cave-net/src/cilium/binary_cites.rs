//! Cross-binary citations.
//!
//! Cilium ships several standalone binaries beyond cilium-agent. Each
//! one's *logic* is implemented inside an agent-side library that this
//! crate already ports; the binaries themselves are thin entrypoints
//! that wire flags into a `main()`.
//!
//! Rather than leave the upstream top-level binary directories as
//! "unmapped" — which would falsely imply the relevant logic is
//! missing — this module records the agent-side equivalent.
//!
//! Mirrors the entrypoint thinning of:
//!
//!   * `bpf/` — kernel-side eBPF C source (`bpf_lxc.c`, `bpf_host.c`,
//!     `bpf/lib/conntrack.h`, `bpf/lib/lb.h`, `bpf/lib/nat.h`,
//!     `bpf/lib/srv6.h`, `bpf/lib/ipv6.h`). The semantic state machines
//!     are ported in `cilium::conntrack`, `cilium::nat`, `cilium::lb`,
//!     `cilium::srv6`, `cilium::ipv6`. The actual BPF bytecode generation
//!     (clang → BPF instruction stream → kernel verifier load) is *not*
//!     ported; see PARITY_REPORT.md "Wire-faithful exclusions".
//!   * `hubble-relay/main.go` — entrypoint for the hubble-relay binary.
//!     Agent-side flow ingest + topology graph live in
//!     `cilium::hubble`, `cilium::hubble_ext`, `cilium::hubble_metrics`.
//!   * `clustermesh-apiserver/main.go` — entrypoint for the clustermesh-
//!     apiserver. Multi-cluster identity exchange + service announce
//!     live in `cilium::clustermesh`, `cilium::clustermesh_ext`.
//!   * `standalone-dns-proxy/main.go` — standalone L7 DNS proxy
//!     entrypoint. Agent-side intercept + cache + policy enforcement
//!     live in `cilium::dns_proxy`.

use crate::cilium::types::Cite;

/// One cross-binary cite row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryCite {
    pub upstream_dir: &'static str,
    pub upstream_role: &'static str,
    pub agent_side_modules: &'static [&'static str],
}

pub fn binaries() -> &'static [BinaryCite] {
    &[
        BinaryCite {
            upstream_dir: "bpf/",
            upstream_role: "kernel-side eBPF C source (bpf_lxc.c, bpf_host.c, lib/{conntrack,lb,nat,srv6,ipv6}.h)",
            agent_side_modules: &[
                "src/cilium/conntrack.rs",
                "src/cilium/nat.rs",
                "src/cilium/lb.rs",
                "src/cilium/srv6.rs",
                "src/cilium/ipv6.rs",
                "src/cilium/bpf_loader.rs",
                "src/cilium/bpfmaps.rs",
                "src/cilium/bpf_dump.rs",
                "src/cilium/maps_gc.rs",
            ],
        },
        BinaryCite {
            upstream_dir: "hubble-relay/",
            upstream_role: "hubble-relay binary entrypoint",
            agent_side_modules: &[
                "src/cilium/hubble.rs",
                "src/cilium/hubble_ext.rs",
                "src/cilium/hubble_metrics.rs",
            ],
        },
        BinaryCite {
            upstream_dir: "clustermesh-apiserver/",
            upstream_role: "clustermesh-apiserver binary entrypoint",
            agent_side_modules: &[
                "src/cilium/clustermesh.rs",
                "src/cilium/clustermesh_ext.rs",
            ],
        },
        BinaryCite {
            upstream_dir: "standalone-dns-proxy/",
            upstream_role: "standalone L7 DNS proxy entrypoint",
            agent_side_modules: &[
                "src/cilium/dns_proxy.rs",
                "src/cilium/fqdn.rs",
            ],
        },
    ]
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("bpf/", "BinaryCites");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn binaries_table_lists_all_four_known_binaries() {
        let (_c, _t) = cilium_test_ctx!("bpf/", "Binaries.Count", "tenant-bc-cnt");
        let dirs: Vec<&str> = binaries().iter().map(|b| b.upstream_dir).collect();
        for expected in ["bpf/", "hubble-relay/", "clustermesh-apiserver/", "standalone-dns-proxy/"] {
            assert!(dirs.contains(&expected), "missing {}", expected);
        }
    }

    #[test]
    fn every_binary_lists_at_least_one_agent_side_module() {
        let (_c, _t) = cilium_test_ctx!("bpf/", "Binaries.HaveModules", "tenant-bc-mods");
        for b in binaries() {
            assert!(!b.agent_side_modules.is_empty(), "{} has no agent-side modules", b.upstream_dir);
        }
    }

    #[test]
    fn bpf_dir_cites_all_kernel_state_machines() {
        let (_c, _t) = cilium_test_ctx!("bpf/", "BPF.Modules", "tenant-bc-bpf");
        let bpf = binaries().iter().find(|b| b.upstream_dir == "bpf/").unwrap();
        for expected in ["src/cilium/conntrack.rs", "src/cilium/nat.rs", "src/cilium/lb.rs", "src/cilium/srv6.rs", "src/cilium/ipv6.rs"] {
            assert!(bpf.agent_side_modules.contains(&expected), "bpf/ missing {}", expected);
        }
    }

    #[test]
    fn hubble_relay_cites_observer_and_metrics() {
        let (_c, _t) = cilium_test_ctx!("hubble-relay/", "Modules", "tenant-bc-hr");
        let hr = binaries().iter().find(|b| b.upstream_dir == "hubble-relay/").unwrap();
        assert!(hr.agent_side_modules.iter().any(|m| m.contains("hubble")));
        assert!(hr.agent_side_modules.iter().any(|m| m.contains("hubble_ext")));
    }

    #[test]
    fn clustermesh_apiserver_cites_clustermesh_modules() {
        let (_c, _t) = cilium_test_ctx!("clustermesh-apiserver/", "Modules", "tenant-bc-cm");
        let cm = binaries().iter().find(|b| b.upstream_dir == "clustermesh-apiserver/").unwrap();
        assert!(cm.agent_side_modules.iter().any(|m| m.contains("clustermesh")));
    }

    #[test]
    fn standalone_dns_proxy_cites_dns_modules() {
        let (_c, _t) = cilium_test_ctx!("standalone-dns-proxy/", "Modules", "tenant-bc-dns");
        let dp = binaries().iter().find(|b| b.upstream_dir == "standalone-dns-proxy/").unwrap();
        assert!(dp.agent_side_modules.iter().any(|m| m.contains("dns_proxy") || m.contains("fqdn")));
    }
}
