// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Conntrack flush + sysctl helpers.
//!
//! Cite: `pkg/proxy/conntrack/conntrack.go:32` (Interface),
//! `:48` (Exec), `:73` (ClearEntries), `:94` (SetSysctl),
//! `pkg/util/conntrack/conntrack.go:42` (ClearEntriesForIP),
//! `:62` (ClearEntriesForPort), `:81` (ClearEntriesForNAT).
//!
//! Real netfilter calls live behind a `ConntrackBackend` trait so tests
//! can capture the requested flush list without touching `conntrack -D`.

use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::Protocol;
use std::collections::BTreeMap;
use std::net::IpAddr;

/// Cite: `pkg/proxy/conntrack/conntrack.go:32` (Interface) — the operations
/// the proxier needs from a conntrack helper.
pub trait ConntrackBackend {
    fn clear_entries_for_ip(&mut self, ip: IpAddr, proto: Protocol) -> KubeProxyResult<()>;
    fn clear_entries_for_port(&mut self, port: u16, proto: Protocol) -> KubeProxyResult<()>;
    fn clear_entries_for_nat(
        &mut self,
        origin_ip: IpAddr,
        dest_ip: IpAddr,
        proto: Protocol,
    ) -> KubeProxyResult<()>;
    fn set_sysctl(&mut self, key: &str, value: u32) -> KubeProxyResult<()>;
}

/// In-memory capture backend — tests assert against the recorded
/// (kind, args) tuples instead of running real conntrack syscalls.
#[derive(Debug, Default, Clone)]
pub struct CapturedConntrack {
    pub cleared_ips: Vec<(IpAddr, Protocol)>,
    pub cleared_ports: Vec<(u16, Protocol)>,
    pub cleared_nat: Vec<(IpAddr, IpAddr, Protocol)>,
    pub sysctls: BTreeMap<String, u32>,
}

impl ConntrackBackend for CapturedConntrack {
    fn clear_entries_for_ip(&mut self, ip: IpAddr, proto: Protocol) -> KubeProxyResult<()> {
        self.cleared_ips.push((ip, proto));
        Ok(())
    }

    fn clear_entries_for_port(&mut self, port: u16, proto: Protocol) -> KubeProxyResult<()> {
        self.cleared_ports.push((port, proto));
        Ok(())
    }

    fn clear_entries_for_nat(
        &mut self,
        origin_ip: IpAddr,
        dest_ip: IpAddr,
        proto: Protocol,
    ) -> KubeProxyResult<()> {
        self.cleared_nat.push((origin_ip, dest_ip, proto));
        Ok(())
    }

    fn set_sysctl(&mut self, key: &str, value: u32) -> KubeProxyResult<()> {
        self.sysctls.insert(key.to_string(), value);
        Ok(())
    }
}

/// Cite: `pkg/proxy/conntrack/conntrack.go:94` (SetSysctl) — the four
/// conntrack sysctls the proxier owns at startup.
///
/// `nf_conntrack_max`            : `Conntrack.MaxPerCore * NumCPU` (clamped)
/// `nf_conntrack_tcp_be_liberal` : 1 if `LiberalTCP` is enabled
/// `nf_conntrack_tcp_timeout_established` : `Conntrack.TCPEstablishedTimeout`
/// `nf_conntrack_tcp_timeout_close_wait`  : `Conntrack.TCPCloseWaitTimeout`
pub fn apply_conntrack_sysctls<B: ConntrackBackend>(
    backend: &mut B,
    max_per_core: u32,
    num_cpu: u32,
    tcp_established_secs: u32,
    tcp_close_wait_secs: u32,
    liberal_tcp: bool,
) -> KubeProxyResult<()> {
    let total = (max_per_core as u64)
        .checked_mul(num_cpu as u64)
        .ok_or_else(|| KubeProxyError::ConntrackApply {
            key: "nf_conntrack_max".to_string(),
            reason: "overflow computing max_per_core * num_cpu".to_string(),
        })?;
    let clamped = total.min(u32::MAX as u64) as u32;
    backend.set_sysctl("nf_conntrack_max", clamped)?;
    backend.set_sysctl("nf_conntrack_tcp_be_liberal", liberal_tcp as u32)?;
    backend.set_sysctl(
        "nf_conntrack_tcp_timeout_established",
        tcp_established_secs,
    )?;
    backend.set_sysctl("nf_conntrack_tcp_timeout_close_wait", tcp_close_wait_secs)?;
    Ok(())
}

