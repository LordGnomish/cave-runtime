// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/hostportusage.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). Apache-2.0 upstream; see NOTICE.
//
//! Tracks HostPort usage within a node. On a node each `<hostIP, hostPort,
//! protocol>` used by bound pods must be unique; we track this to know which
//! pods can potentially schedule together.

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;

/// Pod port protocol. Defaults to TCP per the Kubernetes container-port docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
    Sctp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Protocol::Tcp => "TCP",
            Protocol::Udp => "UDP",
            Protocol::Sctp => "SCTP",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPort {
    pub ip: IpAddr,
    pub port: i32,
    pub protocol: Protocol,
}

impl HostPort {
    pub fn new(ip: IpAddr, port: i32, protocol: Protocol) -> HostPort {
        HostPort { ip, port, protocol }
    }

    /// Two host ports match (collide) when protocol and port are equal and the
    /// IPs are equal — or either side is the unspecified address (`0.0.0.0` /
    /// `::`), which binds all interfaces.
    pub fn matches(&self, rhs: &HostPort) -> bool {
        if self.protocol != rhs.protocol {
            return false;
        }
        if self.port != rhs.port {
            return false;
        }
        if self.ip != rhs.ip && !self.ip.is_unspecified() && !rhs.ip.is_unspecified() {
            return false;
        }
        true
    }
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IP={} Port={} Proto={}", self.ip, self.port, self.protocol)
    }
}

/// A container port declaration, as found on a pod spec. Minimal local shape
/// for [`host_ports`] (`GetHostPorts`).
#[derive(Debug, Clone)]
pub struct ContainerPort {
    pub host_ip: String,
    pub host_port: i32,
    pub protocol: Protocol,
}

/// `GetHostPorts` — extract the host ports a pod reserves. Ports with
/// `host_port == 0` are skipped; an empty `host_ip` defaults to `0.0.0.0`.
pub fn host_ports(ports: &[ContainerPort]) -> Vec<HostPort> {
    let mut usage = vec![];
    for p in ports {
        if p.host_port == 0 {
            continue;
        }
        let host_ip = if p.host_ip.is_empty() {
            "0.0.0.0"
        } else {
            &p.host_ip
        };
        usage.push(HostPort {
            ip: host_ip.parse().unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
            port: p.host_port,
            protocol: p.protocol,
        });
    }
    usage
}

/// Per-node reservation map keyed by the owning pod's `namespace/name`.
#[derive(Debug, Clone, Default)]
pub struct HostPortUsage {
    reserved: BTreeMap<String, Vec<HostPort>>,
}

impl HostPortUsage {
    pub fn new() -> HostPortUsage {
        HostPortUsage {
            reserved: BTreeMap::new(),
        }
    }

    /// `Add` — record (replacing) the ports reserved by `used_by`.
    pub fn add(&mut self, used_by: &str, ports: Vec<HostPort>) {
        self.reserved.insert(used_by.to_string(), ports);
    }

    /// `Conflicts` — error if any of `ports` collides with a port reserved by
    /// a *different* pod.
    pub fn conflicts(&self, used_by: &str, ports: &[HostPort]) -> Result<(), String> {
        for new_entry in ports {
            for (pod_key, entries) in &self.reserved {
                for existing in entries {
                    if new_entry.matches(existing) && pod_key != used_by {
                        return Err(format!(
                            "pod hostport conflicts with existing hostport configuration: \
                             pod-hostport={new_entry} existing-hostport={existing}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `DeletePod` — release all ports reserved by the given pod key.
    pub fn delete_pod(&mut self, key: &str) {
        self.reserved.remove(key);
    }
}
