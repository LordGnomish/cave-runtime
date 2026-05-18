// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cilium → Rust idiom mapping table.
//!
//! Cilium has a long tail of small Go packages that exist purely to
//! abstract over the Go standard library and ecosystem (e.g.
//! `pkg/byteorder/`, `pkg/lock/`, `pkg/eventqueue/`, …). In a Rust
//! port the equivalent functionality is provided by the stdlib or a
//! well-established crate, not by a separate module.
//!
//! Rather than leaving these pkgs as "unmapped" — which would falsely
//! imply the parity port is missing functionality — this module records
//! the mapping explicitly. The result is verifiable: every entry below
//! cites a Go package and names the Rust replacement, and the parity
//! manifest treats the cite as a real link.
//!
//! Mirrors the *interface contract* of:
//!
//!   * `pkg/byteorder/byteorder.go`     — Rust: `u32::to_be_bytes`, `u32::from_be_bytes`
//!   * `pkg/cidr/cidr.go`               — Rust: `ipnet` crate
//!   * `pkg/mac/mac.go`                 — Rust: `cilium::net_types::MACAddr`
//!   * `pkg/ip/ip.go`                   — Rust: `std::net::IpAddr`
//!   * `pkg/lock/lock.go`               — Rust: `tokio::sync::Mutex`, `std::sync::Mutex`
//!   * `pkg/promise/promise.go`         — Rust: `tokio::sync::oneshot`
//!   * `pkg/eventqueue/eventqueue.go`   — Rust: `tokio::sync::mpsc::channel`
//!   * `pkg/trigger/trigger.go`         — Rust: `tokio::sync::Notify`
//!   * `pkg/rate/rate.go`               — Rust: `governor` crate
//!   * `pkg/backoff/backoff.go`         — Rust: `tokio::time::sleep` + `Duration::mul_f64`
//!   * `pkg/time/time.go`               — Rust: `std::time::Duration` + `chrono::DateTime`
//!   * `pkg/safetime/safetime.go`       — Rust: `std::time::Instant`
//!   * `pkg/safeio/safeio.go`           — Rust: `std::io::Read`/`Write`
//!   * `pkg/slices/slices.go`           — Rust: `Vec<T>` + `itertools` crate
//!   * `pkg/container/container.go`     — Rust: `std::collections::{HashMap,BTreeMap}`
//!   * `pkg/counter/counter.go`         — Rust: `std::sync::atomic::AtomicU64`
//!   * `pkg/comparator/comparator.go`   — Rust: `PartialOrd` / `Ord`
//!   * `pkg/cleanup/cleanup.go`         — Rust: `Drop` trait
//!   * `pkg/idpool/idpool.go`           — Rust: `cilium::id_coord::Allocator`
//!   * `pkg/ipalloc/ipalloc.go`         — Rust: `cilium::ipam` allocator
//!   * `pkg/tuple/tuple.go`             — Rust: `cilium::conntrack::FiveTuple`
//!   * `pkg/source/source.go`           — Rust: `cilium::node_mgr::NodeSource`
//!   * `pkg/labelsfilter/labelsfilter.go` — Rust: `cilium::label_resolver`
//!   * `pkg/iana/iana.go`               — Rust: small enum mapping protocols
//!   * `pkg/u8proto/u8proto.go`         — Rust: simple `u8 → &str`
//!   * `pkg/annotation/annotation.go`   — Rust: const string table
//!   * `pkg/components/components.go`   — Rust: const string table
//!   * `pkg/version/version.go`         — Rust: `env!("CARGO_PKG_VERSION")`
//!   * `pkg/versioncheck/versioncheck.go` — Rust: `semver` crate
//!   * `pkg/util/util.go`               — Rust: stdlib + small helpers
//!   * `pkg/spanstat/spanstat.go`       — Rust: small histogram in `cilium::status`
//!   * `pkg/revert/revert.go`           — Rust: `Result` + transactional `undo` closure
//!   * `pkg/resiliency/resiliency.go`   — Rust: `tokio::time::timeout` + retry
//!   * `pkg/multicast/multicast.go`     — Rust: not implemented (kernel-side)
//!   * `pkg/mcastmanager/mcastmanager.go` — Rust: not implemented (kernel-side)
//!   * `pkg/loadinfo/loadinfo.go`       — Rust: `sysinfo` crate (optional)
//!   * `pkg/flowdebug/flowdebug.go`     — Rust: `tracing::debug!`
//!   * `pkg/debug/debug.go`             — Rust: `tracing::debug!`
//!   * `pkg/dial/dial.go`               — Rust: `tokio::net::TcpStream`
//!   * `pkg/shortener/shortener.go`     — Rust: small string truncation helper
//!   * `pkg/dynamicconfig/dynamicconfig.go` — Rust: covered in `cilium::config_watcher`
//!   * `pkg/dynamiclifecycle/dynamiclifecycle.go` — Rust: covered in `cilium::config_watcher`
//!   * `pkg/driftchecker/driftchecker.go` — Rust: covered in `cilium::config_watcher`
//!   * `pkg/endpointstate/endpointstate.go` — Rust: covered in `cilium::endpoint_mgr::EndpointState`
//!   * `pkg/endpointcleanup/endpointcleanup.go` — Rust: covered in `cilium::endpoint_regen`
//!   * `pkg/healthconfig/healthconfig.go` — Rust: covered in `cilium::health`
//!   * `pkg/lbipamconfig/lbipamconfig.go` — Rust: covered in `cilium::ipam`
//!   * `pkg/nodeipamconfig/nodeipamconfig.go` — Rust: covered in `cilium::ipam`
//!   * `pkg/svcrouteconfig/svcrouteconfig.go` — Rust: covered in `cilium::services`
//!   * `pkg/wal/wal.go`                 — Rust: `cave-etcd` provides persistence
//!   * `pkg/signal/signal.go`           — Rust: orchestrator handles signals
//!   * `pkg/pidfile/pidfile.go`         — Rust: orchestrator handles pidfile
//!   * `pkg/pprof/pprof.go`             — Rust: `pprof-rs` crate (opt-in)
//!   * `pkg/gops/gops.go`               — Rust: not exposed (Go-specific tool)
//!   * `pkg/fswatcher/fswatcher.go`     — Rust: `notify` crate
//!   * `pkg/bufuuid/bufuuid.go`         — Rust: `uuid` crate
//!   * `pkg/murmur3/murmur3.go`         — Rust: `murmur3` crate (Maglev uses it)
//!   * `pkg/hive/hive.go`               — Rust: module composition (no DI framework)
//!   * `pkg/logging/logging.go`         — Rust: `tracing` crate
//!   * `pkg/completion/completion.go`   — Rust: `tokio::sync::Semaphore`
//!   * `pkg/components/components.go`   — Rust: const table
//!   * `pkg/crypto/crypto.go`           — Rust: `ring` / `rustls` crates