/// Cite: `pkg/util/conntrack/conntrack.go:62` (ClearEntriesForPort) — when
/// a NodePort is reclaimed the proxier must flush stale conntrack entries
/// to prevent leaked flows.
pub fn flush_stale_node_ports<B: ConntrackBackend>(
    backend: &mut B,
    released_ports: &[(u16, Protocol)],
) -> KubeProxyResult<()> {
    for (port, proto) in released_ports {
        backend.clear_entries_for_port(*port, *proto)?;
    }
    Ok(())
}

/// Cite: `pkg/util/conntrack/conntrack.go:42` (ClearEntriesForIP) — when
/// a Service ClusterIP is reassigned, conntrack entries pinned to the
/// old IP must be flushed.
pub fn flush_stale_cluster_ips<B: ConntrackBackend>(
    backend: &mut B,
    released_ips: &[(IpAddr, Protocol)],
) -> KubeProxyResult<()> {
    for (ip, proto) in released_ips {
        backend.clear_entries_for_ip(*ip, *proto)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn captured_backend_records_ip_clears() {
        let mut b = CapturedConntrack::default();
        b.clear_entries_for_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), Protocol::Tcp)
            .unwrap();
        assert_eq!(b.cleared_ips.len(), 1);
    }

    #[test]
    fn apply_sysctls_sets_four_keys() {
        let mut b = CapturedConntrack::default();
        apply_conntrack_sysctls(&mut b, 32_768, 8, 86_400, 3_600, true).unwrap();
        assert_eq!(b.sysctls.get("nf_conntrack_max").copied(), Some(262_144));
        assert_eq!(b.sysctls.get("nf_conntrack_tcp_be_liberal").copied(), Some(1));
        assert_eq!(
            b.sysctls
                .get("nf_conntrack_tcp_timeout_established")
                .copied(),
            Some(86_400)
        );
        assert_eq!(
            b.sysctls
                .get("nf_conntrack_tcp_timeout_close_wait")
                .copied(),
            Some(3_600)
        );
    }

    #[test]
    fn apply_sysctls_clamps_overflow() {
        let mut b = CapturedConntrack::default();
        apply_conntrack_sysctls(&mut b, u32::MAX, 8, 86_400, 3_600, false).unwrap();
        assert_eq!(b.sysctls.get("nf_conntrack_max").copied(), Some(u32::MAX));
    }

    #[test]
    fn flush_stale_node_ports_pushes_each() {
        let mut b = CapturedConntrack::default();
        flush_stale_node_ports(
            &mut b,
            &[(30_001, Protocol::Tcp), (30_002, Protocol::Udp)],
        )
        .unwrap();
        assert_eq!(b.cleared_ports.len(), 2);
    }

    #[test]
    fn flush_stale_cluster_ips_pushes_each() {
        let mut b = CapturedConntrack::default();
        flush_stale_cluster_ips(
            &mut b,
            &[
                (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), Protocol::Tcp),
                (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), Protocol::Tcp),
            ],
        )
        .unwrap();
        assert_eq!(b.cleared_ips.len(), 2);
    }

    #[test]
    fn captured_backend_records_nat_triple() {
        let mut b = CapturedConntrack::default();
        b.clear_entries_for_nat(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            Protocol::Tcp,
        )
        .unwrap();
        assert_eq!(b.cleared_nat.len(), 1);
    }
}
