// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Network kernel events — connect/accept/sendmsg/recvmsg.
//!
//! Upstream: `pkg/grpc/tracing/kprobe.go` network hooks + the
//! `tcp_connect`/`inet_csk_accept` standard library probes.

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkOp {
    Connect,
    Accept,
    Sendmsg,
    Recvmsg,
    Listen,
    Bind,
    Close,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum L4Proto {
    Tcp,
    Udp,
    Sctp,
    Icmp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkEvent {
    pub op: NetworkOp,
    pub proto: L4Proto,
    pub src_ip: IpAddr,
    pub src_port: u16,
    pub dst_ip: IpAddr,
    pub dst_port: u16,
    pub bytes: u64,
    pub process: Process,
    pub observed_at: DateTime<Utc>,
}

impl NetworkEvent {
    pub fn is_loopback(&self) -> bool {
        ip_is_loopback(&self.src_ip) || ip_is_loopback(&self.dst_ip)
    }

    /// True if the destination is within a documented private CIDR.
    pub fn is_internal(&self) -> bool {
        ip_is_private(&self.dst_ip)
    }

    /// True if the destination is a typical egress port used by C2
    /// frameworks (4444, 1337, 31337).
    pub fn is_suspicious_dst_port(&self) -> bool {
        matches!(self.dst_port, 4444 | 1337 | 31337 | 8443)
    }
}

pub fn ip_is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback(),
    }
}

pub fn ip_is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 169 && o[1] == 254)
                || v.is_loopback()
        }
        IpAddr::V6(v) => v.is_loopback() || v.segments()[0] & 0xfe00 == 0xfc00,
    }
}

pub fn ipv4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Credentials, Namespaces};
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }
    fn proc() -> Process {
        Process {
            exec_id: "x".into(),
            pid: 1,
            pid_in_ns: 1,
            binary: "/bin/curl".into(),
            arguments: String::new(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: None,
            container_id: None,
            pod_name: None,
            pod_namespace: None,
            start_time: ts(),
            end_time: None,
        }
    }

    fn ev(op: NetworkOp, dst: IpAddr, dport: u16) -> NetworkEvent {
        NetworkEvent {
            op,
            proto: L4Proto::Tcp,
            src_ip: ipv4(10, 0, 0, 1),
            src_port: 33333,
            dst_ip: dst,
            dst_port: dport,
            bytes: 0,
            process: proc(),
            observed_at: ts(),
        }
    }

    #[test]
    fn test_loopback_detected() {
        let e = ev(NetworkOp::Connect, ipv4(127, 0, 0, 1), 22);
        assert!(e.is_loopback());
    }

    #[test]
    fn test_private_cidrs() {
        assert!(ip_is_private(&ipv4(10, 0, 0, 1)));
        assert!(ip_is_private(&ipv4(172, 17, 0, 1)));
        assert!(ip_is_private(&ipv4(192, 168, 0, 1)));
        assert!(ip_is_private(&ipv4(169, 254, 169, 254)));
        assert!(!ip_is_private(&ipv4(8, 8, 8, 8)));
    }

    #[test]
    fn test_suspicious_port_classifier() {
        for p in [4444u16, 1337, 31337] {
            let e = ev(NetworkOp::Connect, ipv4(1, 2, 3, 4), p);
            assert!(e.is_suspicious_dst_port());
        }
        let safe = ev(NetworkOp::Connect, ipv4(1, 2, 3, 4), 443);
        assert!(!safe.is_suspicious_dst_port());
    }

    #[test]
    fn test_is_internal() {
        assert!(ev(NetworkOp::Connect, ipv4(10, 0, 0, 5), 80).is_internal());
        assert!(!ev(NetworkOp::Connect, ipv4(8, 8, 8, 8), 53).is_internal());
    }

    #[test]
    fn test_network_event_serde_roundtrip() {
        let e = ev(NetworkOp::Sendmsg, ipv4(1, 1, 1, 1), 443);
        let j = serde_json::to_string(&e).unwrap();
        let back: NetworkEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn test_l4_proto_serde() {
        let j = serde_json::to_string(&L4Proto::Sctp).unwrap();
        assert_eq!(j, "\"sctp\"");
    }
}