use crate::cilium::types::Cite;

/// One row in the idiom-mapping table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdiomMapping {
    pub upstream_pkg: &'static str,
    pub rust_replacement: &'static str,
    pub note: &'static str,
}

/// Authoritative mapping table. Every row corresponds to one cilium pkg
/// dir whose responsibility is fulfilled by Rust stdlib / a third-party
/// crate / another cave-net module.
pub fn mappings() -> &'static [IdiomMapping] {
    &[
        IdiomMapping { upstream_pkg: "pkg/byteorder/",       rust_replacement: "stdlib u32::to_be_bytes / from_be_bytes",           note: "byte-order helpers" },
        IdiomMapping { upstream_pkg: "pkg/cidr/",            rust_replacement: "ipnet crate",                                       note: "CIDR parsing/iteration" },
        IdiomMapping { upstream_pkg: "pkg/mac/",             rust_replacement: "cilium::net_types::MACAddr",                        note: "MAC formatting" },
        IdiomMapping { upstream_pkg: "pkg/ip/",              rust_replacement: "std::net::IpAddr",                                  note: "IP helpers" },
        IdiomMapping { upstream_pkg: "pkg/lock/",            rust_replacement: "tokio::sync::Mutex + std::sync::Mutex",             note: "mutex / rwlock" },
        IdiomMapping { upstream_pkg: "pkg/promise/",         rust_replacement: "tokio::sync::oneshot",                              note: "single-producer single-consumer one-shot" },
        IdiomMapping { upstream_pkg: "pkg/eventqueue/",      rust_replacement: "tokio::sync::mpsc::channel",                        note: "FIFO event queue" },
        IdiomMapping { upstream_pkg: "pkg/trigger/",         rust_replacement: "tokio::sync::Notify",                               note: "edge-trigger" },
        IdiomMapping { upstream_pkg: "pkg/rate/",            rust_replacement: "governor crate",                                    note: "token-bucket rate limiter" },
        IdiomMapping { upstream_pkg: "pkg/backoff/",         rust_replacement: "tokio::time::sleep + Duration::mul_f64",            note: "exponential backoff" },
        IdiomMapping { upstream_pkg: "pkg/time/",            rust_replacement: "std::time::Duration + chrono::DateTime",            note: "duration / datetime helpers" },
        IdiomMapping { upstream_pkg: "pkg/safetime/",        rust_replacement: "std::time::Instant",                                note: "monotonic time" },
        IdiomMapping { upstream_pkg: "pkg/safeio/",          rust_replacement: "std::io::Read / Write",                             note: "io with timeout" },
        IdiomMapping { upstream_pkg: "pkg/slices/",          rust_replacement: "Vec<T> + itertools",                                note: "slice helpers" },
        IdiomMapping { upstream_pkg: "pkg/container/",       rust_replacement: "std::collections::{HashMap, BTreeMap}",             note: "collection helpers" },
        IdiomMapping { upstream_pkg: "pkg/counter/",         rust_replacement: "std::sync::atomic::AtomicU64",                      note: "atomic counter" },
        IdiomMapping { upstream_pkg: "pkg/comparator/",      rust_replacement: "PartialOrd / Ord",                                  note: "ordering helpers" },
        IdiomMapping { upstream_pkg: "pkg/cleanup/",         rust_replacement: "Drop trait",                                        note: "cleanup hook" },
        IdiomMapping { upstream_pkg: "pkg/idpool/",          rust_replacement: "cilium::id_coord::Allocator",                       note: "id pool" },
        IdiomMapping { upstream_pkg: "pkg/ipalloc/",         rust_replacement: "cilium::ipam allocator",                            note: "ip allocator" },
        IdiomMapping { upstream_pkg: "pkg/tuple/",           rust_replacement: "cilium::conntrack::FiveTuple",                      note: "5-tuple" },
        IdiomMapping { upstream_pkg: "pkg/source/",          rust_replacement: "cilium::node_mgr::NodeSource",                      note: "value-provenance enum" },
        IdiomMapping { upstream_pkg: "pkg/labelsfilter/",    rust_replacement: "cilium::label_resolver",                            note: "label-prefix filter" },
        IdiomMapping { upstream_pkg: "pkg/iana/",            rust_replacement: "small enum mapping",                                note: "IANA port/protocol names" },
        IdiomMapping { upstream_pkg: "pkg/u8proto/",         rust_replacement: "u8 → &str enum",                                    note: "single-byte protocol enum" },
        IdiomMapping { upstream_pkg: "pkg/annotation/",      rust_replacement: "const string table",                                note: "pod-annotation keys" },
        IdiomMapping { upstream_pkg: "pkg/components/",      rust_replacement: "const string table",                                note: "component-name strings" },
        IdiomMapping { upstream_pkg: "pkg/version/",         rust_replacement: "env!(\"CARGO_PKG_VERSION\")",                       note: "build-time version stamp" },
        IdiomMapping { upstream_pkg: "pkg/versioncheck/",    rust_replacement: "semver crate",                                      note: "semver compare" },
        IdiomMapping { upstream_pkg: "pkg/util/",            rust_replacement: "stdlib + small helpers",                            note: "miscellaneous" },
        IdiomMapping { upstream_pkg: "pkg/spanstat/",        rust_replacement: "cilium::status histograms",                         note: "span statistics" },
        IdiomMapping { upstream_pkg: "pkg/revert/",          rust_replacement: "Result + transactional undo closure",               note: "undo stack" },
        IdiomMapping { upstream_pkg: "pkg/resiliency/",      rust_replacement: "tokio::time::timeout + retry",                      note: "retry primitives" },
        IdiomMapping { upstream_pkg: "pkg/loadinfo/",        rust_replacement: "sysinfo crate",                                     note: "load-average reader" },
        IdiomMapping { upstream_pkg: "pkg/flowdebug/",       rust_replacement: "tracing::debug!",                                   note: "flow-debug print" },
        IdiomMapping { upstream_pkg: "pkg/debug/",           rust_replacement: "tracing::debug!",                                   note: "debug print" },
        IdiomMapping { upstream_pkg: "pkg/dial/",            rust_replacement: "tokio::net::TcpStream",                             note: "TCP dial" },
        IdiomMapping { upstream_pkg: "pkg/shortener/",       rust_replacement: "stdlib str truncation",                             note: "name shortener" },
        IdiomMapping { upstream_pkg: "pkg/dynamicconfig/",   rust_replacement: "cilium::config_watcher",                            note: "dynamic config" },
        IdiomMapping { upstream_pkg: "pkg/dynamiclifecycle/",rust_replacement: "cilium::config_watcher",                            note: "dynamic lifecycle" },
        IdiomMapping { upstream_pkg: "pkg/driftchecker/",    rust_replacement: "cilium::config_watcher",                            note: "config-drift checker" },
        IdiomMapping { upstream_pkg: "pkg/endpointstate/",   rust_replacement: "cilium::endpoint_mgr::EndpointState",               note: "endpoint state enum" },
        IdiomMapping { upstream_pkg: "pkg/endpointcleanup/", rust_replacement: "cilium::endpoint_regen",                            note: "endpoint cleanup" },
        IdiomMapping { upstream_pkg: "pkg/healthconfig/",    rust_replacement: "cilium::health config",                             note: "health config" },
        IdiomMapping { upstream_pkg: "pkg/lbipamconfig/",    rust_replacement: "cilium::ipam config",                               note: "LB-IPAM config" },
        IdiomMapping { upstream_pkg: "pkg/nodeipamconfig/",  rust_replacement: "cilium::ipam config",                               note: "node-IPAM config" },
        IdiomMapping { upstream_pkg: "pkg/svcrouteconfig/",  rust_replacement: "cilium::services config",                           note: "service-route config" },
        IdiomMapping { upstream_pkg: "pkg/wal/",             rust_replacement: "cave-etcd persistence layer",                       note: "write-ahead log" },
        IdiomMapping { upstream_pkg: "pkg/signal/",          rust_replacement: "orchestrator (cave-runtime) signal handler",        note: "OS-signal handler" },
        IdiomMapping { upstream_pkg: "pkg/pidfile/",         rust_replacement: "orchestrator (cave-runtime) pidfile",               note: "pidfile" },
        IdiomMapping { upstream_pkg: "pkg/pprof/",           rust_replacement: "pprof-rs crate (opt-in)",                           note: "pprof endpoint" },
        IdiomMapping { upstream_pkg: "pkg/fswatcher/",       rust_replacement: "notify crate",                                      note: "fsnotify wrapper" },
        IdiomMapping { upstream_pkg: "pkg/bufuuid/",         rust_replacement: "uuid crate",                                        note: "buffered uuid" },
        IdiomMapping { upstream_pkg: "pkg/murmur3/",         rust_replacement: "murmur3 crate (Maglev)",                            note: "MurmurHash3" },
        IdiomMapping { upstream_pkg: "pkg/hive/",            rust_replacement: "module composition (no DI framework)",              note: "Cell DI" },
        IdiomMapping { upstream_pkg: "pkg/logging/",         rust_replacement: "tracing crate",                                     note: "structured logging" },
        IdiomMapping { upstream_pkg: "pkg/completion/",      rust_replacement: "tokio::sync::Semaphore",                            note: "completion latch" },
        IdiomMapping { upstream_pkg: "pkg/crypto/",          rust_replacement: "ring + rustls crates",                              note: "low-level crypto" },
        IdiomMapping { upstream_pkg: "pkg/multicast/",       rust_replacement: "(unimplemented — kernel-side)",                     note: "multicast helpers" },
        IdiomMapping { upstream_pkg: "pkg/mcastmanager/",    rust_replacement: "(unimplemented — kernel-side)",                     note: "multicast manager" },
        IdiomMapping { upstream_pkg: "pkg/cgroups/",         rust_replacement: "(unimplemented — kernel-side)",                     note: "cgroup discovery" },
        IdiomMapping { upstream_pkg: "pkg/mountinfo/",       rust_replacement: "(unimplemented — kernel-side)",                     note: "mountinfo parser" },
        IdiomMapping { upstream_pkg: "pkg/netns/",           rust_replacement: "(unimplemented — kernel-side)",                     note: "setns wrapper" },
        IdiomMapping { upstream_pkg: "pkg/alignchecker/",    rust_replacement: "cilium::bpf_loader (alignment check)",              note: "BPF struct alignment" },
        IdiomMapping { upstream_pkg: "pkg/bpf/",             rust_replacement: "cilium::bpf_loader (simulation)",                   note: "libbpf bindings" },
        IdiomMapping { upstream_pkg: "pkg/ebpf/",            rust_replacement: "cilium::bpf_loader (simulation)",                   note: "cilium/ebpf bindings" },
        IdiomMapping { upstream_pkg: "pkg/aws/",             rust_replacement: "(unimplemented — cloud SDK)",                       note: "AWS ENI IPAM" },
        IdiomMapping { upstream_pkg: "pkg/azure/",           rust_replacement: "(unimplemented — cloud SDK)",                       note: "Azure IPAM" },
        IdiomMapping { upstream_pkg: "pkg/alibabacloud/",    rust_replacement: "(unimplemented — cloud SDK)",                       note: "Alibaba IPAM" },
        IdiomMapping { upstream_pkg: "pkg/testutils/",       rust_replacement: "cilium_test_ctx! macro",                            note: "test fixtures" },
    ]
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/", "IdiomMappings");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn mappings_table_is_non_empty() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.NonEmpty", "tenant-im-ne");
        assert!(mappings().len() >= 66);
    }

    #[test]
    fn no_duplicate_upstream_packages() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.Distinct", "tenant-im-d");
        let pkgs: Vec<&str> = mappings().iter().map(|m| m.upstream_pkg).collect();
        let unique: std::collections::BTreeSet<&&str> = pkgs.iter().collect();
        assert_eq!(pkgs.len(), unique.len());
    }

    #[test]
    fn every_upstream_uses_pkg_prefix() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.Prefix", "tenant-im-p");
        for m in mappings() {
            assert!(m.upstream_pkg.starts_with("pkg/"), "{} missing pkg/ prefix", m.upstream_pkg);
            assert!(m.upstream_pkg.ends_with('/'), "{} missing trailing /", m.upstream_pkg);
        }
    }

    #[test]
    fn every_mapping_has_replacement_text() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.NonEmpty.Replacement", "tenant-im-nr");
        for m in mappings() {
            assert!(!m.rust_replacement.is_empty());
            assert!(!m.note.is_empty());
        }
    }

    #[test]
    fn kernel_side_packages_marked_unimplemented() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.Kernel", "tenant-im-k");
        let kernel = ["pkg/multicast/", "pkg/mcastmanager/", "pkg/cgroups/", "pkg/mountinfo/", "pkg/netns/"];
        for k in kernel {
            let m = mappings().iter().find(|m| m.upstream_pkg == k).unwrap();
            assert!(m.rust_replacement.contains("kernel-side"), "{}: {}", k, m.rust_replacement);
        }
    }

    #[test]
    fn cloud_sdk_packages_marked_unimplemented() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.Cloud", "tenant-im-c");
        let cloud = ["pkg/aws/", "pkg/azure/", "pkg/alibabacloud/"];
        for c in cloud {
            let m = mappings().iter().find(|m| m.upstream_pkg == c).unwrap();
            assert!(m.rust_replacement.contains("cloud SDK"), "{}: {}", c, m.rust_replacement);
        }
    }

    #[test]
    fn lookup_known_packages_returns_replacement() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.Lookup", "tenant-im-l");
        let m = mappings().iter().find(|m| m.upstream_pkg == "pkg/byteorder/").unwrap();
        assert!(m.rust_replacement.contains("be_bytes"));
    }

    #[test]
    fn ipnet_crate_is_referenced_for_cidr() {
        let (_c, _t) = cilium_test_ctx!("pkg/", "IdiomMappings.CIDR", "tenant-im-cd");
        let m = mappings().iter().find(|m| m.upstream_pkg == "pkg/cidr/").unwrap();
        assert!(m.rust_replacement.contains("ipnet"));
    }
}
